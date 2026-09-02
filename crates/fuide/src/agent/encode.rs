//! Screenshot encoding without extra crates: RGB downscale, PNG (via `sips` on macOS, else an
//! uncompressed-deflate PNG written by hand) and base64 for the MCP image payload.

use std::io::Write as _;
use std::path::Path;

/// Standard base64 with padding.
pub fn base64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            T[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            T[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

/// An opaque RGB raster.
pub struct Rgb {
    pub width: usize,
    pub height: usize,
    /// `width * height * 3` bytes, row-major.
    pub data: Vec<u8>,
}

impl Rgb {
    /// Composite `img` on black (the shell is drawn on a transparent window) and scale it by
    /// `scale` (0 < scale <= 1) with box averaging.
    pub fn from_image(img: &egui::ColorImage, scale: f32) -> Self {
        let [sw, sh] = img.size;
        let scale = if scale.is_finite() {
            scale.clamp(0.1, 1.0)
        } else {
            1.0
        };
        let width = ((sw as f32 * scale).round() as usize).max(1);
        let height = ((sh as f32 * scale).round() as usize).max(1);
        let mut data = Vec::with_capacity(width * height * 3);
        for y in 0..height {
            let y0 = y * sh / height;
            let y1 = ((y + 1) * sh / height).max(y0 + 1).min(sh);
            for x in 0..width {
                let x0 = x * sw / width;
                let x1 = ((x + 1) * sw / width).max(x0 + 1).min(sw);
                let (mut r, mut g, mut b, mut n) = (0u32, 0u32, 0u32, 0u32);
                for yy in y0..y1 {
                    for xx in x0..x1 {
                        let px = img.pixels[yy * sw + xx];
                        // premultiplied alpha → already composited on black
                        r += u32::from(px.r());
                        g += u32::from(px.g());
                        b += u32::from(px.b());
                        n += 1;
                    }
                }
                data.extend_from_slice(&[(r / n) as u8, (g / n) as u8, (b / n) as u8]);
            }
        }
        Self {
            width,
            height,
            data,
        }
    }

    /// PNG bytes. macOS: write a TGA and let `sips` compress it (no extra permissions or
    /// crates). Elsewhere, or if `sips` fails: a valid but uncompressed PNG.
    pub fn to_png(&self) -> Vec<u8> {
        self.png_via_sips().unwrap_or_else(|| self.png_stored())
    }

    fn png_via_sips(&self) -> Option<Vec<u8>> {
        if !cfg!(target_os = "macos") {
            return None;
        }
        let dir = std::env::temp_dir();
        let stem = format!(
            "fuide-agent-{}-{}",
            std::process::id(),
            std::thread::current().id().as_u64_or_zero()
        );
        let tga = dir.join(format!("{stem}.tga"));
        let png = dir.join(format!("{stem}.png"));
        let ok = write_tga_rgb(&tga, self).is_ok()
            && std::process::Command::new("sips")
                .args(["-s", "format", "png"])
                .arg(&tga)
                .arg("--out")
                .arg(&png)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .is_ok_and(|s| s.success());
        let bytes = if ok { std::fs::read(&png).ok() } else { None };
        let _ = std::fs::remove_file(&tga);
        let _ = std::fs::remove_file(&png);
        bytes.filter(|b| b.starts_with(b"\x89PNG"))
    }

    /// PNG with zlib "stored" (uncompressed) deflate blocks — big, but needs no compressor.
    pub fn png_stored(&self) -> Vec<u8> {
        // filter byte 0 (None) in front of every row
        let stride = self.width * 3;
        let mut raw = Vec::with_capacity((stride + 1) * self.height);
        for row in self.data.chunks(stride) {
            raw.push(0);
            raw.extend_from_slice(row);
        }
        let mut z = vec![0x78, 0x01]; // zlib header, no compression preset
        let mut blocks = raw.chunks(65535).peekable();
        if raw.is_empty() {
            z.extend_from_slice(&[1, 0, 0, 0xff, 0xff]);
        }
        while let Some(block) = blocks.next() {
            let last = blocks.peek().is_none();
            z.push(u8::from(last));
            let len = block.len() as u16;
            z.extend_from_slice(&len.to_le_bytes());
            z.extend_from_slice(&(!len).to_le_bytes());
            z.extend_from_slice(block);
        }
        z.extend_from_slice(&adler32(&raw).to_be_bytes());

        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        let mut ihdr = Vec::with_capacity(13);
        ihdr.extend_from_slice(&(self.width as u32).to_be_bytes());
        ihdr.extend_from_slice(&(self.height as u32).to_be_bytes());
        ihdr.extend_from_slice(&[8, 2, 0, 0, 0]); // 8-bit, truecolor, deflate, no filter, no interlace
        chunk(&mut png, b"IHDR", &ihdr);
        chunk(&mut png, b"IDAT", &z);
        chunk(&mut png, b"IEND", &[]);
        png
    }
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let start = out.len();
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let crc = crc32(&out[start..]);
    out.extend_from_slice(&crc.to_be_bytes());
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for &b in bytes {
        crc ^= u32::from(b);
        for _ in 0..8 {
            crc = if crc & 1 == 1 {
                (crc >> 1) ^ 0xedb8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

fn adler32(bytes: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &x in bytes {
        a = (a + u32::from(x)) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

/// Uncompressed 24-bit TGA (top-left origin), the input format for `sips`.
fn write_tga_rgb(path: &Path, img: &Rgb) -> std::io::Result<()> {
    let mut f = std::io::BufWriter::new(std::fs::File::create(path)?);
    f.write_all(&[0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0])?;
    f.write_all(&(img.width as u16).to_le_bytes())?;
    f.write_all(&(img.height as u16).to_le_bytes())?;
    f.write_all(&[24, 0x20])?; // bpp, top-left origin
    for px in img.data.chunks(3) {
        f.write_all(&[px[2], px[1], px[0]])?;
    }
    f.flush()
}

trait ThreadIdExt {
    fn as_u64_or_zero(&self) -> u64;
}

impl ThreadIdExt for std::thread::ThreadId {
    /// `ThreadId::as_u64` is unstable; the Debug form (`ThreadId(N)`) is stable enough for a
    /// temp-file name.
    fn as_u64_or_zero(&self) -> u64 {
        let s = format!("{self:?}");
        s.trim_start_matches("ThreadId(")
            .trim_end_matches(')')
            .parse()
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_known_vectors() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn stored_png_has_valid_chunks() {
        let img = egui::ColorImage::filled([4, 2], egui::Color32::from_rgb(10, 20, 30));
        let rgb = Rgb::from_image(&img, 1.0);
        assert_eq!((rgb.width, rgb.height), (4, 2));
        assert_eq!(&rgb.data[..3], &[10, 20, 30]);
        let png = rgb.png_stored();
        assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
        // IHDR length 13, then the type
        assert_eq!(&png[8..16], b"\0\0\0\x0dIHDR");
        // width / height
        assert_eq!(&png[16..24], &[0, 0, 0, 4, 0, 0, 0, 2]);
        // IHDR CRC (known value for 4x2 RGB8)
        assert_eq!(
            crc32(&png[12..29]),
            u32::from_be_bytes(png[29..33].try_into().unwrap())
        );
        assert!(png.ends_with(b"IEND\xaeB`\x82"));
    }

    #[test]
    fn downscale_averages_boxes() {
        let mut img = egui::ColorImage::filled([2, 2], egui::Color32::BLACK);
        img.pixels[0] = egui::Color32::from_rgb(200, 0, 0);
        img.pixels[1] = egui::Color32::from_rgb(0, 200, 0);
        let rgb = Rgb::from_image(&img, 0.5);
        assert_eq!((rgb.width, rgb.height), (1, 1));
        assert_eq!(rgb.data, vec![50, 50, 0]);
    }

    #[test]
    fn to_png_yields_png_on_every_platform() {
        let img = egui::ColorImage::filled([8, 8], egui::Color32::WHITE);
        let png = Rgb::from_image(&img, 1.0).to_png();
        assert!(png.starts_with(b"\x89PNG"));
    }
}
