//! Agent interface: lets an AI agent (Claude Code, or any MCP client) observe and drive the app
//! while a human watches.
//!
//! - **Observation**: every interactive part registers itself each frame ([`describe`] /
//!   [`note`] — label, role, state, rect — next to the accesskit label it already reports), and
//!   the app adds a one-paragraph state summary. `observe` returns both as text.
//! - **Action**: clicks are injected as `Event::AccessKitActionRequest(Click)`, which egui
//!   treats as a real click on that widget; keys and text as `Event::Key` / `Event::Text`.
//!   A synthetic cursor glides to the target first and the target flashes, so the operation is
//!   legible on screen. Screenshots use the same self-capture path as [`crate::devshot`].
//! - **Transport**: a Unix socket server inside the app ([`server`]) speaking MCP over
//!   newline-delimited JSON-RPC, switched on/off in the settings window; the same binary's
//!   `--mcp` mode ([`bridge`]) is the stdio process an MCP client spawns.
//!
//! ```ignore
//! // app struct
//! agent: fuide::Agent,
//! // start-up
//! agent: Agent::new("brew", "FUIDE Brew"),  then  app.agent.set_enabled(&ctx, settings.agent);
//! // every frame, first thing in `ui()`
//! self.agent.set_blocked(labels_the_human_must_press);
//! let state = self.agent.wants_state().then(|| self.agent_state());
//! self.agent.tick(ui.ctx(), state);
//! // after the shell
//! self.agent.paint(ui.ctx());
//! ```

pub mod bridge;
pub mod encode;
pub mod mcp;
pub mod server;

use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;

use egui::accesskit::{Action, ActionRequest, TreeId};
use egui::{
    Align2, Context, Event, Id, Key, LayerId, Modifiers, Order, Pos2, Rect, Response, Stroke,
    UserData, ViewportCommand, ViewportId, WidgetInfo, WidgetType,
};

use crate::{geom, theme};

// ------------------------------------------------------------------------------- registry

/// One interactive widget as seen by the agent.
#[derive(Clone, Debug, PartialEq)]
pub struct Widget {
    pub id: Id,
    pub role: &'static str,
    pub label: String,
    pub enabled: bool,
    pub selected: Option<bool>,
    /// Current text of an input.
    pub value: Option<String>,
    pub rect: Rect,
    pub order: Order,
}

#[derive(Clone, Default)]
struct Registry {
    widgets: Vec<Widget>,
}

fn registry_id() -> Id {
    Id::new("fuide-agent-registry")
}
fn active_id() -> Id {
    Id::new("fuide-agent-active")
}

fn role_name(typ: WidgetType) -> &'static str {
    match typ {
        WidgetType::Button => "button",
        WidgetType::SelectableLabel => "item",
        WidgetType::Checkbox => "toggle",
        WidgetType::TextEdit => "input",
        WidgetType::Slider | WidgetType::DragValue => "slider",
        WidgetType::RadioButton => "radio",
        WidgetType::ComboBox => "combo",
        WidgetType::Link => "link",
        WidgetType::Label => "label",
        _ => "other",
    }
}

/// Report a widget to assistive technology (`Response::widget_info`) **and** to the agent.
/// Use this wherever the kit or an app would call `widget_info`.
pub fn describe(resp: &Response, info: impl FnOnce() -> WidgetInfo) {
    let info = info();
    note(resp, &info);
    resp.widget_info(|| info.clone());
}

/// Register a widget with the agent only (for widgets that already report their own
/// accesskit node, e.g. an `egui::TextEdit`). Cheap when the agent is off.
pub fn note(resp: &Response, info: &WidgetInfo) {
    let ctx = &resp.ctx;
    if ctx.viewport_id() != ViewportId::ROOT
        || !ctx.data(|d| d.get_temp::<bool>(active_id()).unwrap_or(false))
    {
        return;
    }
    let w = Widget {
        id: resp.id,
        role: role_name(info.typ),
        label: info.label.clone().unwrap_or_default(),
        enabled: info.enabled,
        selected: info.selected,
        value: info.current_text_value.clone(),
        rect: resp.rect,
        order: resp.layer_id.order,
    };
    ctx.data_mut(|d| {
        d.get_temp_mut_or_default::<Registry>(registry_id())
            .widgets
            .push(w);
    });
}

// ------------------------------------------------------------------------------- protocol

/// An app-defined MCP tool: listed by `tools/list` next to the generic ones, delivered to the
/// app through [`Agent::take_tool`] and answered with [`Agent::finish_tool`].
#[derive(Clone, Debug, PartialEq)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    /// JSON schema of the arguments (`inputSchema`).
    pub schema: serde_json::Value,
}

