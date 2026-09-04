//! State-machine tests against a real repository: `git init` in a temporary directory, so the
//! worker threads and the streaming runner run end to end without any network.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Once;
use std::time::{Duration, Instant};

use super::*;

/// Keep the user's global git config (signing, hooks, aliases) out of the tests.
pub(super) fn isolate_git() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        // SAFETY: set once before any worker thread is spawned; every test sets the same values.
        unsafe {
            std::env::set_var("GIT_CONFIG_GLOBAL", "/dev/null");
            std::env::set_var("GIT_CONFIG_NOSYSTEM", "1");
        }
    });
}

pub(super) fn sh(repo: &Path, args: &[&str]) -> String {
    let o = Command::new("git")
        .current_dir(repo)
        .args(["-c", "user.name=test", "-c", "user.email=test@example.com"])
        .args(args)
        .output()
        .expect("git");
    assert!(
        o.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    String::from_utf8_lossy(&o.stdout).into_owned()
}

/// A fresh repository with one commit (`a.txt` = three lines), then `a.txt` modified in the
/// worktree and an untracked `new.txt`.
pub(super) fn temp_repo() -> PathBuf {
    isolate_git();
    static N: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "fuide-git-test-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    sh(&dir, &["init", "-q", "-b", "main"]);
    sh(&dir, &["config", "user.name", "test"]);
    sh(&dir, &["config", "user.email", "test@example.com"]);
    sh(&dir, &["config", "commit.gpgsign", "false"]);
    std::fs::write(dir.join("a.txt"), "one\ntwo\nthree\n").unwrap();
    sh(&dir, &["add", "a.txt"]);
    sh(&dir, &["commit", "-q", "-m", "initial"]);
    std::fs::write(dir.join("a.txt"), "one\nTWO\nthree\n").unwrap();
    std::fs::write(dir.join("new.txt"), "hello\n").unwrap();
    // macOS: /var → /private/var; the app stores git's canonical toplevel
    dir.canonicalize().unwrap()
}

fn app(repo: &Path) -> (egui::Context, GitApp) {
    let ctx = egui::Context::default();
    let mut app = GitApp::with_context(&ctx, Settings::default());
    app.apply(&ctx, Action::Open(repo.to_path_buf()), 0.0);
    pump(&ctx, &mut app);
    (ctx, app)
}

/// Pump git messages until nothing is in flight (a finished command re-reads the repository;
/// a new status re-requests the diff — both are waited for).
pub(super) fn pump(ctx: &egui::Context, app: &mut GitApp) {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        app.poll(ctx, 0.0);
        app.sync_diff(ctx);
        if !app.git.busy() {
            break;
        }
        assert!(Instant::now() < deadline, "git worker did not finish");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn paths(app: &GitApp, list: &[usize]) -> Vec<String> {
    list.iter()
        .map(|&i| app.status.changes[i].path.clone())
        .collect()
}

#[test]
fn open_reads_status_log_and_refs() {
    let repo = temp_repo();
    let (_ctx, app) = app(&repo);
    assert_eq!(app.repo.as_deref(), Some(repo.as_path()));
    assert_eq!(app.status.branch, "main");
    assert_eq!(paths(&app, &app.unstaged), ["a.txt", "new.txt"]);
    assert!(app.staged.is_empty());
    assert_eq!(app.commits.len(), 1);
    assert_eq!(app.commits[0].subject, "initial");
    assert!(app.refs.iter().any(|r| r.name == "main" && r.current));
    assert!(app.last_error.is_none());
}

#[test]
fn open_rejects_a_directory_that_is_not_a_repository() {
    isolate_git();
    let ctx = egui::Context::default();
    let mut app = GitApp::with_context(&ctx, Settings::default());
    app.apply(&ctx, Action::Open(std::env::temp_dir()), 0.0);
    assert!(app.repo.is_none());
    assert!(matches!(app.notice_queue.front(), Some((false, _))));
}

#[test]
fn stage_unstage_and_commit() {
    let repo = temp_repo();
    let (ctx, mut app) = app(&repo);
    app.apply(&ctx, Action::Select(Focus::Unstaged, Some(0)), 0.0);
    pump(&ctx, &mut app);
    let (key, d) = app.diff.as_ref().expect("diff of a.txt");
    assert_eq!(*key, DiffKey::Unstaged("a.txt".into()));
    assert_eq!((d.adds, d.dels), (1, 1));

    app.apply(&ctx, Action::ToggleSelected, 0.0);
    pump(&ctx, &mut app);
    assert_eq!(paths(&app, &app.staged), ["a.txt"]);
    assert_eq!(paths(&app, &app.unstaged), ["new.txt"]);

    app.apply(&ctx, Action::Select(Focus::Staged, Some(0)), 0.0);
    app.apply(&ctx, Action::ToggleSelected, 0.0);
    pump(&ctx, &mut app);
    assert!(app.staged.is_empty());

    app.apply(&ctx, Action::StageAll, 0.0);
    pump(&ctx, &mut app);
    assert_eq!(paths(&app, &app.staged), ["a.txt", "new.txt"]);

    app.commit_msg = "feat: second\n\nbody".into();
    app.apply(&ctx, Action::Commit, 0.0);
    pump(&ctx, &mut app);
    assert_eq!(app.commits.len(), 2, "log re-read after the commit");
    assert_eq!(app.commits[0].subject, "feat: second");
    assert!(app.commit_msg.is_empty(), "message cleared on success");
    assert!(app.staged.is_empty() && app.unstaged.is_empty());
    assert!(matches!(app.notice_queue.front(), Some((true, _))));
    assert!(sh(&repo, &["log", "-1", "--format=%B"]).contains("body"));
}

#[test]
fn a_single_hunk_is_staged_with_git_apply() {
    let repo = temp_repo();
    let (ctx, mut app) = app(&repo);
    app.apply(&ctx, Action::Select(Focus::Unstaged, Some(0)), 0.0);
    pump(&ctx, &mut app);
    app.apply(&ctx, Action::StageHunk(0), 0.0);
    pump(&ctx, &mut app);
    assert_eq!(paths(&app, &app.staged), ["a.txt"]);
    assert_eq!(paths(&app, &app.unstaged), ["new.txt"]);
    // and back
    app.apply(&ctx, Action::Select(Focus::Staged, Some(0)), 0.0);
    pump(&ctx, &mut app);
    assert!(matches!(&app.diff, Some((DiffKey::Staged(p), _)) if p == "a.txt"));
    app.apply(&ctx, Action::UnstageHunk(0), 0.0);
    pump(&ctx, &mut app);
    assert!(app.staged.is_empty());
}

#[test]
fn discard_asks_first_and_the_agent_may_not_confirm() {
    let repo = temp_repo();
    let (ctx, mut app) = app(&repo);
    app.apply(&ctx, Action::Discard("new.txt".into(), true), 0.0);
    assert!(matches!(
        &app.dialog,
        Some(OpenDialog {
            state: DialogState::Confirm(c),
            ..
        }) if c.danger && c.args[0] == "clean"
    ));
    assert_eq!(app.agent_blocked(), ["DISCARD"]);
    app.apply(&ctx, Action::ConfirmDialog, 0.0);
    pump(&ctx, &mut app);
    assert!(!repo.join("new.txt").exists());
    assert_eq!(paths(&app, &app.unstaged), ["a.txt"]);
}

#[test]
fn history_selection_loads_the_commit_detail_and_its_diff() {
    let repo = temp_repo();
    let (ctx, mut app) = app(&repo);
    app.apply(&ctx, Action::SetView(View::History), 0.0);
    app.apply(&ctx, Action::Select(Focus::History, Some(0)), 0.0);
    pump(&ctx, &mut app);
    let d = app.detail.as_ref().expect("detail");
    assert_eq!(d.author, "test");
    assert_eq!(d.files, vec![('A', "a.txt".to_string())]);
    let (key, diff) = app.diff.as_ref().expect("diff of the first file");
    assert!(matches!(key, DiffKey::Commit(_, p) if p == "a.txt"));
    assert_eq!(diff.adds, 3);
}

#[test]
fn new_branch_and_switch() {
    let repo = temp_repo();
    let (ctx, mut app) = app(&repo);
    app.apply(&ctx, Action::BranchDialog, 0.0);
    if let Some(OpenDialog {
        state: DialogState::Branch { input, .. },
        ..
    }) = &mut app.dialog
    {
        *input = "feat/x".into();
    }
    app.apply(&ctx, Action::ConfirmBranch, 0.0);
    pump(&ctx, &mut app);
    assert_eq!(app.status.branch, "feat/x");
    app.apply(&ctx, Action::Switch("main".into()), 0.0);
    pump(&ctx, &mut app);
    assert_eq!(app.status.branch, "main");
    assert!(app.refs.iter().any(|r| r.name == "feat/x" && !r.current));
}
