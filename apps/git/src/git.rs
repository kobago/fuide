//! Git access through the `git` CLI only (no libgit2 / gitoxide): one code path for reads and
//! writes, and fetch / push get the user's credential helpers for free.
//!
//! Reads (`status --porcelain=v2 -z`, `log`, `for-each-ref`, `diff`, `show`) run on a worker
//! thread and answer as one [`Msg`]. Writes (`add`, `commit`, `switch`, `fetch`, `push` …) all go
//! through the streaming runner ([`Git::run`]) so their output lands in the log line by line and
//! every completed command re-reads the repository. One write at a time.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{channel, Receiver, Sender};

/// One entry of `git status`: a path with its index (`staged`) and worktree (`unstaged`) state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
    pub path: String,
    /// Old path of a rename / copy.
    pub orig_path: Option<String>,
    /// Index status letter (`M`, `A`, `D`, `R`, `C`, `T`), `.` = nothing staged.
    pub staged: char,
    /// Worktree status letter, `.` = nothing unstaged. `?` = untracked, `U` = unmerged.
    pub unstaged: char,
}

impl Change {
    pub fn is_untracked(&self) -> bool {
        self.unstaged == '?'
    }
    pub fn is_unmerged(&self) -> bool {
        self.unstaged == 'U'
    }
    pub fn has_staged(&self) -> bool {
        self.staged != '.' && self.staged != '?' && !self.is_unmerged()
    }
    pub fn has_unstaged(&self) -> bool {
        self.unstaged != '.'
    }
}

