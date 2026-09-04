//! FUIDE Git — a git client with a tactical-console look. Everything goes through the `git`
//! CLI (`crate::git`).

use std::collections::VecDeque;
use std::path::{Path, PathBuf};

use egui::{pos2, vec2, Align2, Key, Rect, RichText, Sense, Stroke, Ui};
use fuide::table::{self, Cell, Column, TableState, Width};
use fuide::widgets::{self, LogLine};
use fuide::{
    mono, palette, theme, type_scale, Dialog, PaletteKind, Panel, Settings, SettingsWindow, Shell,
};

use crate::git::{
    self, Change, Commit, CommitDetail, Diff, DiffKey, Git, LineKind, Msg, Ref, RefKind, Status,
};

const LEFT_W: f32 = 240.0;
const RIGHT_W: f32 = 340.0;
const GAP: f32 = 14.0;
const TOOLBAR_H: f32 = 32.0;
const LOG_H: f32 = 140.0;
const LOG_MIN: f32 = 60.0;
const LOG_CLOSED: f32 = 26.0;
const BODY_MIN: f32 = 360.0;
const REPO_H: f32 = 210.0;
/// Commits read per `git log` (older history is not loaded yet).
const LOG_LIMIT: usize = 500;
const RECENT_MAX: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum View {
    Changes,
    History,
}

/// Which list the arrow keys / Space act on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Focus {
    Unstaged,
    Staged,
    History,
}

#[derive(Clone, Copy)]
enum Level {
    Info,
    Ok,
    Warn,
    Danger,
}

struct Event {
    time: String,
    text: String,
    level: Level,
}

/// A confirmation before a git command that changes something the user may not expect.
struct Confirm {
    title: String,
    line: String,
    note: String,
    verb: String,
    danger: bool,
    label: String,
    args: Vec<String>,
}

enum DialogState {
    Confirm(Confirm),
    Notice {
        success: bool,
        line: String,
    },
    /// Open a repository by path (Tab completes directory names).
    Open {
        input: String,
        error: Option<String>,
        focus: bool,
        suggestions: Vec<String>,
        suggested_for: String,
    },
    /// Create a branch at HEAD and switch to it.
    Branch {
        input: String,
        focus: bool,
    },
}

struct OpenDialog {
    state: DialogState,
    closing: bool,
}

enum Action {
    SetView(View),
    Open(PathBuf),
    OpenDialog,
    CompleteOpen,
    ConfirmOpen,
    Select(Focus, Option<usize>),
    SelectDetailFile(Option<usize>),
    ToggleSelected,
    Stage(Vec<String>),
    Unstage(Vec<String>),
    StageAll,
    UnstageAll,
    StageHunk(usize),
    UnstageHunk(usize),
    Discard(String, bool),
    Commit,
    Switch(String),
    BranchDialog,
    ConfirmBranch,
    Fetch,
    Pull,
    Push,
    Copy(String),
    Refresh,
    Run(String, Vec<String>, Option<String>),
    CloseDialog,
    ConfirmDialog,
    OpenSettings,
}

/// Settings file name (`Settings::path`).
const APP_ID: &str = "git";

pub struct GitApp {
    git: Git,
    repo: Option<PathBuf>,
    recent: Vec<PathBuf>,
    recent_path: Option<PathBuf>,
    status: Status,
    commits: Vec<Commit>,
    /// Lane layout of `commits` (`git::graph`), rebuilt with the log.
    graph: Vec<git::GraphRow>,
    refs: Vec<Ref>,
    view: View,
    focus: Focus,
    /// Indices into `status.changes` with something in the worktree / in the index.
    unstaged: Vec<usize>,
    staged: Vec<usize>,
    unstaged_table: TableState,
    staged_table: TableState,
    hist_table: TableState,
    detail: Option<CommitDetail>,
    detail_table: TableState,
    /// The diff on screen and the key it was requested for.
    diff: Option<(DiffKey, Diff)>,
    diff_want: Option<DiffKey>,
    commit_msg: String,
    log: Vec<Event>,
    dialog: Option<OpenDialog>,
    notice_queue: VecDeque<(bool, String)>,
    last_error: Option<String>,
    devshot: fuide::devshot::DevShot,
    agent: fuide::Agent,
    dev_dialog: Option<String>,
    dev_frame: u32,
    dev_close_frame: Option<u32>,
    settings: Settings,
    settings_win: SettingsWindow,
    settings_path: Option<PathBuf>,
    log_h: f32,
    now: i64,
}

impl GitApp {
    /// Production entry point: settings + recent list from disk, then open `start`, the most
    /// recent repository, or the repository around the current directory.
    pub fn new(cc: &eframe::CreationContext<'_>, start: Option<PathBuf>) -> Self {
        let settings = Settings::load(APP_ID).unwrap_or_else(|| Settings::new(PaletteKind::Green));
        let mut app = Self::with_context(&cc.egui_ctx, settings);
        if let Some(path) = Settings::path(APP_ID) {
            app.settings_path = Some(path);
        }
        if let Some(path) = Settings::path("git-recent") {
            app.recent = std::fs::read_to_string(&path)
                .map(|t| {
                    t.lines()
                        .filter(|l| !l.is_empty())
                        .map(PathBuf::from)
                        .collect()
                })
                .unwrap_or_default();
            app.recent_path = Some(path);
        }
        let start = start
            .or_else(|| app.recent.first().cloned())
            .or_else(|| std::env::current_dir().ok());
        if let Some(p) = start {
            app.apply(&cc.egui_ctx, Action::Open(p), 0.0);
        }
        app
    }

    /// Build the app on any `egui::Context` without opening anything (tests).
    pub fn with_context(ctx: &egui::Context, settings: Settings) -> Self {
        theme::install(ctx, settings.palette.palette(), theme::macos_cjk_fallback());
        settings.apply(ctx);
        let mut app = Self {
            git: Git::new(),
            repo: None,
            recent: Vec::new(),
            recent_path: None,
            status: Status::default(),
            commits: Vec::new(),
            graph: Vec::new(),
            refs: Vec::new(),
            view: View::Changes,
            focus: Focus::Unstaged,
            unstaged: Vec::new(),
            staged: Vec::new(),
            unstaged_table: TableState::default(),
            staged_table: TableState::default(),
            hist_table: TableState::default(),
            detail: None,
            detail_table: TableState::default(),
            diff: None,
            diff_want: None,
            commit_msg: String::new(),
            log: Vec::new(),
            dialog: None,
            notice_queue: VecDeque::new(),
            last_error: None,
            devshot: fuide::devshot::DevShot::from_env(),
            agent: fuide::Agent::new(APP_ID, "FUIDE Git"),
            dev_dialog: std::env::var("FUIDE_DEV_DIALOG").ok(),
            dev_frame: 0,
            dev_close_frame: std::env::var("FUIDE_DEV_DIALOG_CLOSE")
                .ok()
                .and_then(|v| v.parse().ok()),
            log_h: settings.log_height.unwrap_or(LOG_H),
            settings,
            settings_win: SettingsWindow::default(),
            settings_path: None,
            now: chrono::Utc::now().timestamp(),
        };
        app.push_log(0.0, "git console online", Level::Ok);
        app.agent.set_enabled(ctx, app.settings.agent);
        if app.settings.agent {
            app.push_log(
                0.0,
                "agent // interface on :: waiting for a client",
                Level::Warn,
            );
        }
        if std::env::var_os("FUIDE_DEV_SETTINGS").is_some() {
            app.settings_win.open();
        }
        if let Ok(text) = std::env::var("FUIDE_DEV_LOG") {
            app.push_log(0.0, text, Level::Danger);
        }
        app
    }

    // ------------------------------------------------------------------ state

    fn push_log(&mut self, t: f64, text: impl Into<String>, level: Level) {
        self.log.push(Event {
            time: format!("[{}]", fuide::fmt::uptime(t)),
            text: text.into(),
            level,
        });
        if self.log.len() > 2000 {
            self.log.drain(..500);
        }
    }

    fn fail(&mut self, t: f64, line: impl Into<String>, detail: &str) {
        let line = line.into();
        self.push_log(t, format!("{line} :: failed :: {detail}"), Level::Danger);
        self.notice_queue.push_back((false, line));
    }

    fn agent_blocked(&self) -> Vec<String> {
        match &self.dialog {
            Some(OpenDialog {
                state: DialogState::Confirm(c),
                closing: false,
            }) if !self.settings.agent_confirm => vec![c.verb.clone()],
            _ => Vec::new(),
        }
    }

