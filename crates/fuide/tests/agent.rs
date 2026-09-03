//! The agent interface end to end inside an `egui_kittest` harness (no socket): commands go in
//! through `Agent::submit`, `tick` injects the input, the kit's widgets react as if a person had
//! clicked / typed, and the reply carries the observation.

use std::sync::mpsc::Receiver;
use std::time::Duration;

use egui::{vec2, Vec2};
use egui_kittest::Harness;
use fuide::agent::{Command, Reply};
use fuide::{theme, widgets, Agent, Palette, Panel};

#[derive(Default)]
struct Demo {
    installed: bool,
    agent: Option<Agent>,
    clicks: u32,
    danger_clicks: u32,
    hidden: bool,
    filter: String,
    submitted: u32,
    enter_presses: u32,
    /// Reserve `PURGE` (and Enter) for the human, like an open confirmation dialog.
    guard: bool,
}

fn demo(ui: &mut egui::Ui, st: &mut Demo) {
    if !st.installed {
        theme::install(ui.ctx(), Palette::cyan(), vec![]);
        st.installed = true;
        return;
    }
    let agent = st.agent.get_or_insert_with(|| {
        let mut a = Agent::new("kittest", "Kittest Demo");
        a.set_tools(vec![fuide::agent::ToolSpec {
            name: "bump".into(),
            description: "adds `by` clicks".into(),
            schema: serde_json::json!({ "type": "object" }),
        }]);
        a.enable_without_server();
        a
    });
    agent.set_blocked(if st.guard {
        vec!["PURGE".into()]
    } else {
        Vec::new()
    });
    let state = agent
        .wants_state()
        .then(|| format!("clicks: {} :: filter: {:?}", st.clicks, st.filter));
    agent.tick(ui.ctx(), state);
    if let Some(call) = agent.take_tool() {
        let by = call.args.get("by").and_then(|v| v.as_u64()).unwrap_or(1) as u32;
        st.clicks += by;
        agent.finish_tool(Ok(format!("bumped by {by}")), Some("GO"));
    }
    if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
        st.enter_presses += 1;
    }
    let rect = egui::Rect::from_min_size(egui::pos2(20.0, 20.0), vec2(500.0, 200.0));
    Panel::new("Demo").show_rect(ui, rect, |ui| {
        ui.horizontal(|ui| {
            if widgets::button(ui, vec2(90.0, 24.0), "GO", true).clicked() {
                st.clicks += 1;
            }
            if widgets::button(ui, vec2(90.0, 24.0), "PURGE", true).clicked() {
                st.danger_clicks += 1;
            }
            widgets::button(ui, vec2(90.0, 24.0), "LOCKED", false);
            widgets::toggle_chip(ui, "hidden", &mut st.hidden);
        });
        let resp = widgets::text_input(ui, 200.0, &mut st.filter, "filter");
        if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            st.submitted += 1;
        }
    });
    if let Some(a) = &st.agent {
        a.paint(ui.ctx());
    }
}

fn harness() -> Harness<'static, Demo> {
    let mut h = Harness::builder()
        .with_size(Vec2::new(640.0, 300.0))
        .with_step_dt(1.0 / 60.0)
        .build_ui_state(
            demo,
            Demo {
                guard: true,
                ..Demo::default()
            },
        );
    h.run_steps(3);
    h
}

/// Step frames until the reply arrives (the cursor glide and settle take a few dozen).
fn drive(h: &mut Harness<'static, Demo>, rx: Receiver<Reply>) -> Reply {
    for _ in 0..400 {
        h.run_steps(1);
        if let Ok(r) = rx.recv_timeout(Duration::from_millis(1)) {
            return r;
        }
    }
    panic!("no reply from the agent")
}

fn submit(h: &Harness<'static, Demo>, cmd: Command) -> Receiver<Reply> {
    h.state().agent.as_ref().unwrap().submit(cmd)
}

fn text(r: Reply) -> String {
    match r {
        Reply::Text(t) => t,
        other => panic!("expected text, got {other:?}"),
    }
}

#[test]
fn observe_lists_widgets_with_roles_and_state() {
    let mut h = harness();
    let rx = submit(&h, Command::Observe);
    let t = text(drive(&mut h, rx));
    assert!(t.contains("clicks: 0"), "{t}");
    assert!(t.contains("[button] GO @"), "{t}");
    assert!(t.contains("[button] LOCKED (disabled)"), "{t}");
    assert!(t.contains("[toggle] HIDDEN @"), "{t}");
    assert!(t.contains("[input] FILTER = \"\""), "{t}");
    assert!(t.contains("[button] PURGE (human only)"), "{t}");
}

