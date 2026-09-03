//! Typed paths: `~` / relative expansion and Tab completion for path input fields (the file
//! manager's "go to path" dialog, the player's "open" dialog).

use std::path::{Path, PathBuf};

/// Expand `~` and make `input` absolute against `cwd`, then drop `.` / `..` lexically (no
/// symlink resolution — the path is shown as typed).
pub fn expand(input: &str, cwd: &Path) -> PathBuf {
    let home = || std::env::var_os("HOME").map(PathBuf::from);
    let raw = if input == "~" {
        home().unwrap_or_else(|| PathBuf::from("/"))
    } else if let Some(rest) = input.strip_prefix("~/") {
        home().unwrap_or_else(|| PathBuf::from("/")).join(rest)
    } else if input.starts_with('/') {
        PathBuf::from(input)
    } else {
        cwd.join(input)
    };
    let mut out = PathBuf::new();
    for c in raw.components() {
        match c {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    if out.as_os_str().is_empty() {
        out.push("/");
    }
    out
}

/// Completions for a partially typed path: entries of the parent of the last component whose
/// name starts with it (case-insensitive), each returned as the full input to substitute (the
/// typed prefix style — `~/`, relative — is kept). Directories get a trailing `/`; files are
/// included only when `files` is set. Hidden entries only appear once the component starts
/// with `.`. Sorted (directories first when files are included), at most `max`.
pub fn complete(input: &str, cwd: &Path, files: bool, max: usize) -> Vec<String> {
    let (head, part) = match input.rfind('/') {
        Some(i) => (&input[..=i], &input[i + 1..]),
        None => ("", input),
    };
    if head.is_empty() && (part == "~" || part.is_empty()) {
        return Vec::new();
    }
    let dir = if head.is_empty() {
        cwd.to_path_buf()
    } else {
        expand(head, cwd)
    };
    let Ok(rd) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let want = part.to_lowercase();
    let mut names: Vec<(bool, String)> = rd
        .flatten()
        .map(|e| {
            (
                e.path().is_dir(),
                e.file_name().to_string_lossy().into_owned(),
            )
        })
        .filter(|(is_dir, _)| *is_dir || files)
        .filter(|(_, n)| n.to_lowercase().starts_with(&want))
        .filter(|(_, n)| !n.starts_with('.') || part.starts_with('.'))
        .collect();
    names.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then_with(|| a.1.to_lowercase().cmp(&b.1.to_lowercase()))
    });
    names.truncate(max);
    names
        .into_iter()
        .map(|(is_dir, n)| {
            if is_dir {
                format!("{head}{n}/")
            } else {
                format!("{head}{n}")
            }
        })
        .collect()
}

/// Longest common prefix of the candidates (what Tab fills in when several match).
pub fn common_prefix(items: &[String]) -> String {
    let Some(first) = items.first() else {
        return String::new();
    };
    let mut end = first.len();
    for other in &items[1..] {
        end = first
            .char_indices()
            .zip(other.chars())
            .take_while(|((_, a), b)| a == b)
            .last()
            .map(|((i, a), _)| i + a.len_utf8())
            .unwrap_or(0)
            .min(end);
    }
    first[..end].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("fuide-pathinput-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn expand_handles_tilde_relative_and_dotdot() {
        let cwd = PathBuf::from("/a/b");
        let home = PathBuf::from(std::env::var("HOME").unwrap());
        assert_eq!(expand("~", &cwd), home);
        assert_eq!(expand("~/x", &cwd), home.join("x"));
        assert_eq!(expand("c/./d", &cwd), PathBuf::from("/a/b/c/d"));
        assert_eq!(expand("../c", &cwd), PathBuf::from("/a/c"));
        assert_eq!(expand("/..", &cwd), PathBuf::from("/"));
        assert_eq!(expand("/x/", &cwd), PathBuf::from("/x"));
    }

    #[test]
    fn complete_lists_directories_and_optionally_files() {
        let dir = scratch("complete");
        for d in ["Documents", "Downloads", ".config"] {
            std::fs::create_dir(dir.join(d)).unwrap();
        }
        std::fs::write(dir.join("Do.txt"), "").unwrap();
        std::fs::write(dir.join("dance.mp3"), "").unwrap();
        let abs = dir.display().to_string();

        assert_eq!(
            complete("do", &dir, false, 10),
            ["Documents/", "Downloads/"]
        );
        assert_eq!(
            complete("d", &dir, true, 10),
            ["Documents/", "Downloads/", "dance.mp3", "Do.txt"],
            "directories first, then files, case-insensitive"
        );
        assert_eq!(
            complete(&format!("{abs}/.c"), &dir, false, 10),
            [format!("{abs}/.config/")]
        );
        assert_eq!(complete(&format!("{abs}/"), &dir, false, 10).len(), 2);
        assert_eq!(complete("", &dir, true, 10), Vec::<String>::new());
        assert_eq!(complete("x/y", &dir, true, 10), Vec::<String>::new());
        assert_eq!(complete("d", &dir, true, 1).len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn common_prefix_of_candidates() {
        assert_eq!(
            common_prefix(&["Documents/".into(), "Downloads/".into()]),
            "Do"
        );
        assert_eq!(common_prefix(&["a/".into()]), "a/");
        assert_eq!(common_prefix(&[]), "");
    }
}