    fn agent_state(&self) -> String {
        use std::fmt::Write as _;
        let mut s = String::new();
        match &self.repo {
            Some(r) => {
                let _ = writeln!(
                    s,
                    "repo: {} :: branch {} :: upstream {} :: ahead {} behind {}",
                    r.display(),
                    self.status.branch,
                    if self.status.upstream.is_empty() {
                        "-"
                    } else {
                        &self.status.upstream
                    },
                    self.status.ahead,
                    self.status.behind
                );
            }
            None => s.push_str("repo: none (OPEN or Cmd+O)\n"),
        }
        let _ = writeln!(
            s,
            "view: {:?} :: unstaged {} :: staged {} :: commits {}",
            self.view,
            self.unstaged.len(),
            self.staged.len(),
            self.commits.len()
        );
        for (name, list, st) in [
            ("unstaged", &self.unstaged, &self.unstaged_table),
            ("staged", &self.staged, &self.staged_table),
        ] {
            let paths: Vec<&str> = list
                .iter()
                .take(30)
                .map(|&i| self.status.changes[i].path.as_str())
                .collect();
            let _ = writeln!(s, "{name}: {}", paths.join(", "));
            if let Some(c) = st.selected.and_then(|r| list.get(r)) {
                let _ = writeln!(s, "{name} selected: {}", self.status.changes[*c].path);
            }
        }
        if let Some(c) = self.selected_commit() {
            let _ = writeln!(s, "selected commit: {} {}", c.short(), c.subject);
        }
        if let Some((key, d)) = &self.diff {
            let _ = writeln!(s, "diff: {key:?} :: +{} -{}", d.adds, d.dels);
        }
        if !self.commit_msg.is_empty() {
            let _ = writeln!(s, "commit message: {:?}", self.commit_msg);
        }
        if let Some(l) = self.git.running() {
            let _ = writeln!(s, "running: {l}");
        }
        if let Some(e) = &self.last_error {
            let _ = writeln!(s, "last error: {e}");
        }
        match self.dialog.as_ref().filter(|d| !d.closing) {
            Some(OpenDialog {
                state: DialogState::Confirm(c),
                ..
            }) => {
                let _ = writeln!(
                    s,
                    "dialog: CONFIRM {} :: {} :: git {} :: buttons CANCEL / {}",
                    c.title.to_uppercase(),
                    c.line,
                    c.args.join(" "),
                    c.verb
                );
            }
            Some(OpenDialog {
                state: DialogState::Notice { success, line },
                ..
            }) => {
                let _ = writeln!(
                    s,
                    "dialog: {} :: {line} :: press ACKNOWLEDGE",
                    if *success { "SUCCESS" } else { "ERROR" }
                );
            }
            Some(OpenDialog {
                state: DialogState::Open { input, error, .. },
                ..
            }) => {
                let _ = writeln!(s, "dialog: OPEN REPOSITORY :: input {input:?}");
                if let Some(e) = error {
                    let _ = writeln!(s, "  error: {e}");
                }
            }
            Some(OpenDialog {
                state: DialogState::Branch { input, .. },
                ..
            }) => {
                let _ = writeln!(s, "dialog: NEW BRANCH :: input {input:?}");
            }
            None => {}
        }
        s.push_str("log (latest last):\n");
        let skip = self.log.len().saturating_sub(6);
        for e in &self.log[skip..] {
            let _ = writeln!(s, "  {} {}", e.time, e.text);
        }
        s
    }

    fn change_at(&self, focus: Focus, row: usize) -> Option<&Change> {
        let list = match focus {
            Focus::Unstaged => &self.unstaged,
            Focus::Staged => &self.staged,
            Focus::History => return None,
        };
        list.get(row).map(|&i| &self.status.changes[i])
    }

    fn selected_change(&self, focus: Focus) -> Option<&Change> {
        let st = match focus {
            Focus::Unstaged => &self.unstaged_table,
            Focus::Staged => &self.staged_table,
            Focus::History => return None,
        };
        st.selected.and_then(|r| self.change_at(focus, r))
    }

    fn selected_commit(&self) -> Option<&Commit> {
        self.hist_table.selected.and_then(|r| self.commits.get(r))
    }

    /// The diff the current selection asks for, and whether it is an untracked file.
    fn wanted_diff(&self) -> Option<(DiffKey, bool)> {
        match self.view {
            View::Changes => match self.focus {
                Focus::Unstaged => self
                    .selected_change(Focus::Unstaged)
                    .map(|c| (DiffKey::Unstaged(c.path.clone()), c.is_untracked())),
                Focus::Staged => self
                    .selected_change(Focus::Staged)
                    .map(|c| (DiffKey::Staged(c.path.clone()), false)),
                Focus::History => None,
            },
            View::History => {
                let d = self.detail.as_ref()?;
                let commit = self.selected_commit()?;
                if d.hash != commit.hash {
                    return None;
                }
                let (_, path) = self.detail_table.selected.and_then(|r| d.files.get(r))?;
                Some((DiffKey::Commit(d.hash.clone(), path.clone()), false))
            }
        }
    }

    /// Ask for the diff of the selection when it is not the one on screen / in flight.
    fn sync_diff(&mut self, ctx: &egui::Context) {
        let Some(repo) = self.repo.clone() else {
            return;
        };
        match self.wanted_diff() {
            None => {
                self.diff = None;
                self.diff_want = None;
            }
            Some((key, untracked)) => {
                if self.diff_want.as_ref() != Some(&key) {
                    self.diff_want = Some(key.clone());
                    self.git.fetch_diff(repo, key, untracked, ctx.clone());
                }
            }
        }
        // history: the detail follows the selected commit
        if self.view == View::History {
            if let Some(c) = self.selected_commit() {
                if self.detail.as_ref().map(|d| d.hash.as_str()) != Some(c.hash.as_str()) {
                    let hash = c.hash.clone();
                    self.detail = Some(CommitDetail {
                        hash: hash.clone(),
                        ..Default::default()
                    });
                    self.detail_table.selected = None;
                    if let Some(repo) = self.repo.clone() {
                        self.git.fetch_detail(repo, hash, ctx.clone());
                    }
                }
            }
        }
    }

    fn refresh(&mut self, ctx: &egui::Context) {
        let Some(repo) = self.repo.clone() else {
            return;
        };
        self.git.fetch_status(repo.clone(), ctx.clone());
        self.git.fetch_log(repo.clone(), LOG_LIMIT, ctx.clone());
        self.git.fetch_refs(repo, ctx.clone());
    }

    /// A new status arrived: rebuild the two lists and keep the selections on the same paths.
    fn set_status(&mut self, st: Status) {
        let keep_u = self
            .selected_change(Focus::Unstaged)
            .map(|c| c.path.clone());
        let keep_s = self.selected_change(Focus::Staged).map(|c| c.path.clone());
        self.status = st;
        self.unstaged = (0..self.status.changes.len())
            .filter(|&i| self.status.changes[i].has_unstaged())
            .collect();
        self.staged = (0..self.status.changes.len())
            .filter(|&i| self.status.changes[i].has_staged())
            .collect();
        let find = |list: &[usize], path: Option<String>, changes: &[Change]| -> Option<usize> {
            let p = path?;
            list.iter().position(|&i| changes[i].path == p)
        };
        self.unstaged_table.selected = find(&self.unstaged, keep_u, &self.status.changes);
        self.staged_table.selected = find(&self.staged, keep_s, &self.status.changes);
        // the file contents changed: re-read whatever diff is on screen
        self.diff_want = None;
    }

    fn poll(&mut self, ctx: &egui::Context, t: f64) {
        for msg in self.git.poll() {
            match msg {
                Msg::Status(Ok(st)) => {
                    self.set_status(st);
                    self.last_error = None;
                }
                Msg::Status(Err(e)) => {
                    self.last_error = Some(e.clone());
                    self.fail(t, "status", &e);
                }
                Msg::Log(Ok(commits)) => {
                    let keep = self.selected_commit().map(|c| c.hash.clone());
                    self.graph = git::graph(&commits);
                    self.commits = commits;
                    self.hist_table.selected =
                        keep.and_then(|h| self.commits.iter().position(|c| c.hash == h));
                }
                Msg::Log(Err(e)) => self.push_log(t, format!("log :: {e}"), Level::Warn),
                Msg::Refs(Ok(refs)) => self.refs = refs,
                Msg::Refs(Err(e)) => self.push_log(t, format!("refs :: {e}"), Level::Warn),
                Msg::Diff(key, result) => {
                    if self.diff_want.as_ref() == Some(&key) {
                        match result {
                            Ok(d) => self.diff = Some((key, d)),
                            Err(e) => {
                                self.push_log(t, format!("diff :: {e}"), Level::Warn);
                                self.diff = None;
                            }
                        }
                    }
                }
                Msg::Detail(Ok(d)) => {
                    if self.detail.as_ref().map(|x| x.hash.as_str()) == Some(d.hash.as_str()) {
                        self.detail = Some(d);
                        if self.detail_table.selected.is_none() {
                            self.detail_table.selected = Some(0);
                        }
                    }
                }
                Msg::Detail(Err(e)) => self.push_log(t, format!("show :: {e}"), Level::Warn),
                Msg::Line { text, stderr } => {
                    let level = if text.starts_with("error") || text.starts_with("fatal") {
                        Level::Danger
                    } else if text.starts_with("warning") || text.starts_with("hint") {
                        Level::Warn
                    } else if stderr {
                        Level::Info
                    } else {
                        Level::Ok
                    };
                    self.push_log(t, text, level);
                }
                Msg::Exit {
                    label,
                    args,
                    ok,
                    code,
                    elapsed_ms,
                } => {
                    if ok {
                        self.push_log(
                            t,
                            format!(
                                "{label} :: git {} done in {:.2} s",
                                args.first().map(String::as_str).unwrap_or(""),
                                elapsed_ms / 1000.0
                            ),
                            Level::Ok,
                        );
                        match args.first().map(String::as_str) {
                            Some("commit") => {
                                self.commit_msg.clear();
                                self.notice_queue.push_back((true, label));
                            }
                            Some("fetch" | "pull" | "push") => {
                                self.notice_queue.push_back((true, label))
                            }
                            _ => {}
                        }
                    } else {
                        let detail = match code {
                            Some(c) => format!("exit code {c}"),
                            None => "terminated".into(),
                        };
                        self.fail(t, label, &detail);
                    }
                    self.refresh(ctx);
                }
            }
        }
    }