#[test]
fn click_by_label_reaches_the_widget() {
    let mut h = harness();
    let rx = submit(
        &h,
        Command::Click {
            label: "go".into(),
            nth: 1,
        },
    );
    let t = text(drive(&mut h, rx));
    assert_eq!(h.state().clicks, 1);
    assert!(t.contains("clicks: 1"), "observation after the click: {t}");
    assert_eq!(
        h.state().agent.as_ref().unwrap().last_action(),
        Some("CLICK ▸ GO")
    );

    let rx = submit(
        &h,
        Command::Click {
            label: "HIDDEN".into(),
            nth: 1,
        },
    );
    let t = text(drive(&mut h, rx));
    assert!(h.state().hidden);
    assert!(t.contains("[toggle] HIDDEN (on)"), "{t}");
}

#[test]
fn disabled_blocked_and_unknown_targets_are_refused() {
    let mut h = harness();
    for (label, needle) in [
        ("LOCKED", "disabled"),
        ("PURGE", "reserved for the human"),
        ("NOPE", "no widget labelled"),
    ] {
        let rx = submit(
            &h,
            Command::Click {
                label: label.into(),
                nth: 1,
            },
        );
        match drive(&mut h, rx) {
            Reply::Error(e) => assert!(e.contains(needle), "{label}: {e}"),
            other => panic!("{label}: expected an error, got {other:?}"),
        }
    }
    assert_eq!(h.state().danger_clicks, 0);
    // Enter is refused too while something is reserved
    let rx = submit(
        &h,
        Command::Key {
            combo: "enter".into(),
            repeat: 1,
        },
    );
    assert!(matches!(drive(&mut h, rx), Reply::Error(_)));
    assert_eq!(h.state().enter_presses, 0);
}

#[test]
fn type_focuses_the_input_and_submits() {
    let mut h = harness();
    h.state_mut().guard = false;
    let rx = submit(
        &h,
        Command::Type {
            text: "rip".into(),
            label: Some("filter".into()),
            submit: false,
        },
    );
    let t = text(drive(&mut h, rx));
    assert_eq!(h.state().filter, "rip");
    assert!(t.contains("[input] FILTER = \"rip\""), "{t}");
    // no label: the focused input keeps receiving text; submit presses Enter
    let rx = submit(
        &h,
        Command::Type {
            text: "grep".into(),
            label: None,
            submit: true,
        },
    );
    text(drive(&mut h, rx));
    assert_eq!(h.state().filter, "ripgrep");
    assert!(h.state().enter_presses >= 1);
}

#[test]
fn keys_and_wait_report_back() {
    let mut h = harness();
    let rx = submit(
        &h,
        Command::Key {
            combo: "down".into(),
            repeat: 3,
        },
    );
    text(drive(&mut h, rx));
    let rx = submit(&h, Command::Wait { ms: 50 });
    let t = text(drive(&mut h, rx));
    assert!(t.contains("WIDGETS ("), "{t}");
}

#[test]
fn screenshot_comes_back_as_scaled_png() {
    // kittest answers `ViewportCommand::Screenshot` with its wgpu render
    let mut h = harness();
    let dir = std::env::temp_dir().join(format!("fuide-agent-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("shot.png");
    let rx = submit(
        &h,
        Command::Screenshot {
            scale: 0.5,
            path: Some(path.to_string_lossy().into_owned()),
        },
    );
    match drive(&mut h, rx) {
        Reply::Image { png, text } => {
            assert!(png.starts_with(b"\x89PNG"));
            assert!(text.contains("320x150"), "{text}");
            assert!(text.contains("saved to"), "{text}");
            assert_eq!(std::fs::read(&path).unwrap(), png);
        }
        other => panic!("expected an image, got {other:?}"),
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn app_tools_run_in_the_app_and_point_at_their_widget() {
    let mut h = harness();
    let rx = submit(
        &h,
        Command::Tool {
            name: "bump".into(),
            args: serde_json::json!({ "by": 3 }),
        },
    );
    let t = text(drive(&mut h, rx));
    assert!(t.starts_with("bumped by 3\n\n"), "{t}");
    assert!(
        t.contains("clicks: 3"),
        "the observation follows the tool text: {t}"
    );
    assert_eq!(
        h.state().agent.as_ref().unwrap().last_action(),
        Some("TOOL ▸ BUMP")
    );
    // no click was injected: the tool did the work itself
    assert_eq!(h.state().clicks, 3);
    let rx = submit(
        &h,
        Command::Tool {
            name: "nope".into(),
            args: serde_json::json!({}),
        },
    );
    assert!(matches!(drive(&mut h, rx), Reply::Error(e) if e.contains("unknown tool")));
}