/// Human tag for a status letter.
pub fn status_tag(c: char) -> &'static str {
    match c {
        'M' => "MOD",
        'A' => "ADD",
        'D' => "DEL",
        'R' => "REN",
        'C' => "COPY",
        'T' => "TYPE",
        '?' => "NEW",
        'U' => "CONFLICT",
        _ => "--",
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Status {
    pub head: String,
    /// `(detached)` when HEAD is not on a branch.
    pub branch: String,
    pub upstream: String,
    pub ahead: u32,
    pub behind: u32,
    pub changes: Vec<Change>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Commit {
    pub hash: String,
    pub parents: Vec<String>,
    pub author: String,
    /// Unix seconds.
    pub time: i64,
    /// Decorations (`HEAD -> main, origin/main`).
    pub refs: String,
    pub subject: String,
}

impl Commit {
    pub fn short(&self) -> &str {
        &self.hash[..self.hash.len().min(8)]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RefKind {
    Local,
    Remote,
    Tag,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ref {
    pub name: String,
    pub kind: RefKind,
    pub current: bool,
    pub upstream: String,
    pub hash: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineKind {
    Context,
    Add,
    Del,
    /// `@@ -a,b +c,d @@` (start of a hunk).
    Hunk,
    /// File header lines (`diff --git`, `index`, `---`, `+++`, `Binary files …`).
    Meta,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffLine {
    pub kind: LineKind,
    pub old_no: Option<u32>,
    pub new_no: Option<u32>,
    /// Text without the leading `+` / `-` / ` `.
    pub text: String,
    /// Index into [`Diff::hunks`] for hunk header and body lines.
    pub hunk: Option<usize>,
}

/// A unified diff, kept both as lines for display and as raw text per hunk so a single hunk
/// can be handed back to `git apply`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Diff {
    pub lines: Vec<DiffLine>,
    /// File header (everything before the first `@@`), verbatim.
    pub header: String,
    /// Each hunk verbatim (header line included).
    pub hunks: Vec<String>,
    pub adds: usize,
    pub dels: usize,
}

impl Diff {
    /// A patch containing only hunk `i`, suitable for `git apply --cached`.
    pub fn hunk_patch(&self, i: usize) -> String {
        format!("{}{}", self.header, self.hunks[i])
    }
}

/// Parse `git diff` output (one file) into display lines and raw hunks.
pub fn parse_diff(text: &str) -> Diff {
    let mut d = Diff::default();
    let (mut old, mut new) = (0u32, 0u32);
    let mut hunk: Option<usize> = None;
    for raw in text.split_inclusive('\n') {
        let line = raw.strip_suffix('\n').unwrap_or(raw);
        if let Some(rest) = line.strip_prefix("@@") {
            // @@ -old[,n] +new[,n] @@ context
            let mut it = rest.split_whitespace();
            let parse = |s: Option<&str>, sign: char| -> u32 {
                s.and_then(|s| s.strip_prefix(sign))
                    .map(|s| s.split(',').next().unwrap_or("0"))
                    .and_then(|n| n.parse().ok())
                    .unwrap_or(0)
            };
            old = parse(it.next(), '-');
            new = parse(it.next(), '+');
            d.hunks.push(raw.to_string());
            hunk = Some(d.hunks.len() - 1);
            d.lines.push(DiffLine {
                kind: LineKind::Hunk,
                old_no: None,
                new_no: None,
                text: line.to_string(),
                hunk,
            });
            continue;
        }
        let Some(h) = hunk else {
            d.header.push_str(raw);
            d.lines.push(DiffLine {
                kind: LineKind::Meta,
                old_no: None,
                new_no: None,
                text: line.to_string(),
                hunk: None,
            });
            continue;
        };
        d.hunks[h].push_str(raw);
        let (kind, text) = match line.chars().next() {
            Some('+') => (LineKind::Add, &line[1..]),
            Some('-') => (LineKind::Del, &line[1..]),
            Some(' ') => (LineKind::Context, &line[1..]),
            Some('\\') => (LineKind::Meta, line), // "\ No newline at end of file"
            _ => (LineKind::Context, line),
        };
        // `old` / `new` hold the number of the next line on each side
        let (old_no, new_no) = match kind {
            LineKind::Add => {
                new += 1;
                d.adds += 1;
                (None, Some(new - 1))
            }
            LineKind::Del => {
                old += 1;
                d.dels += 1;
                (Some(old - 1), None)
            }
            LineKind::Context => {
                old += 1;
                new += 1;
                (Some(old - 1), Some(new - 1))
            }
            _ => (None, None),
        };
        d.lines.push(DiffLine {
            kind,
            old_no,
            new_no,
            text: text.to_string(),
            hunk: Some(h),
        });
    }
    d
}

/// Which diff is on screen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DiffKey {
    /// Worktree vs index (`git diff`), or the whole file for an untracked one.
    Unstaged(String),
    /// Index vs HEAD (`git diff --cached`).
    Staged(String),
    /// One file of a commit (`git show <hash> -- path`).
    Commit(String, String),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CommitDetail {
    pub hash: String,
    pub parents: Vec<String>,
    pub author: String,
    pub email: String,
    pub time: i64,
    pub refs: String,
    /// Full message (subject + body).
    pub message: String,
    /// (status letter, path)
    pub files: Vec<(char, String)>,
}

pub enum Msg {
    Status(Result<Status, String>),
    Log(Result<Vec<Commit>, String>),
    Refs(Result<Vec<Ref>, String>),
    Diff(DiffKey, Result<Diff, String>),
    Detail(Result<CommitDetail, String>),
    /// One line of output from the streaming runner.
    Line {
        text: String,
        stderr: bool,
    },
    /// A streaming command finished.
    Exit {
        label: String,
        args: Vec<String>,
        ok: bool,
        code: Option<i32>,
        elapsed_ms: f32,
    },
}

/// Locate `git`. `FUIDE_GIT_BIN` overrides it (tests could point it at a script).
pub fn git_executable() -> PathBuf {
    if let Some(p) = std::env::var_os("FUIDE_GIT_BIN") {
        return PathBuf::from(p);
    }
    for p in [
        "/opt/homebrew/bin/git",
        "/usr/local/bin/git",
        "/usr/bin/git",
    ] {
        if Path::new(p).is_file() {
            return PathBuf::from(p);
        }
    }
    PathBuf::from("git")
}

/// `PATH` with the Homebrew bin directories prepended: apps started from Finder get a minimal
/// `PATH`, and credential helpers (`gh`, `git-credential-osxkeychain`) must still resolve.
fn augmented_path() -> String {
    let mut parts: Vec<String> = vec!["/opt/homebrew/bin".into(), "/usr/local/bin".into()];
    parts.extend(
        std::env::var("PATH")
            .unwrap_or_else(|_| "/usr/bin:/bin:/usr/sbin:/sbin".into())
            .split(':')
            .filter(|s| !s.is_empty())
            .map(String::from),
    );
    parts.dedup();
    parts.join(":")
}

fn base_command(repo: &Path) -> Command {
    let mut c = Command::new(git_executable());
    c.current_dir(repo)
        .env("PATH", augmented_path())
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("LC_ALL", "C")
        .args(["-c", "color.ui=never"])
        .stdin(Stdio::null());
    c
}

/// Run a read-only git command to completion; stdout on success, trimmed stderr on failure.
fn query(repo: &Path, args: &[&str]) -> Result<Vec<u8>, String> {
    let o = base_command(repo)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .args(args)
        .output()
        .map_err(|e| format!("cannot start git: {e}"))?;
    if o.status.success() {
        Ok(o.stdout)
    } else {
        Err(String::from_utf8_lossy(&o.stderr).trim().to_string())
    }
}

fn lossy(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

/// Top of the working tree containing `path`, if it is inside a git repository.
pub fn toplevel(path: &Path) -> Result<PathBuf, String> {
    let dir = if path.is_dir() {
        path
    } else {
        path.parent().unwrap_or(path)
    };
    query(dir, &["rev-parse", "--show-toplevel"]).map(|o| PathBuf::from(lossy(&o).trim()))
}

/// Parse `git status --porcelain=v2 -z --branch`.
pub fn parse_status(bytes: &[u8]) -> Status {
    let mut st = Status::default();
    let mut fields = bytes.split(|&b| b == 0).map(lossy).peekable();
    while let Some(f) = fields.next() {
        if f.is_empty() {
            continue;
        }
        if let Some(h) = f.strip_prefix("# ") {
            let (k, v) = h.split_once(' ').unwrap_or((h, ""));
            match k {
                "branch.oid" => st.head = v.to_string(),
                "branch.head" => st.branch = v.to_string(),
                "branch.upstream" => st.upstream = v.to_string(),
                "branch.ab" => {
                    for tok in v.split_whitespace() {
                        if let Some(n) = tok.strip_prefix('+') {
                            st.ahead = n.parse().unwrap_or(0);
                        } else if let Some(n) = tok.strip_prefix('-') {
                            st.behind = n.parse().unwrap_or(0);
                        }
                    }
                }
                _ => {}
            }
            continue;
        }
        let mut it = f.splitn(2, ' ');
        let kind = it.next().unwrap_or("");
        let rest = it.next().unwrap_or("");
        match kind {
            "1" | "2" | "u" => {
                // "1 XY sub mH mI mW hH hI path"
                // "2 XY sub mH mI mW hH hI Xscore path" + NUL + origPath
                // "u XY sub m1 m2 m3 mW h1 h2 h3 path"
                let n_meta = match kind {
                    "1" => 7,
                    "2" => 8,
                    _ => 9,
                };
                let mut parts = rest.splitn(n_meta + 1, ' ');
                let xy: Vec<char> = parts.next().unwrap_or("..").chars().collect();
                let path = parts.nth(n_meta - 1).unwrap_or("").to_string();
                let orig_path = if kind == "2" { fields.next() } else { None };
                let (staged, unstaged) = if kind == "u" {
                    ('.', 'U')
                } else {
                    (
                        xy.first().copied().unwrap_or('.'),
                        xy.get(1).copied().unwrap_or('.'),
                    )
                };
                st.changes.push(Change {
                    path,
                    orig_path,
                    staged,
                    unstaged,
                });
            }
            "?" => st.changes.push(Change {
                path: rest.to_string(),
                orig_path: None,
                staged: '.',
                unstaged: '?',
            }),
            _ => {} // "!" ignored entries are not requested
        }
    }
    st
}

const LOG_FORMAT: &str = "%H%x1f%P%x1f%an%x1f%at%x1f%D%x1f%s%x1e";

/// Parse `git log --format=LOG_FORMAT`.
pub fn parse_log(text: &str) -> Vec<Commit> {
    text.split('\x1e')
        .filter(|r| !r.trim().is_empty())
        .filter_map(|r| {
            let f: Vec<&str> = r.trim_start_matches('\n').split('\x1f').collect();
            if f.len() < 6 {
                return None;
            }
            Some(Commit {
                hash: f[0].to_string(),
                parents: f[1].split_whitespace().map(String::from).collect(),
                author: f[2].to_string(),
                time: f[3].parse().unwrap_or(0),
                refs: f[4].to_string(),
                subject: f[5].to_string(),
            })
        })
        .collect()
}

const REF_FORMAT: &str = "%(refname)%1f%(HEAD)%1f%(upstream:short)%1f%(objectname:short)";

/// Parse `git for-each-ref --format=REF_FORMAT refs/heads refs/remotes refs/tags`.
pub fn parse_refs(text: &str) -> Vec<Ref> {
    text.lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split('\x1f').collect();
            if f.len() < 4 {
                return None;
            }
            let (kind, name) = if let Some(n) = f[0].strip_prefix("refs/heads/") {
                (RefKind::Local, n)
            } else if let Some(n) = f[0].strip_prefix("refs/remotes/") {
                if n.ends_with("/HEAD") {
                    return None;
                }
                (RefKind::Remote, n)
            } else {
                (RefKind::Tag, f[0].strip_prefix("refs/tags/")?)
            };
            Some(Ref {
                name: name.to_string(),
                kind,
                current: f[1] == "*",
                upstream: f[2].to_string(),
                hash: f[3].to_string(),
            })
        })
        .collect()
}

const SHOW_FORMAT: &str = "%H%x1f%P%x1f%an%x1f%ae%x1f%at%x1f%D%x1f%B%x1e";

/// Parse `git show --format=SHOW_FORMAT --name-status <hash>`.
pub fn parse_detail(text: &str) -> Option<CommitDetail> {
    let (rec, files) = text.split_once('\x1e')?;
    let f: Vec<&str> = rec.split('\x1f').collect();
    if f.len() < 7 {
        return None;
    }
    Some(CommitDetail {
        hash: f[0].to_string(),
        parents: f[1].split_whitespace().map(String::from).collect(),
        author: f[2].to_string(),
        email: f[3].to_string(),
        time: f[4].parse().unwrap_or(0),
        refs: f[5].to_string(),
        message: f[6].trim().to_string(),
        files: files
            .lines()
            .filter_map(|l| {
                let (st, path) = l.split_once('\t')?;
                // renames: "R100\told\tnew" — show the new path
                let path = path.rsplit('\t').next().unwrap_or(path);
                Some((st.chars().next()?, path.to_string()))
            })
            .collect(),
    })
}

/// One row of the commit graph: where the commit's node sits and which lane lines cross the row.
/// Lane `x` positions are up to the drawer; the numbers here are lane indices.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GraphRow {
    /// Lane of this commit's node.
    pub lane: usize,
    /// Lanes whose line comes down from the row above into the node (children's lanes).
    pub into: Vec<usize>,
    /// Lanes passing straight through this row without touching the node.
    pub through: Vec<usize>,
    /// Lanes leaving the node towards the row below (one per parent).
    pub out: Vec<usize>,
    /// Number of lanes in use on this row (for the column width).
    pub width: usize,
}

/// Assign lanes to commits in `--topo-order` (children before parents), newest first.
/// A lane "waits" for a hash: the first parent keeps the commit's lane, further parents take
/// the lane already waiting for them or a free one; a lane is freed when its commit appears.
pub fn graph(commits: &[Commit]) -> Vec<GraphRow> {
    let mut lanes: Vec<Option<&str>> = Vec::new();
    let mut rows = Vec::with_capacity(commits.len());
    let free = |lanes: &mut Vec<Option<&str>>| -> usize {
        match lanes.iter().position(Option::is_none) {
            Some(i) => i,
            None => {
                lanes.push(None);
                lanes.len() - 1
            }
        }
    };
    for c in commits {
        let into: Vec<usize> = lanes
            .iter()
            .enumerate()
            .filter(|(_, l)| **l == Some(c.hash.as_str()))
            .map(|(i, _)| i)
            .collect();
        let lane = match into.first() {
            Some(&i) => i,
            None => free(&mut lanes),
        };
        let through: Vec<usize> = lanes
            .iter()
            .enumerate()
            .filter(|(i, l)| l.is_some() && !into.contains(i) && *i != lane)
            .map(|(i, _)| i)
            .collect();
        for &i in &into {
            lanes[i] = None;
        }
        let mut out = Vec::with_capacity(c.parents.len());
        for (k, p) in c.parents.iter().enumerate() {
            let target = if k == 0 {
                lane
            } else if let Some(i) = lanes.iter().position(|l| *l == Some(p.as_str())) {
                i
            } else {
                free(&mut lanes)
            };
            lanes[target] = Some(p.as_str());
            out.push(target);
        }
        rows.push(GraphRow {
            lane,
            into,
            through,
            out,
            width: lanes.len().max(lane + 1),
        });
        while lanes.last().is_some_and(Option::is_none) {
            lanes.pop();
        }
    }
    rows
}

pub struct Git {
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    /// Label of the streaming command in flight (only one at a time).
    running: Option<String>,
    /// Read queries in flight (status / log / refs / diff / detail).
    pending: usize,
}

impl Default for Git {
    fn default() -> Self {
        Self::new()
    }
}

impl Git {
    pub fn new() -> Self {
        let (tx, rx) = channel();
        Self {
            tx,
            rx,
            running: None,
            pending: 0,
        }
    }

    pub fn running(&self) -> Option<&str> {
        self.running.as_deref()
    }

    pub fn busy(&self) -> bool {
        self.pending > 0 || self.running.is_some()
    }

    pub fn poll(&mut self) -> Vec<Msg> {
        let mut out = Vec::new();
        while let Ok(m) = self.rx.try_recv() {
            match &m {
                Msg::Exit { .. } => self.running = None,
                Msg::Line { .. } => {}
                _ => self.pending = self.pending.saturating_sub(1),
            }
            out.push(m);
        }
        out
    }

    fn spawn_query(&mut self, ctx: egui::Context, f: impl FnOnce() -> Msg + Send + 'static) {
        self.pending += 1;
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(f());
            ctx.request_repaint();
        });
    }

    pub fn fetch_status(&mut self, repo: PathBuf, ctx: egui::Context) {
        self.spawn_query(ctx, move || {
            Msg::Status(
                query(
                    &repo,
                    &[
                        "status",
                        "--porcelain=v2",
                        "-z",
                        "--branch",
                        "--untracked-files=all",
                    ],
                )
                .map(|o| parse_status(&o)),
            )
        });
    }

    /// Newest `limit` commits reachable from any ref.
    pub fn fetch_log(&mut self, repo: PathBuf, limit: usize, ctx: egui::Context) {
        self.spawn_query(ctx, move || {
            let n = format!("-n{limit}");
            Msg::Log(
                query(
                    &repo,
                    &[
                        "log",
                        "--all",
                        "--topo-order",
                        &n,
                        &format!("--format={LOG_FORMAT}"),
                    ],
                )
                .map(|o| parse_log(&lossy(&o))),
            )
        });
    }

    pub fn fetch_refs(&mut self, repo: PathBuf, ctx: egui::Context) {
        self.spawn_query(ctx, move || {
            Msg::Refs(
                query(
                    &repo,
                    &[
                        "for-each-ref",
                        &format!("--format={REF_FORMAT}"),
                        "refs/heads",
                        "refs/remotes",
                        "refs/tags",
                    ],
                )
                .map(|o| parse_refs(&lossy(&o))),
            )
        });
    }

    pub fn fetch_diff(&mut self, repo: PathBuf, key: DiffKey, untracked: bool, ctx: egui::Context) {
        self.spawn_query(ctx, move || {
            let result = match &key {
                DiffKey::Unstaged(p) if untracked => {
                    // `--no-index` exits 1 when the files differ, which is the normal case
                    let o = base_command(&repo)
                        .args(["diff", "--no-index", "--", "/dev/null", p])
                        .output()
                        .map_err(|e| e.to_string());
                    o.and_then(|o| match o.status.code() {
                        Some(0 | 1) => Ok(o.stdout),
                        _ => Err(lossy(&o.stderr).trim().to_string()),
                    })
                }
                DiffKey::Unstaged(p) => query(&repo, &["diff", "--", p]),
                DiffKey::Staged(p) => query(&repo, &["diff", "--cached", "--", p]),
                DiffKey::Commit(h, p) => query(&repo, &["show", "--format=", h, "--", p]),
            };
            Msg::Diff(key, result.map(|o| parse_diff(&lossy(&o))))
        });
    }

    pub fn fetch_detail(&mut self, repo: PathBuf, hash: String, ctx: egui::Context) {
        self.spawn_query(ctx, move || {
            Msg::Detail(
                query(
                    &repo,
                    &[
                        "show",
                        &format!("--format={SHOW_FORMAT}"),
                        "--name-status",
                        &hash,
                    ],
                )
                .and_then(|o| parse_detail(&lossy(&o)).ok_or_else(|| "unparsable show".into())),
            )
        });
    }

    /// Run a mutating git command, streaming output. `stdin` is written to the child (patches
    /// for `apply`). Returns false if one is already running.
    pub fn run(
        &mut self,
        repo: PathBuf,
        label: String,
        args: Vec<String>,
        stdin: Option<String>,
        ctx: egui::Context,
    ) -> bool {
        if self.running.is_some() {
            return false;
        }
        self.running = Some(label.clone());
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let t0 = std::time::Instant::now();
            let mut cmd = base_command(&repo);
            cmd.args(&args)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            if stdin.is_some() {
                cmd.stdin(Stdio::piped());
            }
            let mut child = match cmd.spawn() {
                Ok(c) => c,
                Err(e) => {
                    let _ = tx.send(Msg::Line {
                        text: format!("cannot start git: {e}"),
                        stderr: true,
                    });
                    let _ = tx.send(Msg::Exit {
                        label,
                        args,
                        ok: false,
                        code: None,
                        elapsed_ms: 0.0,
                    });
                    ctx.request_repaint();
                    return;
                }
            };
            if let (Some(text), Some(mut pipe)) = (stdin, child.stdin.take()) {
                use std::io::Write as _;
                let _ = pipe.write_all(text.as_bytes());
            }
            let mut readers = Vec::new();
            for (stream, is_err) in [
                (
                    child
                        .stdout
                        .take()
                        .map(|s| Box::new(s) as Box<dyn std::io::Read + Send>),
                    false,
                ),
                (
                    child
                        .stderr
                        .take()
                        .map(|s| Box::new(s) as Box<dyn std::io::Read + Send>),
                    true,
                ),
            ] {
                let Some(stream) = stream else { continue };
                let tx = tx.clone();
                let ctx = ctx.clone();
                readers.push(std::thread::spawn(move || {
                    // progress output (fetch / push) uses CR to redraw one line; split on it too
                    let mut reader = BufReader::new(stream);
                    let mut buf = Vec::new();
                    loop {
                        buf.clear();
                        match reader.read_until(b'\n', &mut buf) {
                            Ok(0) | Err(_) => break,
                            Ok(_) => {}
                        }
                        for piece in lossy(&buf).split('\r') {
                            let text = piece.trim_end().to_string();
                            if text.is_empty() {
                                continue;
                            }
                            let _ = tx.send(Msg::Line {
                                text,
                                stderr: is_err,
                            });
                        }
                        ctx.request_repaint();
                    }
                }));
            }
            let status = child.wait();
            for r in readers {
                let _ = r.join();
            }
            let (ok, code) = match status {
                Ok(s) => (s.success(), s.code()),
                Err(_) => (false, None),
            };
            let _ = tx.send(Msg::Exit {
                label,
                args,
                ok,
                code,
                elapsed_ms: t0.elapsed().as_secs_f32() * 1000.0,
            });
            ctx.request_repaint();
        });
        true
    }
}