    fn confirm(&mut self, c: Confirm) {
        self.dialog = Some(OpenDialog {
            state: DialogState::Confirm(c),
            closing: false,
        });
    }

    fn remember(&mut self, repo: &Path) {
        self.recent.retain(|r| r != repo);
        self.recent.insert(0, repo.to_path_buf());
        self.recent.truncate(RECENT_MAX);
        if let Some(path) = &self.recent_path {
            let text: String = self
                .recent
                .iter()
                .map(|p| format!("{}\n", p.display()))
                .collect();
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let _ = std::fs::write(path, text);
        }
    }

    fn apply(&mut self, ctx: &egui::Context, action: Action, t: f64) {
        match action {
            Action::SetView(v) => {
                if self.view != v {
                    self.view = v;
                    self.focus = match v {
                        View::Changes => Focus::Unstaged,
                        View::History => Focus::History,
                    };
                }
            }
            Action::Open(path) => match git::toplevel(&path) {
                Ok(top) => {
                    if self.repo.as_ref() == Some(&top) {
                        self.refresh(ctx);
                        return;
                    }
                    self.push_log(t, format!("open // {}", top.display()), Level::Ok);
                    self.repo = Some(top.clone());
                    self.remember(&top);
                    self.status = Status::default();
                    self.commits.clear();
                    self.graph.clear();
                    self.refs.clear();
                    self.unstaged.clear();
                    self.staged.clear();
                    self.unstaged_table = TableState::default();
                    self.staged_table = TableState::default();
                    self.hist_table = TableState::default();
                    self.detail = None;
                    self.diff = None;
                    self.diff_want = None;
                    self.commit_msg.clear();
                    self.last_error = None;
                    self.refresh(ctx);
                }
                Err(e) => {
                    if let Some(OpenDialog {
                        state: DialogState::Open { error, .. },
                        ..
                    }) = &mut self.dialog
                    {
                        *error = Some(e);
                    } else {
                        self.fail(t, format!("open // {}", path.display()), &e);
                    }
                }
            },
            Action::OpenDialog => {
                let input = self
                    .repo
                    .as_ref()
                    .map(|r| format!("{}/", r.display()))
                    .unwrap_or_else(|| "~/".into());
                self.dialog = Some(OpenDialog {
                    state: DialogState::Open {
                        input,
                        error: None,
                        focus: true,
                        suggestions: Vec::new(),
                        suggested_for: String::from("\0"),
                    },
                    closing: false,
                });
            }
            Action::CompleteOpen => {
                let Some(OpenDialog {
                    state:
                        DialogState::Open {
                            input, suggestions, ..
                        },
                    closing: false,
                }) = &mut self.dialog
                else {
                    return;
                };
                let filled = match suggestions.len() {
                    0 => return,
                    1 => suggestions[0].clone(),
                    _ => fuide::pathinput::common_prefix(suggestions),
                };
                if filled.len() > input.len() {
                    *input = filled;
                }
            }
            Action::ConfirmOpen => {
                let Some(OpenDialog {
                    state: DialogState::Open { input, .. },
                    closing: false,
                }) = &self.dialog
                else {
                    return;
                };
                let cwd = self
                    .repo
                    .clone()
                    .or_else(|| std::env::current_dir().ok())
                    .unwrap_or_else(|| PathBuf::from("/"));
                let path = fuide::pathinput::expand(input.trim(), &cwd);
                self.apply(ctx, Action::Open(path), t);
                // still open with an error = the path was not a repository
                if let Some(OpenDialog {
                    state: DialogState::Open { error: None, .. },
                    closing,
                }) = &mut self.dialog
                {
                    *closing = true;
                }
            }
            Action::Select(focus, row) => {
                self.focus = focus;
                let st = match focus {
                    Focus::Unstaged => &mut self.unstaged_table,
                    Focus::Staged => &mut self.staged_table,
                    Focus::History => &mut self.hist_table,
                };
                st.selected = row;
                st.scroll_to_selected = true;
            }
            Action::SelectDetailFile(row) => {
                self.detail_table.selected = row;
            }
            Action::ToggleSelected => {
                if let Some(c) = self.selected_change(self.focus) {
                    let path = c.path.clone();
                    let a = match self.focus {
                        Focus::Unstaged => Action::Stage(vec![path]),
                        Focus::Staged => Action::Unstage(vec![path]),
                        Focus::History => return,
                    };
                    self.apply(ctx, a, t);
                }
            }
            Action::Stage(paths) => {
                let label = format!("stage // {}", short_list(&paths));
                let mut args = vec!["add".to_string(), "--".to_string()];
                args.extend(paths);
                self.apply(ctx, Action::Run(label, args, None), t);
            }
            Action::Unstage(paths) => {
                let label = format!("unstage // {}", short_list(&paths));
                let mut args = vec![
                    "restore".to_string(),
                    "--staged".to_string(),
                    "--".to_string(),
                ];
                args.extend(paths);
                self.apply(ctx, Action::Run(label, args, None), t);
            }
            Action::StageAll => self.apply(
                ctx,
                Action::Run("stage // all".into(), vec!["add".into(), "-A".into()], None),
                t,
            ),
            Action::UnstageAll => self.apply(
                ctx,
                Action::Run(
                    "unstage // all".into(),
                    vec!["reset".into(), "-q".into()],
                    None,
                ),
                t,
            ),
            Action::StageHunk(i) | Action::UnstageHunk(i) => {
                let Some((key, d)) = &self.diff else { return };
                let Some(patch) = d.hunks.get(i).map(|_| d.hunk_patch(i)) else {
                    return;
                };
                let (verb, path, extra) = match key {
                    DiffKey::Unstaged(p) => ("stage hunk", p.clone(), None),
                    DiffKey::Staged(p) => ("unstage hunk", p.clone(), Some("-R")),
                    DiffKey::Commit(..) => return,
                };
                let mut args = vec!["apply".to_string(), "--cached".to_string()];
                args.extend(extra.map(String::from));
                self.apply(
                    ctx,
                    Action::Run(format!("{verb} // {path} #{}", i + 1), args, Some(patch)),
                    t,
                );
            }
            Action::Discard(path, untracked) => {
                let args: Vec<String> = if untracked {
                    vec!["clean".into(), "-f".into(), "--".into(), path.clone()]
                } else {
                    vec!["restore".into(), "--".into(), path.clone()]
                };
                self.confirm(Confirm {
                    title: "Discard changes".into(),
                    line: path.clone(),
                    note: if untracked {
                        "DELETES THE FILE. THIS CANNOT BE UNDONE".into()
                    } else {
                        "THROWS AWAY THE WORKTREE CHANGES. THIS CANNOT BE UNDONE".into()
                    },
                    verb: "DISCARD".into(),
                    danger: true,
                    label: format!("discard // {path}"),
                    args,
                });
            }
            Action::Commit => {
                let msg = self.commit_msg.trim().to_string();
                if msg.is_empty() || self.staged.is_empty() {
                    return;
                }
                let label = format!(
                    "commit // {}",
                    msg.lines()
                        .next()
                        .unwrap_or("")
                        .chars()
                        .take(60)
                        .collect::<String>()
                );
                self.apply(
                    ctx,
                    Action::Run(
                        label,
                        vec!["commit".into(), "-F".into(), "-".into()],
                        Some(format!("{msg}\n")),
                    ),
                    t,
                );
            }
            Action::Switch(name) => {
                let r = self.refs.iter().find(|r| r.name == name);
                let args: Vec<String> = match r.map(|r| r.kind) {
                    Some(RefKind::Tag) => vec!["switch".into(), "--detach".into(), name.clone()],
                    Some(RefKind::Remote) => {
                        // `origin/feat` → local `feat` tracking it (git switch --guess)
                        let local = name.split_once('/').map(|(_, b)| b).unwrap_or(&name);
                        vec!["switch".into(), local.to_string()]
                    }
                    _ => vec!["switch".into(), name.clone()],
                };
                self.apply(ctx, Action::Run(format!("switch // {name}"), args, None), t);
            }
            Action::BranchDialog => {
                self.dialog = Some(OpenDialog {
                    state: DialogState::Branch {
                        input: String::new(),
                        focus: true,
                    },
                    closing: false,
                });
            }
            Action::ConfirmBranch => {
                let Some(OpenDialog {
                    state: DialogState::Branch { input, .. },
                    closing,
                }) = &mut self.dialog
                else {
                    return;
                };
                let name = input.trim().to_string();
                if name.is_empty() || *closing {
                    return;
                }
                *closing = true;
                self.apply(
                    ctx,
                    Action::Run(
                        format!("branch // {name}"),
                        vec!["switch".into(), "-c".into(), name],
                        None,
                    ),
                    t,
                );
            }
            Action::Fetch => self.apply(
                ctx,
                Action::Run(
                    "fetch // all".into(),
                    vec!["fetch".into(), "--all".into(), "--prune".into()],
                    None,
                ),
                t,
            ),
            Action::Pull => self.apply(
                ctx,
                Action::Run("pull".into(), vec!["pull".into(), "--ff-only".into()], None),
                t,
            ),
            Action::Push => {
                let mut args = vec!["push".to_string()];
                if self.status.upstream.is_empty() && self.status.branch != "(detached)" {
                    args.extend(["-u".into(), "origin".into(), self.status.branch.clone()]);
                }
                self.apply(ctx, Action::Run("push".into(), args, None), t);
            }
            Action::Copy(text) => {
                ctx.copy_text(text);
                self.push_log(t, "copied to clipboard", Level::Info);
            }
            Action::Refresh => {
                self.push_log(t, "refresh", Level::Info);
                self.refresh(ctx);
            }
            Action::Run(label, args, stdin) => {
                let Some(repo) = self.repo.clone() else {
                    self.fail(t, label, "no repository open");
                    return;
                };
                if self
                    .git
                    .run(repo, label.clone(), args.clone(), stdin, ctx.clone())
                {
                    self.push_log(t, format!("$ git {}", args.join(" ")), Level::Info);
                } else {
                    self.fail(t, label, "another git command is still running");
                }
            }
            Action::CloseDialog => {
                if let Some(d) = &mut self.dialog {
                    d.closing = true;
                }
            }
            Action::ConfirmDialog => {
                let Some(OpenDialog {
                    state: DialogState::Confirm(c),
                    closing,
                }) = &mut self.dialog
                else {
                    return;
                };
                if *closing {
                    return;
                }
                *closing = true;
                let (label, args) = (c.label.clone(), c.args.clone());
                self.apply(ctx, Action::Run(label, args, None), t);
            }
            Action::OpenSettings => self.settings_win.open(),
        }
    }

