//! UI tests for the fuide kit, driven through `egui_kittest`:
//! - accessibility: every interactive part reports a label / state, so tests (and screen
//!   readers) can find it by name instead of by pixel position
//! - snapshots: the shell + panel + widgets rendered headless with wgpu and compared to
//!   `tests/snapshots/*.png` (`UPDATE_SNAPSHOTS=true cargo test -p fuide` to regenerate)
//!
//! The shell animates (pulse, scan band) and asks for a repaint every frame, so tests step a
//! fixed number of frames with a fixed `step_dt` instead of `run()` (which waits for quiescence).
//! The harness runs its first frame while it is being built, so fonts cannot be installed
//! "before the first frame" the way `App::new` does. `set_fonts` only takes effect on the
//! next pass (drawing the display face in the same pass panics with "FontFamily::Name(..) is
//! not bound to any fonts"), so the first frame installs and draws nothing.

use egui::accesskit::Toggled;
use egui::{pos2, vec2, Key, Modifiers, Rect, Vec2};
use egui_kittest::kittest::{NodeT, Queryable};

use egui_kittest::Harness;
use fuide::{theme, widgets, Palette, Panel, Shell};

#[derive(Default)]
struct Demo {
    installed: bool,
    hidden: bool,
    tab: usize,
    clicks: u32,
    settings_clicks: u32,
    close_requests: u32,
    /// Show an `ERROR` alert card over everything (input under it is blocked).
    alert: bool,
}

/// A representative screen: shell with lamps and the gear, one panel with a nav tab group,
/// a toggle chip, buttons and readouts.
fn demo(ui: &mut egui::Ui, st: &mut Demo) {
    if !st.installed {
        theme::install(ui.ctx(), Palette::cyan(), vec![]);
        st.installed = true;
        return;
    }
    let pal = theme::palette(ui.ctx());
    let out = Shell::new("Snapshot")
        .subtitle("v0.1 :: test")
        .status_left("T+00:00:01.0 :: 3 ITEMS")
        .lamp("LINK OK", pal.ok, false)
        .lamp("BUSY", pal.warn, false)
        .settings_button(true)
        .show_full(ui, |ui| {
            let c = ui.max_rect();
            let rect = Rect::from_min_max(pos2(c.left(), c.top() + 10.0), c.max);
            Panel::new("Modules")
                .tag("3", pal.text_dim)
                .show_rect(ui, rect, |ui| {
                    ui.spacing_mut().item_spacing.y = 3.0;
                    widgets::section_label(ui, "Places");
                    for (i, name) in ["Home", "Desktop", "Downloads"].iter().enumerate() {
                        if widgets::nav_tab(ui, name, st.tab == i).clicked() {
                            st.tab = i;
                        }
                    }
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        if widgets::button(ui, vec2(80.0, 24.0), "OPEN", true).clicked() {
                            st.clicks += 1;
                        }
                        widgets::button_colored(ui, vec2(80.0, 24.0), "DELETE", false, pal.danger);
                        widgets::icon_button(ui, vec2(28.0, 24.0), widgets::Icon::Refresh, true);
                        widgets::toggle_chip(ui, "hidden", &mut st.hidden);
                    });
                    ui.add_space(6.0);
                    widgets::readout(ui, "kind", "DIR", None);
                    widgets::readout(ui, "bytes", "261.1 KB", Some(pal.accent));
                    widgets::rule(ui);
                    widgets::arc_gauge(ui, 28.0, 0.72, "USED", pal.accent);
                });
        });
    if out.settings_clicked {
        st.settings_clicks += 1;
    }
    if out.close_requested {
        st.close_requests += 1;
    }
    if st.alert {
        let resp = fuide::dialog::alert(
            ui.ctx(),
            true,
            "ERROR",
            "probe // x",
            "details :: log",
            pal.danger,
        );
        if resp.inner == Some(true) {
            st.alert = false;
        }
    }
}

fn harness<'a>() -> Harness<'a, Demo> {
    Harness::builder()
        .with_size(Vec2::new(520.0, 420.0))
        .with_step_dt(1.0 / 60.0)
        .build_ui_state(demo, Demo::default())
}

#[test]
fn interactive_parts_are_addressable_by_label() {
    let mut h = harness();
    h.run_steps(2);

    // shell buttons
    h.get_by_label("CLOSE WINDOW");
    h.get_by_label("MAXIMIZE");
    h.get_by_label("MINIMIZE");
    h.get_by_label("SETTINGS").click();
    h.run_steps(2);
    assert_eq!(h.state().settings_clicks, 1);

    // nav tabs carry their selection state (egui reports `WidgetInfo::selected` as `toggled`)
    assert_eq!(
        h.get_by_label("HOME").accesskit_node().toggled(),
        Some(Toggled::True)
    );
    assert_eq!(
        h.get_by_label("DESKTOP").accesskit_node().toggled(),
        Some(Toggled::False)
    );
    h.get_by_label("DESKTOP").click();
    h.run_steps(2);
    assert_eq!(h.state().tab, 1);
    assert_eq!(
        h.get_by_label("DESKTOP").accesskit_node().toggled(),
        Some(Toggled::True)
    );

    // buttons: enabled one clicks, disabled one is reported disabled
    h.get_by_label("OPEN").click();
    h.run_steps(2);
    assert_eq!(h.state().clicks, 1);
    assert!(h.get_by_label("DELETE").accesskit_node().is_disabled());
    h.get_by_label("REFRESH");

    // toggle chip is a checkbox
    assert!(!h.state().hidden);
    h.get_by_label("HIDDEN").click();
    h.run_steps(2);
    assert!(h.state().hidden);
}

#[test]
fn cmd_w_and_the_close_button_ask_the_window_to_close_even_under_a_modal() {
    // `ViewportCommand::Close` itself is not observable here: kittest runs one pass per queued
    // event and keeps only the last pass' output, so the shell reports the request instead.
    let mut h = harness();
    h.run_steps(2);
    assert_eq!(h.state().close_requests, 0);

    h.key_press_modifiers(Modifiers::COMMAND, Key::W);
    h.run_steps(1);
    assert_eq!(h.state().close_requests, 1);

    h.get_by_label("CLOSE WINDOW").click();
    h.run_steps(2);
    assert_eq!(h.state().close_requests, 2);

    // a modal alert blocks pointer input underneath, but Cmd+W still gets through
    h.state_mut().alert = true;
    h.run_steps(12);
    h.get_by_label("ACKNOWLEDGE");
    h.key_press_modifiers(Modifiers::COMMAND, Key::W);
    h.run_steps(1);
    assert_eq!(h.state().close_requests, 3);
}

#[test]
fn shell_snapshot_cyan() {
    let mut h = harness();
    h.run_steps(3);
    h.snapshot("shell_cyan");
}