/// A call of an app-defined tool, taken by the app from [`Agent::take_tool`].
#[derive(Clone, Debug, PartialEq)]
pub struct ToolCall {
    pub name: String,
    pub args: serde_json::Value,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    Observe,
    /// An app-defined tool (see [`ToolSpec`]).
    Tool {
        name: String,
        args: serde_json::Value,
    },
    Screenshot {
        scale: f32,
        path: Option<String>,
    },
    Click {
        label: String,
        nth: usize,
    },
    Type {
        text: String,
        label: Option<String>,
        submit: bool,
    },
    Key {
        combo: String,
        repeat: usize,
    },
    Wait {
        ms: u64,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum Reply {
    Text(String),
    Image { png: Vec<u8>, text: String },
    Error(String),
}

/// A tool call in flight: the command and where to send its answer.
pub struct Request {
    pub cmd: Command,
    pub reply: Sender<Reply>,
}

/// The `initialize` instructions an MCP client shows its model.
pub fn instructions(app_name: &str) -> String {
    format!(
        "Drives the {app_name} window (a FUI desktop app) while a person watches: call `observe` first, \
         then `click` / `type` / `key` using the exact labels it lists, and read the observation each \
         action returns. Some confirmations are reserved for the human; the observation says so. \
         Tools beyond `observe` / `screenshot` / `click` / `type` / `key` / `wait` are the app's own: \
         prefer them for structured edits — they act directly and return the observation afterwards."
    )
}

// ------------------------------------------------------------------------------- agent

/// Frames to let the app react after an action before reporting back.
const SETTLE_FRAMES: u32 = 8;
/// Frames to wait for a widget an app tool created (it registers on its first draw).
const POINT_FRAMES: u32 = 4;
/// Typewriter delay between characters, seconds.
const CHAR_DELAY: f64 = 0.035;
/// Cursor glide speed, logical px per second, and its duration bounds.
const CURSOR_SPEED: f32 = 1400.0;
const MOVE_MIN: f64 = 0.16;
const MOVE_MAX: f64 = 0.55;
/// The cursor stays fully visible this long after the last action, then fades for a second.
const CURSOR_HOLD: f64 = 2.5;
const FLASH_SECS: f64 = 0.6;

/// A marker on the screenshot request so the reply is ours (see `devshot`).
struct ShotTag;

enum Next {
    Click,
    Focus {
        chars: VecDeque<char>,
        submit: bool,
    },
    /// Arrive and flash only (an app tool already did the work).
    Flash,
}

enum Phase {
    Idle,
    Move {
        req: Request,
        target: Widget,
        from: Pos2,
        t0: f64,
        t1: f64,
        next: Next,
    },
    Type {
        req: Request,
        chars: VecDeque<char>,
        next_at: f64,
        submit: bool,
    },
    Keys {
        req: Request,
        presses: VecDeque<(Key, Modifiers)>,
        release: Option<(Key, Modifiers)>,
    },
    Settle {
        req: Request,
        frames: u32,
    },
    Shot {
        req: Request,
        scale: f32,
        path: Option<String>,
        since: f64,
    },
    Wait {
        req: Request,
        until: f64,
    },
    /// An app tool: `call` is `Some` until the app takes it, then the phase waits for
    /// [`Agent::finish_tool`].
    Tool {
        req: Request,
        call: Option<ToolCall>,
    },
    /// After an app tool: find the widget it touched (it may only appear next frame) and fly
    /// the cursor there.
    Point {
        req: Request,
        label: String,
        frames: u32,
    },
}

pub struct Agent {
    app_id: String,
    app_name: String,
    tx: Sender<Request>,
    rx: Receiver<Request>,
    /// Requests queued but not yet taken by `tick` (the app computes its state summary when > 0).
    pending: Arc<AtomicUsize>,
    server: Option<server::Server>,
    server_error: Option<String>,
    /// Registry active without a socket (tests drive `submit` directly).
    detached: bool,
    phase: Phase,
    /// Widgets registered during the last complete frame.
    widgets: Vec<Widget>,
    blocked: Vec<String>,
    cursor: Option<Pos2>,
    cursor_seen: f64,
    flash: Option<(Rect, f64)>,
    last_action: Option<String>,
    tools: Vec<ToolSpec>,
    /// Text an app tool produced, sent ahead of the observation once the app has settled.
    tool_text: Option<String>,
}

impl Agent {
    /// `app_id` names the socket (next to the settings file); `app_name` is the MCP server
    /// name and what `--mcp` launches.
    pub fn new(app_id: &str, app_name: &str) -> Self {
        let (tx, rx) = channel();
        Self {
            app_id: app_id.to_string(),
            app_name: app_name.to_string(),
            tx,
            rx,
            pending: Arc::new(AtomicUsize::new(0)),
            server: None,
            server_error: None,
            detached: false,
            phase: Phase::Idle,
            widgets: Vec::new(),
            blocked: Vec::new(),
            cursor: None,
            cursor_seen: 0.0,
            flash: None,
            last_action: None,
            tools: Vec::new(),
            tool_text: None,
        }
    }

    /// The app's own MCP tools. Set before enabling the server (they go into `tools/list`).
    pub fn set_tools(&mut self, tools: Vec<ToolSpec>) {
        self.tools = tools;
    }

    pub fn tools(&self) -> &[ToolSpec] {
        &self.tools
    }

    /// The app-defined tool call waiting for the app, if any. Handle it and call
    /// [`Agent::finish_tool`] in the same frame (after `tick`).
    pub fn take_tool(&mut self) -> Option<ToolCall> {
        match &mut self.phase {
            Phase::Tool { call, .. } => call.take(),
            _ => None,
        }
    }

    /// Answer the tool call from [`Agent::take_tool`]: `Ok(text)` is returned ahead of the
    /// observation after the app has settled; `Err` is returned at once. `focus` names the
    /// widget the tool touched (a list row, an input): the cursor flies there and flashes it,
    /// so the person watching sees where the change landed. Nothing is clicked.
    pub fn finish_tool(&mut self, result: Result<String, String>, focus: Option<&str>) {
        let phase = std::mem::replace(&mut self.phase, Phase::Idle);
        let Phase::Tool { req, .. } = phase else {
            self.phase = phase;
            return;
        };
        match result {
            Ok(text) => {
                self.tool_text = Some(text);
                self.phase = match focus {
                    Some(label) => Phase::Point {
                        req,
                        label: label.to_string(),
                        frames: POINT_FRAMES,
                    },
                    None => Phase::Settle {
                        req,
                        frames: SETTLE_FRAMES,
                    },
                };
            }
            Err(e) => {
                let _ = req.reply.send(Reply::Error(e));
            }
        }
    }

    // ------------------------------------------------------------ switches

    /// Start / stop the socket server (the settings toggle). Turning it off answers any call in
    /// flight with an error and hides the cursor.
    pub fn set_enabled(&mut self, ctx: &Context, on: bool) {
        if on == self.server.is_some() {
            return;
        }
        if on {
            let path = server::socket_path(&self.app_id);
            let ctx = ctx.clone();
            match server::Server::start(
                path,
                self.tx.clone(),
                Arc::clone(&self.pending),
                move || ctx.request_repaint(),
                self.app_name.clone(),
                instructions(&self.app_name),
                self.tools.clone(),
            ) {
                Ok(s) => {
                    self.server = Some(s);
                    self.server_error = None;
                }
                Err(e) => self.server_error = Some(e.to_string()),
            }
        } else {
            self.server = None;
            self.abort("the agent interface was switched off");
            self.cursor = None;
            self.flash = None;
        }
    }

    /// Activate the registry without a socket; commands come in through [`Agent::submit`].
    pub fn enable_without_server(&mut self) {
        self.detached = true;
    }

    pub fn is_enabled(&self) -> bool {
        self.server.is_some() || self.detached
    }

    /// Queue a command directly (tests, or an in-process driver). The reply arrives on the
    /// returned receiver once `tick` has run it.
    pub fn submit(&self, cmd: Command) -> Receiver<Reply> {
        let (reply, rx) = channel();
        self.pending.fetch_add(1, Ordering::SeqCst);
        let _ = self.tx.send(Request { cmd, reply });
        rx
    }

    /// Labels the agent must not click this frame (and `Enter` is refused while any are set):
    /// the app passes its confirmation verb while a confirmation dialog is open and the
    /// "agent may confirm" setting is off. Call before `tick`, every frame.
    pub fn set_blocked(&mut self, labels: Vec<String>) {
        self.blocked = labels;
    }

    /// The app should compute its state summary for this frame (a call is queued or finishing).
    pub fn wants_state(&self) -> bool {
        self.is_enabled()
            && (self.pending.load(Ordering::SeqCst) > 0
                || matches!(self.phase, Phase::Settle { .. } | Phase::Wait { .. }))
    }

    // ------------------------------------------------------------ status for the UI

    /// `Some((text, busy))` while the interface is on — for a shell lamp.
    pub fn lamp(&self) -> Option<(&'static str, bool)> {
        self.is_enabled()
            .then_some(("AGENT", !matches!(self.phase, Phase::Idle)))
    }

    /// One line for the settings window.
    pub fn status_line(&self) -> String {
        if let Some(e) = &self.server_error {
            return format!("ERROR :: {e}");
        }
        let Some(s) = &self.server else {
            return "OFF".into();
        };
        let clients = s.stats.clients.load(Ordering::Relaxed);
        let calls = s.stats.calls.load(Ordering::Relaxed);
        let file = s
            .path()
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        match (clients, &self.last_action) {
            (0, _) => format!("ON :: WAITING FOR A CLIENT :: {file}"),
            (n, None) => format!(
                "ON :: {n} CLIENT{} :: {calls} CALLS",
                if n > 1 { "S" } else { "" }
            ),
            (n, Some(a)) => format!("ON :: {n} CLIENT{} :: {a}", if n > 1 { "S" } else { "" }),
        }
    }

    pub fn last_action(&self) -> Option<&str> {
        self.last_action.as_deref()
    }

    /// Widgets seen in the last complete frame.
    pub fn widgets(&self) -> &[Widget] {
        &self.widgets
    }

    // ------------------------------------------------------------ per frame

    /// Run the state machine for one frame. Call first thing in `ui()`, before the app reads
    /// input (injected events must be visible to this frame's widgets). `state` is the app's
    /// summary, needed when [`Agent::wants_state`] is true.
    pub fn tick(&mut self, ctx: &Context, state: Option<String>) {
        let now = ctx.input(|i| i.time);
        let active = self.is_enabled();
        let taken = ctx.data_mut(|d| {
            d.insert_temp(active_id(), active);
            std::mem::take(&mut d.get_temp_mut_or_default::<Registry>(registry_id()).widgets)
        });
        if !active {
            self.widgets.clear();
            return;
        }
        if !taken.is_empty() {
            self.widgets = taken;
        }

        let phase = std::mem::replace(&mut self.phase, Phase::Idle);
        self.phase = match phase {
            Phase::Idle => match self.take_request(ctx, state.as_deref()) {
                Some(req) => self.start(ctx, req, now, state.as_deref()),
                None => Phase::Idle,
            },
            Phase::Move {
                req,
                target,
                from,
                t0,
                t1,
                next,
            } => {
                let k = ((now - t0) / (t1 - t0)).clamp(0.0, 1.0) as f32;
                let e = k * k * (3.0 - 2.0 * k);
                self.cursor = Some(from.lerp(target.rect.center(), e));
                self.cursor_seen = now;
                if k < 1.0 {
                    Phase::Move {
                        req,
                        target,
                        from,
                        t0,
                        t1,
                        next,
                    }
                } else {
                    match next {
                        Next::Click => {
                            ctx.input_mut(|i| {
                                i.events.push(Event::AccessKitActionRequest(ActionRequest {
                                    action: Action::Click,
                                    target_tree: TreeId::ROOT,
                                    target_node: target.id.accesskit_id(),
                                    data: None,
                                }));
                            });
                            self.flash = Some((target.rect, now));
                            Phase::Settle {
                                req,
                                frames: SETTLE_FRAMES,
                            }
                        }
                        Next::Focus { chars, submit } => {
                            ctx.memory_mut(|m| m.request_focus(target.id));
                            self.flash = Some((target.rect, now));
                            Phase::Type {
                                req,
                                chars,
                                next_at: now + CHAR_DELAY,
                                submit,
                            }
                        }
                        Next::Flash => {
                            self.flash = Some((target.rect, now));
                            Phase::Settle {
                                req,
                                frames: SETTLE_FRAMES,
                            }
                        }
                    }
                }
            }
            Phase::Type {
                req,
                mut chars,
                next_at,
                submit,
            } => {
                self.cursor_seen = now;
                let mut next_at = next_at;
                if now >= next_at {
                    if let Some(ch) = chars.pop_front() {
                        ctx.input_mut(|i| {
                            if ch == '\n' {
                                push_key(i, Key::Enter, Modifiers::NONE, true);
                            } else {
                                i.events.push(Event::Text(ch.to_string()));
                            }
                        });
                        next_at = now + CHAR_DELAY;
                    }
                }
                if chars.is_empty() && now >= next_at {
                    if submit {
                        Phase::Keys {
                            req,
                            presses: VecDeque::from([(Key::Enter, Modifiers::NONE)]),
                            release: None,
                        }
                    } else {
                        Phase::Settle {
                            req,
                            frames: SETTLE_FRAMES,
                        }
                    }
                } else {
                    Phase::Type {
                        req,
                        chars,
                        next_at,
                        submit,
                    }
                }
            }
            Phase::Keys {
                req,
                mut presses,
                release,
            } => {
                self.cursor_seen = now;
                if let Some((key, mods)) = release {
                    ctx.input_mut(|i| push_key(i, key, mods, false));
                    Phase::Keys {
                        req,
                        presses,
                        release: None,
                    }
                } else if let Some((key, mods)) = presses.pop_front() {
                    ctx.input_mut(|i| push_key(i, key, mods, true));
                    Phase::Keys {
                        req,
                        presses,
                        release: Some((key, mods)),
                    }
                } else {
                    Phase::Settle {
                        req,
                        frames: SETTLE_FRAMES,
                    }
                }
            }
            Phase::Settle { req, frames } => {
                if frames > 0 {
                    Phase::Settle {
                        req,
                        frames: frames - 1,
                    }
                } else if let Some(state) = state.as_deref() {
                    let mut text = self.observation(ctx, state);
                    if let Some(prefix) = self.tool_text.take() {
                        text = format!("{prefix}\n\n{text}");
                    }
                    let _ = req.reply.send(Reply::Text(text));
                    Phase::Idle
                } else {
                    Phase::Settle { req, frames } // the app supplies the state next frame
                }
            }
            Phase::Tool { req, call } => Phase::Tool { req, call },
            Phase::Point { req, label, frames } => match self.find(&label, 1, None) {
                Ok(w) => self.begin_move(ctx, req, w, now, Next::Flash),
                Err(_) if frames > 0 => Phase::Point {
                    req,
                    label,
                    frames: frames - 1,
                },
                Err(_) => Phase::Settle {
                    req,
                    frames: SETTLE_FRAMES,
                },
            },
            Phase::Shot {
                req,
                scale,
                path,
                since,
            } => {
                let image = ctx.input(|i| {
                    i.events.iter().find_map(|e| match e {
                        Event::Screenshot {
                            image, user_data, ..
                        } if user_data
                            .data
                            .as_ref()
                            .is_some_and(|d| d.downcast_ref::<ShotTag>().is_some()) =>
                        {
                            Some(image.clone())
                        }
                        _ => None,
                    })
                });
                if let Some(img) = image {
                    let rgb = encode::Rgb::from_image(&img, scale);
                    let png = rgb.to_png();
                    let mut text = format!("screenshot {}x{} px", rgb.width, rgb.height);
                    if let Some(p) = &path {
                        match std::fs::write(p, &png) {
                            Ok(()) => text.push_str(&format!(" :: saved to {p}")),
                            Err(e) => text.push_str(&format!(" :: could not save to {p}: {e}")),
                        }
                    }
                    let _ = req.reply.send(Reply::Image { png, text });
                    Phase::Idle
                } else if now - since > 3.0 {
                    let _ = req.reply.send(Reply::Error(
                        "no screenshot arrived (the window may be hidden)".into(),
                    ));
                    Phase::Idle
                } else {
                    Phase::Shot {
                        req,
                        scale,
                        path,
                        since,
                    }
                }
            }
            Phase::Wait { req, until } => {
                if now >= until {
                    Phase::Settle { req, frames: 1 }
                } else {
                    Phase::Wait { req, until }
                }
            }
        };

        let cursor_alive = self.cursor.is_some() && now - self.cursor_seen < CURSOR_HOLD + 1.0;
        let flash_alive = self.flash.is_some_and(|(_, t)| now - t < FLASH_SECS);
        if !matches!(self.phase, Phase::Idle) || cursor_alive || flash_alive {
            ctx.request_repaint();
        }
    }

    /// Next queued request, if its prerequisites are met.
    fn take_request(&mut self, ctx: &Context, state: Option<&str>) -> Option<Request> {
        if self.pending.load(Ordering::SeqCst) == 0 {
            return None;
        }
        if state.is_none() {
            ctx.request_repaint(); // the app computes the state summary next frame
            return None;
        }
        let req = self.rx.try_recv().ok()?;
        self.pending.fetch_sub(1, Ordering::SeqCst);
        Some(req)
    }

    fn start(&mut self, ctx: &Context, req: Request, now: f64, state: Option<&str>) -> Phase {
        let state = state.unwrap_or("");
        match req.cmd.clone() {
            Command::Observe => {
                let _ = req.reply.send(Reply::Text(self.observation(ctx, state)));
                Phase::Idle
            }
            Command::Screenshot { scale, path } => {
                ctx.send_viewport_cmd(ViewportCommand::Screenshot(UserData::new(ShotTag)));
                Phase::Shot {
                    req,
                    scale,
                    path,
                    since: now,
                }
            }
            Command::Click { label, nth } => match self.find(&label, nth, None) {
                Err(e) => {
                    let _ = req.reply.send(Reply::Error(e));
                    Phase::Idle
                }
                Ok(w) => {
                    if self.is_blocked(&w.label) {
                        let _ = req.reply.send(Reply::Error(self.blocked_message(&w.label)));
                        return Phase::Idle;
                    }
                    if !w.enabled {
                        let _ = req
                            .reply
                            .send(Reply::Error(format!("`{}` is disabled right now", w.label)));
                        return Phase::Idle;
                    }
                    self.last_action = Some(format!("CLICK ▸ {}", w.label));
                    self.begin_move(ctx, req, w, now, Next::Click)
                }
            },
            Command::Type {
                text,
                label,
                submit,
            } => {
                if submit && !self.blocked.is_empty() {
                    let _ = req.reply.send(Reply::Error(self.blocked_message("Enter")));
                    return Phase::Idle;
                }
                let chars: VecDeque<char> = text.chars().collect();
                let shown: String = text.chars().take(24).collect();
                self.last_action = Some(format!("TYPE ▸ {shown}"));
                match label {
                    Some(label) => match self.find(&label, 1, Some("input")) {
                        Err(e) => {
                            let _ = req.reply.send(Reply::Error(e));
                            Phase::Idle
                        }
                        Ok(w) => self.begin_move(ctx, req, w, now, Next::Focus { chars, submit }),
                    },
                    None => {
                        self.cursor_seen = now;
                        Phase::Type {
                            req,
                            chars,
                            next_at: now,
                            submit,
                        }
                    }
                }
            }
            Command::Key { combo, repeat } => match parse_combo(&combo) {
                Err(e) => {
                    let _ = req.reply.send(Reply::Error(e));
                    Phase::Idle
                }
                Ok((key, mods)) => {
                    if key == Key::Enter && !self.blocked.is_empty() {
                        let _ = req.reply.send(Reply::Error(self.blocked_message("Enter")));
                        return Phase::Idle;
                    }
                    self.last_action = Some(format!("KEY ▸ {}", combo.to_uppercase()));
                    self.cursor_seen = now;
                    Phase::Keys {
                        req,
                        presses: std::iter::repeat_n((key, mods), repeat).collect(),
                        release: None,
                    }
                }
            },
            Command::Wait { ms } => Phase::Wait {
                req,
                until: now + ms as f64 / 1000.0,
            },
            Command::Tool { name, args } => {
                if !self.tools.iter().any(|t| t.name == name) {
                    let _ = req
                        .reply
                        .send(Reply::Error(format!("unknown tool `{name}`")));
                    return Phase::Idle;
                }
                self.last_action = Some(format!("TOOL ▸ {}", name.to_uppercase()));
                self.cursor_seen = now;
                Phase::Tool {
                    req,
                    call: Some(ToolCall { name, args }),
                }
            }
        }
    }

    fn begin_move(
        &mut self,
        ctx: &Context,
        req: Request,
        target: Widget,
        now: f64,
        next: Next,
    ) -> Phase {
        let from = self.cursor.unwrap_or_else(|| ctx.content_rect().center());
        let dist = from.distance(target.rect.center());
        let dur = if target.rect.contains(from) {
            MOVE_MIN
        } else {
            (f64::from(dist / CURSOR_SPEED)).clamp(MOVE_MIN, MOVE_MAX)
        };
        self.cursor = Some(from);
        self.cursor_seen = now;
        Phase::Move {
            req,
            target,
            from,
            t0: now,
            t1: now + dur,
            next,
        }
    }

    /// Cancel whatever is in flight with an error.
    fn abort(&mut self, why: &str) {
        let phase = std::mem::replace(&mut self.phase, Phase::Idle);
        let req = match phase {
            Phase::Idle => None,
            Phase::Move { req, .. }
            | Phase::Type { req, .. }
            | Phase::Keys { req, .. }
            | Phase::Settle { req, .. }
            | Phase::Shot { req, .. }
            | Phase::Wait { req, .. }
            | Phase::Tool { req, .. }
            | Phase::Point { req, .. } => Some(req),
        };
        self.tool_text = None;
        if let Some(req) = req {
            let _ = req.reply.send(Reply::Error(why.into()));
        }
        while let Ok(req) = self.rx.try_recv() {
            self.pending.fetch_sub(1, Ordering::SeqCst);
            let _ = req.reply.send(Reply::Error(why.into()));
        }
    }

    // ------------------------------------------------------------ lookup

    /// Widgets the agent may address: when a modal (foreground layer) is up, only its parts.
    fn addressable(&self) -> Vec<&Widget> {
        let modal = self.widgets.iter().any(|w| w.order == Order::Foreground);
        self.widgets
            .iter()
            .filter(|w| !modal || w.order == Order::Foreground)
            .collect()
    }

    /// Exact label, then case-insensitive, then substring. With `prefer_role`, widgets of that
    /// role are searched first (so `type` finds the input `FILTER` before a button `Filter`).
    fn find(&self, label: &str, nth: usize, prefer_role: Option<&str>) -> Result<Widget, String> {
        let pool = self.addressable();
        let wanted = label.trim();
        fn tiers<'a>(pool: &[&'a Widget], wanted: &str) -> Vec<&'a Widget> {
            let mut hits: Vec<&Widget> =
                pool.iter().copied().filter(|w| w.label == wanted).collect();
            if hits.is_empty() {
                hits = pool
                    .iter()
                    .copied()
                    .filter(|w| w.label.eq_ignore_ascii_case(wanted))
                    .collect();
            }
            if hits.is_empty() {
                let lw = wanted.to_lowercase();
                hits = pool
                    .iter()
                    .copied()
                    .filter(|w| w.label.to_lowercase().contains(&lw))
                    .collect();
            }
            hits
        }
        let mut hits = Vec::new();
        if let Some(role) = prefer_role {
            let same_role: Vec<&Widget> = pool.iter().copied().filter(|w| w.role == role).collect();
            hits = tiers(&same_role, wanted);
        }
        if hits.is_empty() {
            hits = tiers(&pool, wanted);
        }
        match hits.get(nth.saturating_sub(1)) {
            Some(w) => Ok((*w).clone()),
            None if hits.is_empty() => {
                let words: Vec<String> = wanted.split_whitespace().map(str::to_lowercase).collect();
                let mut near: Vec<&str> = pool
                    .iter()
                    .filter(|w| {
                        let l = w.label.to_lowercase();
                        words.iter().any(|x| l.contains(x.as_str()))
                    })
                    .map(|w| w.label.as_str())
                    .collect();
                near.dedup();
                near.truncate(6);
                Err(if near.is_empty() {
                    format!("no widget labelled `{wanted}` (call `observe` for the current labels)")
                } else {
                    format!(
                        "no widget labelled `{wanted}`; similar: {}",
                        near.join(", ")
                    )
                })
            }
            None => Err(format!(
                "only {} widget(s) labelled `{wanted}`, asked for #{nth}",
                hits.len()
            )),
        }
    }

    fn is_blocked(&self, label: &str) -> bool {
        self.blocked.iter().any(|b| b.eq_ignore_ascii_case(label))
    }

    fn blocked_message(&self, what: &str) -> String {
        format!(
            "`{what}` is reserved for the human while this confirmation is open (Settings → AGENT → CONFIRM DIALOGS). Ask them, or click CANCEL."
        )
    }

    // ------------------------------------------------------------ observation

    fn observation(&self, ctx: &Context, state: &str) -> String {
        let size = ctx.content_rect().size();
        let pool = self.addressable();
        let modal = pool.len() != self.widgets.len();
        let mut out = format!(
            "{} :: window {:.0}x{:.0}\n{}\n\nWIDGETS ({}){}\n",
            self.app_name,
            size.x,
            size.y,
            state.trim_end(),
            pool.len(),
            if modal {
                " — a dialog is open; only its parts are listed"
            } else {
                ""
            }
        );
        for w in pool {
            let c = w.rect.center();
            out.push_str(&format!("[{}] {}", w.role, w.label));
            if let Some(v) = &w.value {
                out.push_str(&format!(" = {v:?}"));
            }
            match w.selected {
                Some(true) if w.role == "toggle" => out.push_str(" (on)"),
                Some(true) => out.push_str(" (selected)"),
                _ => {}
            }
            if !w.enabled {
                out.push_str(" (disabled)");
            }
            if self.is_blocked(&w.label) {
                out.push_str(" (human only)");
            }
            out.push_str(&format!(" @{:.0},{:.0}\n", c.x, c.y));
        }
        if !self.blocked.is_empty() {
            out.push_str(&format!(
                "\nNOTE: {} is reserved for the human (Settings → AGENT → CONFIRM DIALOGS).\n",
                self.blocked.join(" / ")
            ));
        }
        out
    }

    // ------------------------------------------------------------ overlay

    /// Draw the agent cursor, the target flash and the last action. Call after the shell.
    pub fn paint(&self, ctx: &Context) {
        if !self.is_enabled() {
            return;
        }
        let now = ctx.input(|i| i.time);
        let pal = theme::palette(ctx);
        let ts = theme::type_scale(ctx);
        let p = ctx.layer_painter(LayerId::new(Order::Tooltip, Id::new("fuide-agent-overlay")));
        if let Some((r, t)) = self.flash {
            let a = (1.0 - (now - t) / FLASH_SECS).clamp(0.0, 1.0) as f32;
            if a > 0.0 {
                let pts = geom::rect(r.expand(3.0));
                geom::glow_outline(&p, &pts, pal.accent.gamma_multiply(a), 1.5, a);
            }
        }
        let Some(c) = self.cursor else { return };
        let age = now - self.cursor_seen;
        let a = (1.0 - (age - CURSOR_HOLD)).clamp(0.0, 1.0) as f32;
        if a <= 0.0 {
            return;
        }
        let busy = !matches!(self.phase, Phase::Idle);
        let col = pal.accent.gamma_multiply(a);
        let r = if busy { 10.0 } else { 8.0 };
        p.circle_stroke(c, r, Stroke::new(1.2, col));
        p.circle_filled(c, 1.8, col);
        let g = r + 3.0;
        for (dx, dy) in [(1.0, 0.0), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0)] {
            let dir = egui::vec2(dx, dy);
            p.line_segment([c + dir * g, c + dir * (g + 6.0)], Stroke::new(1.2, col));
        }
        // halo
        p.circle_stroke(c, r + 5.0, Stroke::new(3.0, col.gamma_multiply(0.15)));
        if let Some(text) = &self.last_action {
            let pos = c + egui::vec2(g + 10.0, -(g + 6.0));
            p.text(
                pos,
                Align2::LEFT_BOTTOM,
                text,
                theme::mono(ts.small),
                pal.text.gamma_multiply(a),
            );
        }
    }
}