    fn settings_changed(&mut self, t: f64) {
        let s = &self.settings;
        self.push_log(
            t,
            format!(
                "settings // palette {} :: {} :: {} :: {} :: agent {}{}",
                s.palette.name(),
                if s.chamfer { "chamfer" } else { "square" },
                if s.compact { "compact" } else { "normal" },
                if s.transparent {
                    "translucent"
                } else {
                    "opaque"
                },
                if s.agent { "on" } else { "off" },
                if s.agent && s.agent_confirm {
                    " (may confirm)"
                } else {
                    ""
                }
            ),
            Level::Warn,
        );
        self.save_settings(t);
    }

    fn save_settings(&mut self, t: f64) {
        if let Some(path) = self.settings_path.clone() {
            if let Err(e) = self.settings.save_to(&path) {
                self.push_log(t, format!("settings // save failed: {e}"), Level::Danger);
            }
        }
    }

    fn handle_keys(&self, ui: &Ui, actions: &mut Vec<Action>) {
        let focused = ui.memory(|m| m.focused().is_some());
        ui.input(|i| {
            let cmd = i.modifiers.command;
            // Cmd+Enter commits even from inside the message field
            if cmd && i.key_pressed(Key::Enter) && self.dialog.is_none() {
                actions.push(Action::Commit);
            }
        });
        if self.dialog.is_some() || focused {
            return;
        }
        ui.input(|i| {
            let cmd = i.modifiers.command;
            if i.key_pressed(Key::ArrowDown) || i.key_pressed(Key::ArrowUp) {
                let dir: isize = if i.key_pressed(Key::ArrowDown) { 1 } else { -1 };
                let (len, sel) = match self.focus {
                    Focus::Unstaged => (self.unstaged.len(), self.unstaged_table.selected),
                    Focus::Staged => (self.staged.len(), self.staged_table.selected),
                    Focus::History => (self.commits.len(), self.hist_table.selected),
                };
                if len > 0 {
                    let next = match sel {
                        Some(p) => (p as isize + dir).clamp(0, len as isize - 1) as usize,
                        None => 0,
                    };
                    actions.push(Action::Select(self.focus, Some(next)));
                }
            }
            if i.key_pressed(Key::Space) || i.key_pressed(Key::Enter) {
                actions.push(Action::ToggleSelected);
            }
            if cmd && i.key_pressed(Key::A) && self.view == View::Changes {
                actions.push(Action::StageAll);
            }
            if cmd && i.key_pressed(Key::R) {
                actions.push(Action::Refresh);
            }
            if cmd && i.key_pressed(Key::O) {
                actions.push(Action::OpenDialog);
            }
            if cmd && i.key_pressed(Key::Comma) {
                actions.push(Action::OpenSettings);
            }
            if cmd && i.key_pressed(Key::Num1) {
                actions.push(Action::SetView(View::Changes));
            }
            if cmd && i.key_pressed(Key::Num2) {
                actions.push(Action::SetView(View::History));
            }
        });
    }
}

fn short_list(paths: &[String]) -> String {
    match paths {
        [one] => one.clone(),
        many => format!("{} files", many.len()),
    }
}

// ---------------------------------------------------------------------------- UI

impl eframe::App for GitApp {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0; 4]
    }

    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        self.devshot.tick(ui.ctx());
        self.agent.set_enabled(ui.ctx(), self.settings.agent);
        self.agent.set_blocked(self.agent_blocked());
        let agent_state = self.agent.wants_state().then(|| self.agent_state());
        self.agent.tick(ui.ctx(), agent_state);
        let t = ui.input(|i| i.time);
        let ctx = ui.ctx().clone();
        self.now = chrono::Utc::now().timestamp();
        self.poll(&ctx, t);
        self.sync_diff(&ctx);
        let pal = palette(ui.ctx());
        let fps = 1.0 / ui.input(|i| i.stable_dt).max(1e-3);

        let mut actions: Vec<Action> = Vec::new();
        if self.dialog.is_none() {
            if let Some((success, line)) = self.notice_queue.pop_front() {
                self.dialog = Some(OpenDialog {
                    state: DialogState::Notice { success, line },
                    closing: false,
                });
            }
        }
        self.dev_frame += 1;
        if let Some(kind) = self.dev_dialog.take_if(|_| !self.git.busy()) {
            actions.push(match kind.as_str() {
                "discard" => match self.change_at(Focus::Unstaged, 0) {
                    Some(c) => Action::Discard(c.path.clone(), c.is_untracked()),
                    None => Action::Refresh,
                },
                "open" => Action::OpenDialog,
                "diff" => Action::Select(Focus::Unstaged, Some(0)),
                "history" => {
                    self.view = View::History;
                    Action::Select(Focus::History, Some(0))
                }
                "branch" => Action::BranchDialog,
                "error" => {
                    self.notice_queue.push_back((false, "push".into()));
                    Action::Refresh
                }
                "success" => {
                    self.notice_queue
                        .push_back((true, "commit // example".into()));
                    Action::Refresh
                }
                _ => Action::Refresh,
            });
        }
        if self.dev_close_frame == Some(self.dev_frame) && self.dialog.is_some() {
            actions.push(Action::CloseDialog);
        }
        self.handle_keys(ui, &mut actions);
        // a directory dropped from Finder / the file manager opens that repository
        let dropped: Vec<PathBuf> = ui.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .collect()
        });
        if let Some(p) = dropped.into_iter().next() {
            actions.push(Action::Open(p));
        }

        let (link_text, link_color) = match (&self.repo, &self.last_error) {
            (None, _) => ("NO REPOSITORY", pal.text_dim),
            (_, None) => ("GIT LINK OK", pal.ok),
            (_, Some(_)) => ("GIT LINK FAILED", pal.danger),
        };
        let subtitle = match &self.repo {
            Some(r) => format!(
                "{} :: {}",
                r.file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                self.status.branch
            ),
            None => "no repository".into(),
        };
        let mut shell = Shell::new("FUIDE Git")
            .subtitle(subtitle)
            .status_left(format!(
                "{} :: {} UNSTAGED :: {} STAGED :: {} COMMITS :: {:.0} FPS",
                fuide::fmt::uptime(t),
                self.unstaged.len(),
                self.staged.len(),
                self.commits.len(),
                fps
            ))
            .lamp(link_text, link_color, false)
            .settings_button(true);
        if self.git.busy() && self.git.running().is_none() {
            shell = shell.lamp("READING", pal.warn, true);
        }
        if let Some(label) = self.git.running() {
            shell = shell.lamp(
                format!("GIT {}", label.split(" //").next().unwrap_or("")).to_uppercase(),
                pal.warn,
                true,
            );
        }
        if let Some((text, busy)) = self.agent.lamp() {
            shell = shell.lamp(text, if busy { pal.warn } else { pal.accent }, busy);
        }

        let log_open = self.settings.log_open;
        let mut log_resized = false;
        let mut log_toggled = false;
        let out = shell.show_full(ui, |ui| {
            let c = ui.max_rect();
            let top = c.top() + 10.0;
            let log_max = c.height() - BODY_MIN;
            self.log_h = self.log_h.clamp(LOG_MIN, log_max.max(LOG_MIN));
            let log_h = if log_open { self.log_h } else { LOG_CLOSED };
            let log_rect = Rect::from_min_max(pos2(c.left(), c.bottom() - log_h), c.max);
            let body_bottom = log_rect.top() - GAP - 8.0;
            let left =
                Rect::from_min_max(pos2(c.left(), top), pos2(c.left() + LEFT_W, body_bottom));
            let right =
                Rect::from_min_max(pos2(c.right() - RIGHT_W, top), pos2(c.right(), body_bottom));
            let center = Rect::from_min_max(
                pos2(left.right() + GAP, top),
                pos2(right.left() - GAP, body_bottom),
            );
            let repo = Rect::from_min_size(left.min, vec2(left.width(), REPO_H));
            let refs = Rect::from_min_max(pos2(left.left(), repo.bottom() + GAP + 8.0), left.max);
            let toolbar = Rect::from_min_size(
                pos2(center.left(), center.top() - 8.0),
                vec2(center.width(), TOOLBAR_H),
            );
            let body = Rect::from_min_max(pos2(center.left(), toolbar.bottom() + 12.0), center.max);
            let lists_h = (body.height() * 0.38).max(120.0);
            let lists = Rect::from_min_size(body.min, vec2(body.width(), lists_h));
            let diff = Rect::from_min_max(pos2(body.left(), lists.bottom() + GAP + 8.0), body.max);

            self.ui_repo(ui, repo, &mut actions);
            self.ui_refs(ui, refs, &mut actions);
            self.ui_toolbar(ui, toolbar, &mut actions);
            match self.view {
                View::Changes => {
                    let half = (lists.width() - GAP) / 2.0;
                    let ul = Rect::from_min_size(lists.min, vec2(half, lists.height()));
                    let sl = Rect::from_min_size(pos2(ul.right() + GAP, lists.top()), ul.size());
                    self.ui_changes(ui, ul, Focus::Unstaged, &mut actions);
                    self.ui_changes(ui, sl, Focus::Staged, &mut actions);
                    self.ui_commit(ui, right, &mut actions);
                }
                View::History => {
                    self.ui_history(ui, lists, &mut actions);
                    self.ui_detail(ui, right, &mut actions);
                }
            }
            self.ui_diff(ui, diff, &mut actions);
            if log_open {
                let strip = Rect::from_min_max(
                    pos2(c.left(), body_bottom),
                    pos2(c.right(), log_rect.top()),
                );
                let resp = widgets::h_splitter(
                    ui,
                    strip,
                    "log",
                    &mut self.log_h,
                    LOG_MIN,
                    log_max,
                    "LOG HEIGHT",
                );
                log_resized = resp.drag_stopped();
            }
            log_toggled = self.ui_log(ui, log_rect, log_open);
        });
        self.agent.paint(&ctx);
        if out.settings_clicked {
            actions.push(Action::OpenSettings);
        }
        if log_resized {
            self.settings.log_height = Some(self.log_h.round());
            self.save_settings(t);
        }
        if log_toggled {
            self.settings.log_open = !log_open;
            self.save_settings(t);
        }

        self.ui_dialog(&ctx, &mut actions);
        for a in actions {
            self.apply(&ctx, a, t);
        }
        self.sync_diff(&ctx);
        self.settings_win
            .set_agent_status(&ctx, &self.agent.status_line());
        if self
            .settings_win
            .show(&ctx, &mut self.settings, "FUIDE Git")
        {
            self.settings_changed(t);
        }
    }
}