/// `2026-09-04 12:34` in local time.
pub fn fmt_time(unix: i64) -> String {
    chrono::DateTime::from_timestamp(unix, 0)
        .map(|t| {
            t.with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M")
                .to_string()
        })
        .unwrap_or_else(|| "--".into())
}

/// `3 min ago`, `2 h ago`, `5 d ago`.
pub fn fmt_ago(unix: i64, now: i64) -> String {
    let d = (now - unix).max(0);
    match d {
        0..=59 => "now".into(),
        60..=3599 => format!("{} min ago", d / 60),
        3600..=86_399 => format!("{} h ago", d / 3600),
        86_400..=2_591_999 => format!("{} d ago", d / 86_400),
        _ => fmt_time(unix),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_v2_parses_headers_ordinary_renames_and_untracked() {
        let raw = b"# branch.oid abc123\0# branch.head main\0# branch.upstream origin/main\0# branch.ab +2 -1\0\
1 .M N... 100644 100644 100644 aaaa bbbb src/app.rs\0\
1 A. N... 000000 100644 100644 0000 cccc new file.txt\0\
2 R. N... 100644 100644 100644 dddd dddd R100 new/name.rs\0old/name.rs\0\
u UU N... 100644 100644 100644 100644 e1 e2 e3 conflict.rs\0\
? notes.md\0";
        let st = parse_status(raw);
        assert_eq!(st.branch, "main");
        assert_eq!(st.upstream, "origin/main");
        assert_eq!((st.ahead, st.behind), (2, 1));
        assert_eq!(st.changes.len(), 5);
        assert_eq!(st.changes[0].path, "src/app.rs");
        assert_eq!((st.changes[0].staged, st.changes[0].unstaged), ('.', 'M'));
        assert_eq!(st.changes[1].path, "new file.txt");
        assert!(st.changes[1].has_staged() && !st.changes[1].has_unstaged());
        assert_eq!(st.changes[2].path, "new/name.rs");
        assert_eq!(st.changes[2].orig_path.as_deref(), Some("old/name.rs"));
        assert!(st.changes[3].is_unmerged());
        assert!(st.changes[4].is_untracked());
        assert!(!st.changes[4].has_staged());
    }

    #[test]
    fn log_and_refs_parse() {
        let log = "a1\x1fp1 p2\x1fkobago\x1f1700000000\x1fHEAD -> main, origin/main\x1ffirst: subject\x1e\nb2\x1f\x1fsomeone\x1f1600000000\x1f\x1froot\x1e\n";
        let c = parse_log(log);
        assert_eq!(c.len(), 2);
        assert_eq!(c[0].parents, ["p1", "p2"]);
        assert_eq!(c[0].subject, "first: subject");
        assert!(c[1].parents.is_empty());
        assert_eq!(c[1].refs, "");

        let refs = "refs/heads/main\x1f*\x1forigin/main\x1fa1b2c3d\nrefs/heads/feat\x1f \x1f\x1f1234567\nrefs/remotes/origin/HEAD\x1f \x1f\x1fa1b2c3d\nrefs/remotes/origin/main\x1f \x1f\x1fa1b2c3d\nrefs/tags/v1\x1f \x1f\x1f9999999\n";
        let r = parse_refs(refs);
        assert_eq!(r.len(), 4, "origin/HEAD is dropped");
        assert!(r[0].current && r[0].kind == RefKind::Local && r[0].upstream == "origin/main");
        assert_eq!(r[2].kind, RefKind::Remote);
        assert_eq!((r[3].name.as_str(), r[3].kind), ("v1", RefKind::Tag));
    }

    #[test]
    fn diff_lines_numbers_and_hunk_patches() {
        let text = "diff --git a/f b/f\nindex 1..2 100644\n--- a/f\n+++ b/f\n@@ -1,3 +1,3 @@\n a\n-b\n+B\n c\n@@ -10,2 +10,3 @@ fn x()\n d\n+e\n f\n";
        let d = parse_diff(text);
        assert_eq!(d.hunks.len(), 2);
        assert_eq!((d.adds, d.dels), (2, 1));
        assert_eq!(
            d.header,
            "diff --git a/f b/f\nindex 1..2 100644\n--- a/f\n+++ b/f\n"
        );
        assert_eq!(
            d.hunk_patch(1),
            "diff --git a/f b/f\nindex 1..2 100644\n--- a/f\n+++ b/f\n@@ -10,2 +10,3 @@ fn x()\n d\n+e\n f\n"
        );
        let del = &d.lines[6];
        assert_eq!(
            (del.kind, del.old_no, del.new_no),
            (LineKind::Del, Some(2), None)
        );
        let add = &d.lines[7];
        assert_eq!(
            (add.kind, add.old_no, add.new_no),
            (LineKind::Add, None, Some(2))
        );
        let ctx = &d.lines[8];
        assert_eq!(
            (ctx.old_no, ctx.new_no, ctx.text.as_str()),
            (Some(3), Some(3), "c")
        );
        assert_eq!(d.lines[9].kind, LineKind::Hunk);
        assert_eq!(d.lines[11].new_no, Some(11));
    }

    #[test]
    fn detail_parses_message_and_files() {
        let text = "abc\x1fp1\x1fkobago\x1fk@x\x1f1700000000\x1ftag: v1\x1fsubject\n\nbody line\n\x1e\nM\tsrc/a.rs\nR100\told.rs\tnew.rs\nA\tnew.txt\n";
        let d = parse_detail(text).unwrap();
        assert_eq!(d.message, "subject\n\nbody line");
        assert_eq!(d.email, "k@x");
        assert_eq!(
            d.files,
            vec![
                ('M', "src/a.rs".into()),
                ('R', "new.rs".into()),
                ('A', "new.txt".into())
            ]
        );
    }

    #[test]
    fn graph_lanes_branch_and_merge() {
        // e (merge of d and c) ← d ← b ← a ; c ← b   (topo order: e d c b a)
        let mk = |h: &str, ps: &[&str]| Commit {
            hash: h.into(),
            parents: ps.iter().map(|s| s.to_string()).collect(),
            author: String::new(),
            time: 0,
            refs: String::new(),
            subject: String::new(),
        };
        let commits = [
            mk("e", &["d", "c"]),
            mk("d", &["b"]),
            mk("c", &["b"]),
            mk("b", &["a"]),
            mk("a", &[]),
        ];
        let g = graph(&commits);
        // e: new lane 0, first parent d stays in lane 0, second parent c opens lane 1
        assert_eq!(
            (g[0].lane, &g[0].out[..], &g[0].into[..]),
            (0, &[0, 1][..], &[][..])
        );
        // d: lane 0, lane 1 (waiting for c) passes through
        assert_eq!(
            (g[1].lane, &g[1].through[..], &g[1].into[..]),
            (0, &[1][..], &[0][..])
        );
        // c: lane 1 keeps its lane down to the fork point (first parent b); lane 0 passes through
        assert_eq!(
            (g[2].lane, &g[2].out[..], &g[2].through[..]),
            (1, &[1][..], &[0][..])
        );
        // b: both lanes come into the node (lane 0 owns it, lane 1 merges in)
        assert_eq!(
            (g[3].lane, &g[3].into[..], &g[3].out[..]),
            (0, &[0, 1][..], &[0][..])
        );
        assert_eq!(g[3].width, 2);
        // a: root, nothing goes out; width back to 1
        assert_eq!((g[4].lane, &g[4].out[..], g[4].width), (0, &[][..], 1));
    }

    #[test]
    fn ago() {
        assert_eq!(fmt_ago(1000, 1030), "now");
        assert_eq!(fmt_ago(1000, 1000 + 180), "3 min ago");
        assert_eq!(fmt_ago(1000, 1000 + 7200), "2 h ago");
        assert_eq!(fmt_ago(1000, 1000 + 5 * 86_400), "5 d ago");
    }
}