fn push_key(i: &mut egui::InputState, key: Key, modifiers: Modifiers, pressed: bool) {
    i.modifiers = modifiers;
    i.events.push(Event::Key {
        key,
        physical_key: None,
        pressed,
        repeat: false,
        modifiers,
    });
}

/// `"cmd+shift+3"`, `"enter"`, `"down"` → key + modifiers.
pub fn parse_combo(combo: &str) -> Result<(Key, Modifiers), String> {
    let mut mods = Modifiers::NONE;
    let mut key = None;
    let parts: Vec<&str> = combo
        .split('+')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    if parts.is_empty() {
        return Err("empty key".into());
    }
    let mac = cfg!(target_os = "macos");
    for (n, part) in parts.iter().enumerate() {
        let last = n + 1 == parts.len();
        match part.to_ascii_lowercase().as_str() {
            "cmd" | "command" | "meta" | "super" | "win" if !last => {
                mods.command = true;
                mods.mac_cmd = mac;
            }
            "ctrl" | "control" if !last => {
                mods.ctrl = true;
                if !mac {
                    mods.command = true;
                }
            }
            "alt" | "option" | "opt" if !last => mods.alt = true,
            "shift" if !last => mods.shift = true,
            _ if last => key = Some(parse_key(part)?),
            other => return Err(format!("unknown modifier `{other}`")),
        }
    }
    key.map(|k| (k, mods)).ok_or_else(|| "no key".into())
}