impl GitApp {
    fn ui_repo(&self, ui: &mut Ui, rect: Rect, actions: &mut Vec<Action>) {
        let pal = palette(ui.ctx());
        let ts = type_scale(ui.ctx());
        let busy = self.git.running().is_some();
        Panel::new("Repository")
            .padding(12.0, 14.0)
            .show_rect(ui, rect, |ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                match &self.repo {
                    Some(r) => {
                        ui.add(
                            egui::Label::new(
                                RichText::new(
                                    r.file_name()
                                        .map(|n| n.to_string_lossy().into_owned())
                                        .unwrap_or_default(),
                                )
                                .font(mono(ts.data + 3.0))
                                .color(pal.accent),
                            )
                            .truncate(),
                        );
                        ui.add(
                            egui::Label::new(
                                RichText::new(r.display().to_string())
                                    .font(mono(ts.small))
                                    .color(pal.text_dim),
                            )
                            .truncate(),
                        );
                        ui.add_space(4.0);
                        widgets::rule(ui);
                        widgets::readout(ui, "branch", &self.status.branch, Some(pal.accent));
                        widgets::readout(
                            ui,
                            "upstream",
                            if self.status.upstream.is_empty() {
                                "--"
                            } else {
                                &self.status.upstream
                            },
                            None,
                        );
                        let (a, b) = (self.status.ahead, self.status.behind);
                        widgets::readout(
                            ui,
                            "ahead / behind",
                            &format!("+{a} / -{b}"),
                            Some(if b > 0 {
                                pal.warn
                            } else if a > 0 {
                                pal.ok
                            } else {
                                pal.text
                            }),
                        );
                    }
                    None => {
                        ui.add_space(4.0);
                        ui.label(
                            RichText::new("OPEN A REPOSITORY")
                                .font(mono(ts.label))
                                .color(pal.text_dim),
                        );
                        ui.label(
                            RichText::new("CMD+O, OR DROP A FOLDER HERE")
                                .font(mono(ts.small))
                                .color(pal.text_dim),
                        );
                    }
                }
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    if widgets::button(ui, vec2(72.0, ts.row), "OPEN", true).clicked() {
                        actions.push(Action::OpenDialog);
                    }
                    if widgets::button(
                        ui,
                        vec2(72.0, ts.row),
                        "FETCH",
                        !busy && self.repo.is_some(),
                    )
                    .clicked()
                    {
                        actions.push(Action::Fetch);
                    }
                });
                let others: Vec<&PathBuf> = self
                    .recent
                    .iter()
                    .filter(|r| Some(*r) != self.repo.as_ref())
                    .take(3)
                    .collect();
                if !others.is_empty() {
                    ui.add_space(4.0);
                    widgets::section_label(ui, "Recent");
                    for r in others {
                        let name = r
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_else(|| r.display().to_string());
                        if widgets::nav_tab(ui, &name, false)
                            .on_hover_text(r.display().to_string())
                            .clicked()
                        {
                            actions.push(Action::Open(r.clone()));
                        }
                    }
                }
            });
    }

    fn ui_refs(&self, ui: &mut Ui, rect: Rect, actions: &mut Vec<Action>) {
        let pal = palette(ui.ctx());
        let ts = type_scale(ui.ctx());
        let busy = self.git.running().is_some();
        let n = self
            .refs
            .iter()
            .filter(|r| r.kind == RefKind::Local)
            .count();
        Panel::new("Branches")
            .tag(format!("{n} local"), pal.text_dim)
            .show_rect(ui, rect, |ui| {
                ui.spacing_mut().item_spacing.y = 3.0;
                if widgets::button(
                    ui,
                    vec2(ui.available_width(), ts.row),
                    "NEW BRANCH",
                    !busy && self.repo.is_some(),
                )
                .clicked()
                {
                    actions.push(Action::BranchDialog);
                }
                ui.add_space(2.0);
                egui::ScrollArea::vertical()
                    .id_salt("refs")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        for (kind, title) in [
                            (RefKind::Local, "Local"),
                            (RefKind::Remote, "Remotes"),
                            (RefKind::Tag, "Tags"),
                        ] {
                            let group: Vec<&Ref> =
                                self.refs.iter().filter(|r| r.kind == kind).collect();
                            if group.is_empty() {
                                continue;
                            }
                            widgets::section_label(ui, title);
                            for r in group {
                                let resp = widgets::nav_tab(ui, &r.name, r.current);
                                if !r.upstream.is_empty() {
                                    let rr = resp.rect;
                                    ui.painter().with_clip_rect(rr).text(
                                        pos2(rr.right() - 8.0, rr.center().y),
                                        Align2::RIGHT_CENTER,
                                        &r.upstream,
                                        mono(ts.small),
                                        pal.text_dim,
                                    );
                                }
                                if resp.double_clicked() && !r.current && !busy {
                                    actions.push(Action::Switch(r.name.clone()));
                                }
                            }
                            ui.add_space(4.0);
                        }
                    });
            });
    }

    fn ui_toolbar(&mut self, ui: &mut Ui, rect: Rect, actions: &mut Vec<Action>) {
        let pal = palette(ui.ctx());
        let ts = type_scale(ui.ctx());
        let busy = self.git.running().is_some();
        let has_repo = self.repo.is_some();
        let mut left = ui.new_child(
            egui::UiBuilder::new()
                .id_salt("toolbar-left")
                .max_rect(rect)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        left.set_clip_rect(rect.intersect(ui.clip_rect()));
        {
            let ui = &mut left;
            ui.spacing_mut().item_spacing.x = 6.0;
            if widgets::icon_button(ui, vec2(32.0, ts.row), widgets::Icon::Refresh, has_repo)
                .on_hover_text("Re-read the repository (Cmd+R)")
                .clicked()
            {
                actions.push(Action::Refresh);
            }
            for (v, label) in [
                (
                    View::Changes,
                    format!("CHANGES  {}", self.unstaged.len() + self.staged.len()),
                ),
                (View::History, format!("HISTORY  {}", self.commits.len())),
            ] {
                let on = self.view == v;
                if widgets::button_colored(
                    ui,
                    vec2(130.0, ts.row),
                    &label,
                    true,
                    if on { pal.accent } else { pal.text_dim },
                )
                .clicked()
                {
                    actions.push(Action::SetView(v));
                }
            }
        }
        let mut right = ui.new_child(
            egui::UiBuilder::new()
                .id_salt("toolbar-right")
                .max_rect(rect)
                .layout(egui::Layout::right_to_left(egui::Align::Center)),
        );
        {
            let ui = &mut right;
            ui.spacing_mut().item_spacing.x = 6.0;
            let can_net = !busy && has_repo;
            let push_label = if self.status.ahead > 0 {
                format!("PUSH  {}", self.status.ahead)
            } else {
                "PUSH".into()
            };
            if widgets::button_colored(
                ui,
                vec2(84.0, ts.row),
                &push_label,
                can_net,
                if self.status.ahead > 0 {
                    pal.ok
                } else {
                    pal.accent
                },
            )
            .clicked()
            {
                actions.push(Action::Push);
            }
            let pull_label = if self.status.behind > 0 {
                format!("PULL  {}", self.status.behind)
            } else {
                "PULL".into()
            };
            if widgets::button_colored(
                ui,
                vec2(84.0, ts.row),
                &pull_label,
                can_net,
                if self.status.behind > 0 {
                    pal.warn
                } else {
                    pal.accent
                },
            )
            .clicked()
            {
                actions.push(Action::Pull);
            }
        }
    }

    fn ui_changes(&mut self, ui: &mut Ui, rect: Rect, focus: Focus, actions: &mut Vec<Action>) {
        let pal = palette(ui.ctx());
        let ts = type_scale(ui.ctx());
        let busy = self.git.running().is_some();
        let (title, list, salt) = match focus {
            Focus::Unstaged => ("Unstaged", &self.unstaged, "unstaged"),
            _ => ("Staged", &self.staged, "staged"),
        };
        // PATH first: the table labels a row by its first cell, so agents / tests address rows by path
        let columns = [
            Column::new("PATH", Width::Flex).unsortable(),
            Column::new("ST", Width::Chars(8.0)).unsortable(),
        ];
        let changes = &self.status.changes;
        let mut state = std::mem::take(match focus {
            Focus::Unstaged => &mut self.unstaged_table,
            _ => &mut self.staged_table,
        });
        let active = self.focus == focus;
        let resp = Panel::new(title)
            .tag(
                format!("{} files", list.len()),
                if active { pal.accent } else { pal.text_dim },
            )
            .padding(8.0, 12.0)
            .show_rect(ui, rect, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    match focus {
                        Focus::Unstaged => {
                            if widgets::button(
                                ui,
                                vec2(100.0, ts.row),
                                "STAGE ALL",
                                !busy && !list.is_empty(),
                            )
                            .clicked()
                            {
                                actions.push(Action::StageAll);
                            }
                            let sel = state
                                .selected
                                .and_then(|r| list.get(r))
                                .map(|&i| &changes[i]);
                            if widgets::button_colored(
                                ui,
                                vec2(90.0, ts.row),
                                "DISCARD",
                                !busy && sel.is_some(),
                                pal.danger,
                            )
                            .clicked()
                            {
                                if let Some(c) = sel {
                                    actions.push(Action::Discard(c.path.clone(), c.is_untracked()));
                                }
                            }
                        }
                        _ => {
                            if widgets::button(
                                ui,
                                vec2(110.0, ts.row),
                                "UNSTAGE ALL",
                                !busy && !list.is_empty(),
                            )
                            .clicked()
                            {
                                actions.push(Action::UnstageAll);
                            }
                        }
                    }
                });
                ui.add_space(4.0);
                table::table(ui, salt, &columns, list.len(), &mut state, |row, col| {
                    let c = &changes[list[row]];
                    let letter = match focus {
                        Focus::Unstaged => c.unstaged,
                        _ => c.staged,
                    };
                    match col {
                        0 => match &c.orig_path {
                            Some(o) => Cell::text(format!("{o} → {}", c.path)),
                            None => Cell::text(&c.path),
                        },
                        _ => Cell::tag(git::status_tag(letter)).color(match letter {
                            'A' | '?' => pal.ok,
                            'D' | 'U' => pal.danger,
                            _ => pal.warn,
                        }),
                    }
                })
            });
        if let Some(r) = resp.clicked {
            actions.push(Action::Select(focus, Some(r)));
        }
        if let Some(r) = resp.double_clicked {
            if let Some(c) = list.get(r).map(|&i| &changes[i]) {
                actions.push(match focus {
                    Focus::Unstaged => Action::Stage(vec![c.path.clone()]),
                    _ => Action::Unstage(vec![c.path.clone()]),
                });
            }
        }
        match focus {
            Focus::Unstaged => self.unstaged_table = state,
            _ => self.staged_table = state,
        }
    }

    fn ui_history(&mut self, ui: &mut Ui, rect: Rect, actions: &mut Vec<Action>) {
        let pal = palette(ui.ctx());
        // graph column: one slot per lane, capped so a wild history cannot eat the subject
        const LANE_W: f32 = 12.0;
        const LANES_MAX: usize = 8;
        let lanes = self
            .graph
            .iter()
            .map(|g| g.width)
            .max()
            .unwrap_or(1)
            .clamp(1, LANES_MAX);
        let columns = [
            Column::new("", Width::Fixed(LANE_W * lanes as f32 + 10.0)).unsortable(),
            Column::new("HASH", Width::Chars(8.0)).unsortable(),
            Column::new("SUBJECT", Width::Flex).unsortable(),
            Column::new("AUTHOR", Width::Chars(14.0)).unsortable(),
            Column::new("WHEN", Width::Chars(12.0)).unsortable(),
        ];
        let commits = &self.commits;
        let graph = &self.graph;
        let now = self.now;
        let t = ui.input(|i| i.time);
        let head = &self.status.head;
        let mut state = std::mem::take(&mut self.hist_table);
        let lane_colors = [pal.accent, pal.ok, pal.warn, pal.accent_dim];
        let resp = Panel::new("History")
            .tag(format!("{} commits", commits.len()), pal.text_dim)
            .padding(8.0, 12.0)
            .show_rect(ui, rect, |ui| {
                table::table_decorated(
                    ui,
                    "history",
                    &columns,
                    commits.len(),
                    &mut state,
                    |row, col| {
                        let c = &commits[row];
                        match col {
                            0 => Cell::dim(""),
                            1 => Cell::dim(c.short()).color(if &c.hash == head {
                                pal.accent
                            } else {
                                pal.text_dim
                            }),
                            2 => {
                                if c.refs.is_empty() {
                                    Cell::text(&c.subject)
                                } else {
                                    Cell::text(format!("[{}] {}", c.refs, c.subject))
                                        .color(pal.accent)
                                }
                            }
                            3 => Cell::dim(&c.author),
                            _ => Cell::dim(git::fmt_ago(c.time, now)),
                        }
                    },
                    |p, row, r| {
                        // commit graph: lines top → node → bottom, node = ring (HEAD = filled + pulse)
                        let Some(g) = graph.get(row) else { return };
                        let x = |lane: usize| r.left() + 8.0 + LANE_W * lane as f32 + LANE_W / 2.0;
                        let color = |lane: usize| lane_colors[lane % lane_colors.len()];
                        let (top, bot, cy) = (r.top(), r.bottom(), r.center().y);
                        let node = egui::pos2(x(g.lane), cy);
                        let p = p.with_clip_rect(egui::Rect::from_min_max(
                            r.min,
                            egui::pos2(r.left() + LANE_W * lanes as f32 + 10.0, r.bottom()),
                        ));
                        for &l in &g.through {
                            fuide::geom::glow_line(
                                &p,
                                &[egui::pos2(x(l), top), egui::pos2(x(l), bot)],
                                color(l).gamma_multiply(0.8),
                                1.0,
                                3.0,
                            );
                        }
                        for &l in &g.into {
                            fuide::geom::glow_line(
                                &p,
                                &[egui::pos2(x(l), top), node],
                                color(l),
                                1.0,
                                3.0,
                            );
                        }
                        for &l in &g.out {
                            fuide::geom::glow_line(
                                &p,
                                &[node, egui::pos2(x(l), bot)],
                                color(l),
                                1.0,
                                3.0,
                            );
                        }
                        let c = color(g.lane);
                        let is_head = &commits[row].hash == head;
                        p.circle_filled(node, 4.5, pal.bg_deep);
                        if is_head {
                            let pulse = 0.5 + 0.5 * (t * 3.0).sin() as f32;
                            p.circle_stroke(
                                node,
                                6.5,
                                Stroke::new(1.0, c.gamma_multiply(0.3 + 0.4 * pulse)),
                            );
                            p.circle_filled(node, 3.5, c);
                        } else {
                            p.circle_stroke(node, 3.5, Stroke::new(1.5, c));
                        }
                    },
                )
            });
        self.hist_table = state;
        if let Some(r) = resp.clicked {
            actions.push(Action::Select(Focus::History, Some(r)));
        }
    }

    fn ui_commit(&mut self, ui: &mut Ui, rect: Rect, actions: &mut Vec<Action>) {
        let pal = palette(ui.ctx());
        let ts = type_scale(ui.ctx());
        let busy = self.git.running().is_some();
        let staged = self.staged.len();
        let can_commit = !busy && staged > 0 && !self.commit_msg.trim().is_empty();
        Panel::new("Commit")
            .tag(
                format!("{staged} staged"),
                if staged > 0 { pal.ok } else { pal.text_dim },
            )
            .show_rect(ui, rect, |ui| {
                ui.spacing_mut().item_spacing.y = 6.0;
                egui::Frame::new()
                    .stroke(Stroke::new(1.0, pal.accent_dim))
                    .inner_margin(6.0)
                    .show(ui, |ui| {
                        let resp = ui.add(
                            egui::TextEdit::multiline(&mut self.commit_msg)
                                .font(mono(ts.data))
                                .text_color(pal.text)
                                .desired_width(f32::INFINITY)
                                .desired_rows(8)
                                .frame(egui::Frame::NONE)
                                .hint_text(
                                    RichText::new("commit message (Cmd+Enter to commit)")
                                        .font(mono(ts.label))
                                        .color(pal.text_dim),
                                ),
                        );
                        // egui's TextEdit reports its own accesskit node; list it for the agent
                        let mut info = egui::WidgetInfo::text_edit(
                            true,
                            self.commit_msg.as_str(),
                            self.commit_msg.as_str(),
                            "commit message",
                        );
                        info.label = Some("COMMIT MESSAGE".into());
                        fuide::agent::note(&resp, &info);
                    });
                if widgets::button_colored(
                    ui,
                    vec2(ui.available_width(), ts.row + 4.0),
                    "COMMIT",
                    can_commit,
                    pal.ok,
                )
                .clicked()
                {
                    actions.push(Action::Commit);
                }
                ui.add_space(4.0);
                widgets::rule(ui);
                widgets::readout(ui, "branch", &self.status.branch, Some(pal.accent));
                widgets::readout(ui, "staged", &staged.to_string(), None);
                widgets::readout(ui, "unstaged", &self.unstaged.len().to_string(), None);
                if let Some(c) = self.commits.iter().find(|c| c.hash == self.status.head) {
                    ui.add_space(6.0);
                    widgets::readout(ui, "head", c.short(), None);
                    ui.add(
                        egui::Label::new(
                            RichText::new(&c.subject)
                                .font(mono(ts.label))
                                .color(pal.text_dim),
                        )
                        .wrap(),
                    );
                }
                let (fr, _) = ui.allocate_exact_size(
                    vec2(ui.available_width(), ts.small + 6.0),
                    Sense::hover(),
                );
                ui.painter().text(
                    pos2(fr.left(), fr.center().y),
                    Align2::LEFT_CENTER,
                    "SPACE STAGE/UNSTAGE  CMD+A STAGE ALL  CMD+1/2 VIEWS",
                    mono(ts.small),
                    pal.text_dim,
                );
            });
    }

    fn ui_detail(&mut self, ui: &mut Ui, rect: Rect, actions: &mut Vec<Action>) {
        let pal = palette(ui.ctx());
        let ts = type_scale(ui.ctx());
        let detail = self.detail.as_ref();
        let mut state = std::mem::take(&mut self.detail_table);
        let resp = Panel::new("Commit")
            .tag(
                detail
                    .map(|d| d.hash[..d.hash.len().min(8)].to_string())
                    .unwrap_or_else(|| "none".into()),
                pal.text_dim,
            )
            .show_rect(ui, rect, |ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                let Some(d) = detail.filter(|d| !d.author.is_empty()) else {
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new("SELECT A COMMIT")
                            .font(mono(ts.label))
                            .color(pal.text_dim),
                    );
                    return None;
                };
                ui.add(
                    egui::Label::new(
                        RichText::new(d.message.lines().next().unwrap_or(""))
                            .font(mono(ts.data + 2.0))
                            .color(pal.accent),
                    )
                    .wrap(),
                );
                let body: String = d.message.lines().skip(1).collect::<Vec<_>>().join("\n");
                if !body.trim().is_empty() {
                    ui.add(
                        egui::Label::new(
                            RichText::new(body.trim())
                                .font(mono(ts.label))
                                .color(pal.text),
                        )
                        .wrap(),
                    );
                }
                ui.add_space(4.0);
                widgets::rule(ui);
                widgets::readout(ui, "hash", &d.hash[..d.hash.len().min(12)], None);
                widgets::readout(ui, "author", &d.author, None);
                widgets::readout(ui, "date", &git::fmt_time(d.time), None);
                widgets::readout(
                    ui,
                    "parents",
                    &d.parents
                        .iter()
                        .map(|p| &p[..p.len().min(8)])
                        .collect::<Vec<_>>()
                        .join(" "),
                    None,
                );
                if !d.refs.is_empty() {
                    widgets::readout(ui, "refs", &d.refs, Some(pal.accent));
                }
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    if widgets::button(ui, vec2(96.0, ts.row), "COPY HASH", true).clicked() {
                        actions.push(Action::Copy(d.hash.clone()));
                    }
                });
                ui.add_space(6.0);
                let columns = [
                    Column::new("FILE", Width::Flex).unsortable(),
                    Column::new("ST", Width::Chars(6.0)).unsortable(),
                ];
                let files = &d.files;
                Some(table::table(
                    ui,
                    "detail-files",
                    &columns,
                    files.len(),
                    &mut state,
                    |row, col| {
                        let (st, path) = &files[row];
                        match col {
                            0 => Cell::text(path),
                            _ => Cell::tag(git::status_tag(*st)).color(match st {
                                'A' => pal.ok,
                                'D' => pal.danger,
                                _ => pal.warn,
                            }),
                        }
                    },
                ))
            });
        self.detail_table = state;
        if let Some(r) = resp.and_then(|r| r.clicked) {
            actions.push(Action::SelectDetailFile(Some(r)));
        }
    }

    fn ui_diff(&self, ui: &mut Ui, rect: Rect, actions: &mut Vec<Action>) {
        let pal = palette(ui.ctx());
        let ts = type_scale(ui.ctx());
        let busy = self.git.running().is_some();
        let (title, tag, tag_color) = match &self.diff {
            Some((key, d)) => (
                match key {
                    DiffKey::Unstaged(p) | DiffKey::Staged(p) | DiffKey::Commit(_, p) => p.clone(),
                },
                format!("+{} -{}", d.adds, d.dels),
                if d.adds >= d.dels { pal.ok } else { pal.danger },
            ),
            None => ("Diff".into(), String::new(), pal.text_dim),
        };
        let mut panel = Panel::new(title).padding(6.0, 12.0);
        if !tag.is_empty() {
            panel = panel.tag(tag, tag_color);
        }
        panel.show_rect(ui, rect, |ui| {
            let Some((key, d)) = &self.diff else {
                ui.add_space(4.0);
                ui.label(
                    RichText::new(if self.diff_want.is_some() {
                        "READING DIFF"
                    } else {
                        "SELECT A FILE"
                    })
                    .font(mono(ts.label))
                    .color(pal.text_dim),
                );
                return;
            };
            if d.lines.is_empty() {
                ui.add_space(4.0);
                ui.label(
                    RichText::new("NO TEXTUAL CHANGES")
                        .font(mono(ts.label))
                        .color(pal.text_dim),
                );
                return;
            }
            let hunk_verb = match key {
                DiffKey::Unstaged(_) if !d.header.contains("--- /dev/null") => Some("STAGE HUNK"),
                DiffKey::Staged(_) => Some("UNSTAGE HUNK"),
                _ => None,
            };
            let row_h = ts.label + 5.0;
            let font = mono(ts.label);
            let ch = ui
                .painter()
                .layout_no_wrap("0".into(), font.clone(), pal.text)
                .size()
                .x;
            let gutter = ch * 5.0;
            egui::ScrollArea::vertical()
                .id_salt("diff")
                .auto_shrink([false, false])
                .show_rows(ui, row_h, d.lines.len(), |ui, range| {
                    for i in range {
                        let l = &d.lines[i];
                        let (r, _) = ui
                            .allocate_exact_size(vec2(ui.available_width(), row_h), Sense::hover());
                        let p = ui.painter().with_clip_rect(r);
                        let (color, sign, bg) = match l.kind {
                            LineKind::Add => (pal.ok, "+", Some(pal.ok.gamma_multiply(0.08))),
                            LineKind::Del => {
                                (pal.danger, "-", Some(pal.danger.gamma_multiply(0.08)))
                            }
                            LineKind::Context => (pal.text.gamma_multiply(0.85), " ", None),
                            LineKind::Hunk => {
                                (pal.accent, "@", Some(pal.accent.gamma_multiply(0.07)))
                            }
                            LineKind::Meta => (pal.text_dim, " ", None),
                        };
                        if let Some(bg) = bg {
                            p.rect_filled(r, egui::CornerRadius::ZERO, bg);
                        }
                        let x0 = r.left() + 4.0;
                        for (n, x) in [(l.old_no, x0), (l.new_no, x0 + gutter)] {
                            if let Some(n) = n {
                                p.text(
                                    pos2(x + gutter - ch, r.center().y),
                                    Align2::RIGHT_CENTER,
                                    n.to_string(),
                                    font.clone(),
                                    pal.text_dim,
                                );
                            }
                        }
                        let tx = x0 + gutter * 2.0 + ch;
                        p.text(
                            pos2(tx, r.center().y),
                            Align2::LEFT_CENTER,
                            sign,
                            font.clone(),
                            color,
                        );
                        let text = l.text.replace('\t', "    ");
                        p.text(
                            pos2(tx + ch * 2.0, r.center().y),
                            Align2::LEFT_CENTER,
                            text,
                            font.clone(),
                            color,
                        );
                        if let (LineKind::Hunk, Some(verb), Some(h)) = (l.kind, hunk_verb, l.hunk) {
                            let w = ch * verb.len() as f32 + 20.0;
                            let br = Rect::from_min_max(
                                pos2(r.right() - w - 4.0, r.top()),
                                pos2(r.right() - 4.0, r.bottom()),
                            );
                            let mut child = ui.new_child(
                                egui::UiBuilder::new()
                                    .id_salt(("hunk", h))
                                    .max_rect(br)
                                    .layout(egui::Layout::right_to_left(egui::Align::Center)),
                            );
                            if widgets::button(&mut child, br.size(), verb, !busy).clicked() {
                                actions.push(match key {
                                    DiffKey::Staged(_) => Action::UnstageHunk(h),
                                    _ => Action::StageHunk(h),
                                });
                            }
                        }
                    }
                });
        });
    }

    fn ui_dialog(&mut self, ctx: &egui::Context, actions: &mut Vec<Action>) {
        let pal = palette(ctx);
        let ts = type_scale(ctx);
        let Some(OpenDialog { state, closing }) = &mut self.dialog else {
            return;
        };
        let open = !*closing;
        let enter = open && ctx.input(|i| i.key_pressed(Key::Enter));
        let tab = open && ctx.input(|i| i.key_pressed(Key::Tab));
        let finished;
        match state {
            DialogState::Confirm(c) => {
                let color = if c.danger { pal.danger } else { pal.warn };
                let resp = Dialog::new(&c.title)
                    .tag("git", pal.text_dim)
                    .outline(color)
                    .width(480.0)
                    .show(ctx, open, |ui| {
                        ui.spacing_mut().item_spacing.y = 4.0;
                        ui.add(
                            egui::Label::new(
                                RichText::new(&c.line)
                                    .font(mono(ts.data + 2.0))
                                    .color(pal.accent),
                            )
                            .wrap(),
                        );
                        ui.add_space(2.0);
                        widgets::readout(ui, "command", &format!("git {}", c.args.join(" ")), None);
                        ui.add_space(6.0);
                        widgets::rule(ui);
                        let (nr, _) = ui.allocate_exact_size(
                            vec2(ui.available_width(), ts.row),
                            Sense::hover(),
                        );
                        ui.painter().text(
                            pos2(nr.left() + 2.0, nr.center().y),
                            Align2::LEFT_CENTER,
                            &c.note,
                            mono(ts.label),
                            color,
                        );
                        ui.add_space(8.0);
                        fuide::dialog::button_row(
                            ui,
                            &[("CANCEL", pal.text_dim, true), (&c.verb, color, true)],
                        )
                    });
                finished = resp.finished;
                let clicked = resp.inner.flatten();
                if !open {
                } else if resp.should_close || clicked == Some(0) {
                    actions.push(Action::CloseDialog);
                } else if enter || clicked == Some(1) {
                    actions.push(Action::ConfirmDialog);
                }
            }
            DialogState::Notice { success, line } => {
                let (word, color) = if *success {
                    ("Success", pal.ok)
                } else {
                    ("Error", pal.danger)
                };
                let resp =
                    fuide::dialog::alert(ctx, open, word, line, "details :: git output", color);
                finished = resp.finished;
                if open && (resp.should_close || resp.inner == Some(true)) {
                    actions.push(Action::CloseDialog);
                }
            }
            DialogState::Open {
                input,
                error,
                focus,
                suggestions,
                suggested_for,
            } => {
                if *suggested_for != *input {
                    let cwd = self
                        .repo
                        .clone()
                        .or_else(|| std::env::current_dir().ok())
                        .unwrap_or_else(|| PathBuf::from("/"));
                    *suggestions = fuide::pathinput::complete(input, &cwd, false, 6);
                    *suggested_for = input.clone();
                }
                let resp = Dialog::new("Open repository")
                    .tag("path", pal.text_dim)
                    .width(600.0)
                    .show(ctx, open, |ui| {
                        ui.spacing_mut().item_spacing.y = 6.0;
                        let field =
                            widgets::text_input(ui, ui.available_width(), input, "repository path");
                        if *focus {
                            field.request_focus();
                            let mut st =
                                egui::TextEdit::load_state(ui.ctx(), field.id).unwrap_or_default();
                            st.cursor.set_char_range(Some(egui::text::CCursorRange::one(
                                egui::text::CCursor::new(input.chars().count()),
                            )));
                            st.store(ui.ctx(), field.id);
                            *focus = false;
                        }
                        if field.changed() {
                            *error = None;
                        }
                        let submit = field.lost_focus() && enter;
                        let complete = tab && (field.has_focus() || field.lost_focus());
                        if complete {
                            *focus = true;
                        }
                        let names: Vec<String> = suggestions
                            .iter()
                            .map(|s| {
                                s.trim_end_matches('/')
                                    .rsplit('/')
                                    .next()
                                    .unwrap_or(s)
                                    .to_string()
                            })
                            .collect();
                        let (sr, _) = ui.allocate_exact_size(
                            vec2(ui.available_width(), ts.label + 6.0),
                            Sense::hover(),
                        );
                        ui.painter().with_clip_rect(sr).text(
                            pos2(sr.left() + 2.0, sr.center().y),
                            Align2::LEFT_CENTER,
                            match error {
                                Some(e) => e.clone(),
                                None if names.is_empty() => "TAB COMPLETES DIRECTORY NAMES".into(),
                                None => names.join("  "),
                            },
                            mono(ts.label),
                            if error.is_some() {
                                pal.danger
                            } else {
                                pal.text_dim
                            },
                        );
                        ui.add_space(6.0);
                        let clicked = fuide::dialog::button_row(
                            ui,
                            &[
                                ("CANCEL", pal.text_dim, true),
                                ("OPEN REPOSITORY", pal.accent, true),
                            ],
                        );
                        (submit, complete, clicked)
                    });
                finished = resp.finished;
                if open {
                    let (submit, complete, clicked) = resp.inner.unwrap_or((false, false, None));
                    if resp.should_close || clicked == Some(0) {
                        actions.push(Action::CloseDialog);
                    } else if submit || clicked == Some(1) {
                        actions.push(Action::ConfirmOpen);
                    } else if complete {
                        actions.push(Action::CompleteOpen);
                    }
                }
            }
            DialogState::Branch { input, focus } => {
                let resp = Dialog::new("New branch")
                    .tag("at HEAD", pal.text_dim)
                    .width(460.0)
                    .show(ctx, open, |ui| {
                        ui.spacing_mut().item_spacing.y = 6.0;
                        let field =
                            widgets::text_input(ui, ui.available_width(), input, "branch name");
                        if *focus {
                            field.request_focus();
                            *focus = false;
                        }
                        let submit = field.lost_focus() && enter;
                        widgets::readout(
                            ui,
                            "command",
                            &format!("git switch -c {}", input.trim()),
                            None,
                        );
                        ui.add_space(6.0);
                        let clicked = fuide::dialog::button_row(
                            ui,
                            &[
                                ("CANCEL", pal.text_dim, true),
                                ("CREATE", pal.accent, !input.trim().is_empty()),
                            ],
                        );
                        (submit, clicked)
                    });
                finished = resp.finished;
                if open {
                    let (submit, clicked) = resp.inner.unwrap_or((false, None));
                    if resp.should_close || clicked == Some(0) {
                        actions.push(Action::CloseDialog);
                    } else if submit || clicked == Some(1) {
                        actions.push(Action::ConfirmBranch);
                    }
                }
            }
        }
        if finished {
            self.dialog = None;
        }
    }

    fn ui_log(&self, ui: &mut Ui, rect: Rect, open: bool) -> bool {
        let pal = palette(ui.ctx());
        let (_, toggled) = Panel::new("Git output")
            .tag(format!("{} lines", self.log.len()), pal.text_dim)
            .padding(8.0, 12.0)
            .show_collapsible_rect(ui, rect, open, |ui| {
                if !open {
                    return;
                }
                let lines: Vec<LogLine> = self
                    .log
                    .iter()
                    .map(|l| LogLine {
                        time: l.time.clone(),
                        text: l.text.clone(),
                        color: match l.level {
                            Level::Info => pal.text,
                            Level::Ok => pal.ok,
                            Level::Warn => pal.warn,
                            Level::Danger => pal.danger,
                        },
                    })
                    .collect();
                widgets::log_feed(
                    ui,
                    &lines,
                    pal.text_dim,
                    type_scale(ui.ctx()).label,
                    widgets::LogOrder::Chronological,
                );
            });
        toggled
    }
}

#[cfg(test)]
mod e2e;
#[cfg(test)]
mod tests;
