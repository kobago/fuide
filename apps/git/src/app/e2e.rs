//! End-to-end: the real `GitApp` driven through the accessibility tree with `egui_kittest`
//! against a `git init`ed temporary repository. See the file manager's `e2e.rs` for the
//! harness conventions.

use std::path::Path;
use std::time::{Duration, Instant};

use egui::{Key, Modifiers, Vec2};
use egui_kittest::kittest::{NodeT, Queryable};
use egui_kittest::Harness;

use super::tests::temp_repo;
use super::*;

fn harness(repo: &Path) -> Harness<'static, GitApp> {
    let repo = repo.to_path_buf();
    let mut h = Harness::builder()
        .with_size(Vec2::new(1380.0, 860.0))
        .with_step_dt(1.0 / 60.0)
        .build_eframe(move |cc| {
            let mut app = GitApp::with_context(&cc.egui_ctx, Settings::default());
            app.apply(&cc.egui_ctx, Action::Open(repo), 0.0);
            app
        });
    pump(&mut h);
    h
}

/// Run frames until no git worker is in flight.
fn pump(h: &mut Harness<'static, GitApp>) {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        h.run_steps(1);
        if !h.state().git.busy() {
            break;
        }
        assert!(Instant::now() < deadline, "git worker did not finish");
        std::thread::sleep(Duration::from_millis(5));
    }
    h.run_steps(1);
}

fn log_has(h: &Harness<'static, GitApp>, needle: &str) -> bool {
    h.state().log.iter().any(|e| e.text.contains(needle))
}

#[test]
fn stage_all_unstage_all_and_commit_by_button() {
    let repo = temp_repo();
    let mut h = harness(&repo);
    h.get_by_label("CHANGES  2");
    h.get_by_label("STAGE ALL").click();
    pump(&mut h);
    assert!(log_has(&h, "$ git add -A"));
    assert_eq!(h.state().staged.len(), 2);

    h.get_by_label("UNSTAGE ALL").click();
    pump(&mut h);
    assert_eq!(h.state().staged.len(), 0);

    // stage one file by double-clicking its row, then commit with Cmd+Enter
    h.get_by_label("STAGE ALL").click();
    pump(&mut h);
    h.state_mut().commit_msg = "e2e: commit".into();
    h.run_steps(1);
    h.key_press_modifiers(Modifiers::COMMAND, Key::Enter);
    pump(&mut h);
    assert!(log_has(&h, "$ git commit -F -"));
    assert_eq!(h.state().commits.len(), 2);
    assert_eq!(h.state().commits[0].subject, "e2e: commit");

    // the SUCCESS card is acknowledged by its button
    let deadline = Instant::now() + Duration::from_secs(5);
    while h
        .query_by_label("ACKNOWLEDGE")
        .is_none_or(|n| n.accesskit_node().is_disabled())
    {
        assert!(Instant::now() < deadline, "no live success card");
        h.run_steps(1);
    }
    h.get_by_label("ACKNOWLEDGE").click();
    h.run_steps(15);
    assert!(h.state().dialog.is_none());
}

#[test]
fn history_view_shows_the_commit_and_its_files() {
    let repo = temp_repo();
    let mut h = harness(&repo);
    h.key_press_modifiers(Modifiers::COMMAND, Key::Num2);
    h.run_steps(2);
    assert_eq!(h.state().view, View::History);
    h.key_press(Key::ArrowDown);
    pump(&mut h);
    assert_eq!(h.state().selected_commit().unwrap().subject, "initial");
    assert_eq!(h.state().detail.as_ref().unwrap().files.len(), 1);
    h.get_by_label("COPY HASH");
    assert!(matches!(&h.state().diff, Some((DiffKey::Commit(_, p), _)) if p == "a.txt"));
}

#[test]
fn open_dialog_takes_a_typed_path() {
    let repo = temp_repo();
    let other = temp_repo();
    let mut h = harness(&repo);
    h.key_press_modifiers(Modifiers::COMMAND, Key::O);
    h.run_steps(3);
    assert!(
        matches!(
            &h.state().dialog,
            Some(OpenDialog {
                state: DialogState::Open { .. },
                ..
            })
        ),
        "Cmd+O opens the dialog"
    );
    h.run_steps(12); // fade-in
    h.get_by_label("REPOSITORY PATH");
    h.state_mut().dialog = Some(OpenDialog {
        state: DialogState::Open {
            input: other.display().to_string(),
            error: None,
            focus: true,
            suggestions: Vec::new(),
            suggested_for: String::new(),
        },
        closing: false,
    });
    h.run_steps(2);
    h.get_by_label("OPEN REPOSITORY").click();
    pump(&mut h);
    assert_eq!(h.state().repo.as_deref(), Some(other.as_path()));
    h.run_steps(15);
    assert!(h.state().dialog.is_none());
}