fn parse_key(name: &str) -> Result<Key, String> {
    let n = name.to_ascii_lowercase();
    let alias = match n.as_str() {
        "enter" | "return" => Some(Key::Enter),
        "esc" | "escape" => Some(Key::Escape),
        "tab" => Some(Key::Tab),
        "space" | " " => Some(Key::Space),
        "backspace" => Some(Key::Backspace),
        "delete" | "del" => Some(Key::Delete),
        "up" => Some(Key::ArrowUp),
        "down" => Some(Key::ArrowDown),
        "left" => Some(Key::ArrowLeft),
        "right" => Some(Key::ArrowRight),
        "home" => Some(Key::Home),
        "end" => Some(Key::End),
        "pageup" | "pgup" => Some(Key::PageUp),
        "pagedown" | "pgdn" => Some(Key::PageDown),
        "comma" | "," => Some(Key::Comma),
        "period" | "." => Some(Key::Period),
        "slash" | "/" => Some(Key::Slash),
        "minus" | "-" => Some(Key::Minus),
        "plus" | "+" => Some(Key::Plus),
        "equals" | "=" => Some(Key::Equals),
        _ => None,
    };
    if let Some(k) = alias {
        return Ok(k);
    }
    Key::from_name(name)
        .or_else(|| {
            Key::ALL.iter().copied().find(|k| {
                k.name().eq_ignore_ascii_case(name) || format!("{k:?}").eq_ignore_ascii_case(name)
            })
        })
        .ok_or_else(|| format!("unknown key `{name}`"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn combos_parse() {
        let (k, m) = parse_combo("cmd+3").unwrap();
        assert_eq!(k, Key::Num3);
        assert!(m.command);
        let (k, m) = parse_combo("Shift+Tab").unwrap();
        assert_eq!(k, Key::Tab);
        assert!(m.shift && !m.command);
        assert_eq!(parse_combo("enter").unwrap().0, Key::Enter);
        assert_eq!(parse_combo("down").unwrap().0, Key::ArrowDown);
        assert_eq!(parse_combo("ArrowDown").unwrap().0, Key::ArrowDown);
        assert_eq!(parse_combo("f").unwrap().0, Key::F);
        assert_eq!(parse_combo("cmd+backspace").unwrap().0, Key::Backspace);
        assert!(parse_combo("hyper+x").is_err());
        assert!(parse_combo("cmd+").is_err());
        assert!(parse_combo("").is_err());
    }

    #[test]
    fn lookup_is_exact_then_case_insensitive_then_substring() {
        let mut a = Agent::new("t", "T");
        let w = |label: &str, role: &'static str, order: Order| Widget {
            id: Id::new(label),
            role,
            label: label.into(),
            enabled: true,
            selected: None,
            value: None,
            rect: Rect::from_min_size(Pos2::ZERO, egui::vec2(10.0, 10.0)),
            order,
        };
        a.widgets = vec![
            w("REFRESH", "button", Order::Background),
            w("OUTDATED  3", "item", Order::Background),
            w("FILTER", "input", Order::Background),
            w("Filter", "button", Order::Background),
        ];
        assert_eq!(a.find("REFRESH", 1, None).unwrap().label, "REFRESH");
        assert_eq!(a.find("outdated", 1, None).unwrap().label, "OUTDATED  3");
        assert_eq!(a.find("filter", 1, None).unwrap().label, "FILTER");
        assert_eq!(a.find("Filter", 1, Some("input")).unwrap().role, "input");
        assert_eq!(a.find("filter", 2, None).unwrap().label, "Filter");
        assert!(a.find("filter", 3, None).unwrap_err().contains("only 2"));
        assert!(a.find("PURGE", 1, None).unwrap_err().contains("no widget"));
        // a modal hides everything else
        a.widgets.push(w("CANCEL", "button", Order::Foreground));
        assert!(a.find("REFRESH", 1, None).is_err());
        assert_eq!(a.find("cancel", 1, None).unwrap().label, "CANCEL");
    }

    #[test]
    fn blocked_labels_are_case_insensitive() {
        let mut a = Agent::new("t", "T");
        a.set_blocked(vec!["UPGRADE ALL".into()]);
        assert!(a.is_blocked("upgrade all"));
        assert!(!a.is_blocked("UPGRADE"));
    }
}
