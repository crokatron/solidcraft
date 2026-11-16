//! Messages (#2763): every status message of this run, kept so it can be read and copied after
//! the status chip has faded. Help › Messages… and a click on the chip open the window; the
//! control channel reads them as `ui.messages`.

use std::collections::VecDeque;

use egui::{RichText, vec2};
use serde_json::{Value, json};

use crate::SolveApp;
use solidcraft_theme::Tokens;
use solidcraft_ui_text::{format, tr};

/// Messages kept (the oldest go first).
pub const KEEP: usize = 200;
/// The longest message kept, in characters (a longer one is cut, saying so).
const MAX_CHARS: usize = 4000;

/// One status message.
#[derive(Clone, Debug, PartialEq)]
pub struct Message {
    pub text: String,
    pub error: bool,
    /// When it last came (ms since the Unix epoch).
    pub at: f64,
    /// How many times in a row it came.
    pub count: u32,
    /// Explicit file-operation location; never inferred from arbitrary diagnostic text.
    pub folder: Option<String>,
    pub dismissed: bool,
}

#[derive(Default)]
pub struct Messages {
    /// Oldest first.
    list: VecDeque<Message>,
    /// The window is open.
    pub open: bool,
}

impl Messages {
    /// Keep a message: the same one again in a row counts up instead.
    pub fn push(&mut self, text: &str, error: bool, at: f64) {
        if let Some(last) = self.list.back_mut()
            && last.text == text
            && last.error == error
        {
            last.count = last.count.saturating_add(1);
            last.at = at;
            return;
        }
        let text =
            if text.chars().count() > MAX_CHARS { format!("{}…", text.chars().take(MAX_CHARS).collect::<String>()) } else { text.to_string() };
        self.list.push_back(Message { text, error, at, count: 1, folder: None, dismissed: false });
        while self.list.len() > KEEP {
            self.list.pop_front();
        }
    }

    /// Oldest first.
    pub fn iter(&self) -> impl DoubleEndedIterator<Item = &Message> + ExactSizeIterator {
        self.list.iter()
    }

    pub fn clear(&mut self) {
        self.list.clear();
    }

    pub fn file(&mut self, text: &str, path: &str, at: f64) {
        self.push(text, false, at);
        if let Some(message) = self.list.back_mut() {
            message.folder = std::path::Path::new(path).parent().map(|p| p.to_string_lossy().into_owned()).filter(|p| !p.is_empty());
            message.dismissed = false;
        }
    }

    pub fn pending(&self) -> Option<&Message> {
        self.list.iter().rev().find(|m| m.folder.is_some() && !m.dismissed)
    }

    pub fn dismiss(&mut self, at: f64) {
        for message in &mut self.list {
            if message.at == at {
                message.dismissed = true;
            }
        }
    }

    pub fn dismiss_all(&mut self) {
        for message in &mut self.list {
            message.dismissed = true;
        }
    }

    /// The messages as text, oldest first, one per line: how long ago, `error:` for errors, the
    /// text and the repeat count.
    pub fn as_text(&self, now: f64) -> String {
        self.list.iter().map(|m| format!("{}\t{}", ago(now, m.at), line(m))).collect::<Vec<_>>().join("\n")
    }
}

/// A message as one line: `error:` for errors, and `(×n)` when it came more than once.
fn line(m: &Message) -> String {
    let repeats = if m.count > 1 { format!(" (×{})", m.count) } else { String::new() };
    format!("{}{}{repeats}", if m.error { "error: " } else { "" }, m.text)
}

/// How long ago `at` was, at `now` (both ms): seconds, minutes or hours.
pub(crate) fn ago(now: f64, at: f64) -> String {
    let s = ((now - at) / 1000.0).max(0.0).floor();
    let (id, n) = if s < 60.0 {
        ("messages.ago_s", s)
    } else if s < 3600.0 {
        ("messages.ago_min", (s / 60.0).floor())
    } else {
        ("messages.ago_h", (s / 3600.0).floor())
    };
    format(id, &[("n", &format!("{n:.0}"))])
}

/// `ui.messages`: `clear?: true` empties the list, `open?: bool` shows or hides the window; the
/// reply lists them, oldest first (`text`, `error`, `at` in ms since the epoch, `count`).
pub fn control(app: &mut SolveApp, p: &Value) -> Value {
    if p.get("clear").and_then(Value::as_bool) == Some(true) {
        app.messages.clear();
    }
    if let Some(open) = p.get("open").and_then(Value::as_bool) {
        app.messages.open = open;
    }
    let list: Vec<Value> = app
        .messages
        .iter()
        .map(|m| json!({"text": m.text, "error": m.error, "at": m.at, "count": m.count, "folder":m.folder,"dismissed":m.dismissed}))
        .collect();
    json!({ "messages": list, "open": app.messages.open })
}

/// The Messages window.
pub fn show(app: &mut SolveApp, ctx: &egui::Context) {
    if !app.messages.open {
        return;
    }
    let t = Tokens::get();
    let now = crate::now_ms();
    let (closed, _) = crate::ui_kit::Window::new(egui::Id::unique("sc_messages"), "list", tr("notifications.title"))
        .fields()
        .actions("", "", None)
        .show(ctx, true, |ui| {
            ui.horizontal(|ui| {
                let copy = crate::ui_kit::button(ui, tr("messages.copy_all"), crate::ui_kit::Button::Enabled(app.messages.iter().len() > 0));
                crate::scenario::publish_handle("messages:copy", copy.rect.center());
                if copy.clicked() {
                    ctx.copy_text(app.messages.as_text(now));
                }
                let clear = crate::ui_kit::button(ui, tr("messages.clear"), crate::ui_kit::Button::Enabled(app.messages.iter().len() > 0));
                crate::scenario::publish_handle("messages:clear", clear.rect.center());
                if clear.clicked() {
                    app.messages.clear();
                }
                let dismiss = ui.button(tr("notifications.dismiss_all"));
                crate::scenario::publish_handle("notifications:dismiss-all", dismiss.rect.center());
                if dismiss.clicked() {
                    app.messages.dismiss_all();
                    app.status = None;
                }
            });
            ui.separator();
            if app.messages.iter().len() == 0 {
                ui.label(RichText::new(tr("messages.none")).color(t.text_dim));
                return;
            }
            egui::ScrollArea::vertical().max_height(solidcraft_theme::Tokens::get().form_card_width).auto_shrink([false, true]).show(ui, |ui| {
                let rows: Vec<_> = app.messages.iter().rev().cloned().collect();
                for (index, m) in rows.iter().enumerate() {
                    ui.horizontal_top(|ui| {
                        ui.add_sized(vec2(70.0, 14.0), egui::Label::new(RichText::new(ago(now, m.at)).small().color(t.text_dim)));
                        let text = RichText::new(line(m)).color(if m.error { t.error } else { t.text });
                        ui.add(egui::Label::new(text).wrap().selectable(true));
                    });
                    if let Some(folder) = &m.folder {
                        ui.horizontal(|ui| {
                            let copy = ui.button(tr("notifications.copy_folder"));
                            crate::scenario::publish_handle(&format!("notifications:copy-folder:{index}"), copy.rect.center());
                            if copy.clicked() {
                                ctx.copy_text(folder.clone());
                            }
                            if !m.dismissed && ui.button(tr("notifications.dismiss")).clicked() {
                                app.messages.dismiss(m.at);
                            }
                        });
                    }
                }
            });
        });
    if closed {
        app.messages.open = false;
    }
    // The ages tick on while it is open.
    ctx.request_repaint_after(std::time::Duration::from_secs(1));
}

#[cfg(test)]
mod tests {
    use super::{KEEP, Messages};

    #[test]
    fn repeats_count_up_and_the_oldest_go_past_the_limit() {
        crate::i18n::install();
        let mut m = Messages::default();
        m.push("Box done", false, 1000.0);
        m.push("Box done", false, 2000.0);
        m.push("`face` is required", true, 3000.0);
        let got: Vec<(&str, bool, u32)> = m.iter().map(|x| (x.text.as_str(), x.error, x.count)).collect();
        assert_eq!(got, [("Box done", false, 2), ("`face` is required", true, 1)]);
        assert_eq!(m.as_text(63_000.0), "1 min ago\tBox done (×2)\n1 min ago\terror: `face` is required");
        for i in 0..KEEP + 5 {
            m.push(&format!("message {i}"), false, 0.0);
        }
        assert_eq!(m.iter().len(), KEEP);
        assert_eq!(m.iter().next().map(|x| x.text.as_str()), Some("message 5"), "the oldest went first");
    }

    #[test]
    fn a_very_long_message_is_cut() {
        let mut m = Messages::default();
        m.push(&"x".repeat(10_000), true, 0.0);
        assert_eq!(m.iter().next().map(|x| x.text.chars().count()), Some(4001));
    }

    #[test]
    fn file_notifications_survive_later_status_and_dismiss_independently() {
        let mut m = Messages::default();
        m.file("Exported first", "C:/Projects/My Door/first.step", 1.0);
        m.push("Selection changed", false, 2.0);
        assert_eq!(m.pending().and_then(|m| m.folder.as_deref()), Some("C:/Projects/My Door"));
        m.file("Exported second", "C:/Projects/My Door/second.x_t", 3.0);
        m.dismiss(3.0);
        assert_eq!(m.pending().map(|m| m.text.as_str()), Some("Exported first"));
        m.dismiss_all();
        assert!(m.pending().is_none());
        assert_eq!(m.iter().len(), 3, "dismissal must retain notification history");
    }

    #[test]
    fn file_notification_stays_after_timeout_and_opens_center() {
        let mut h = crate::scenario::Harness::new();
        h.app.home.open = false;
        h.app.messages.file("Exported test", "C:/Projects/My Door/test.step", crate::now_ms() - 60_000.0);
        h.frames(3);
        assert!(crate::scenario::handle_at("notifications:copy-folder").is_some());
        h.step(&serde_json::json!({"click":{"handle":"status:chip"}})).unwrap();
        assert!(h.app.messages.open);
        assert!(crate::scenario::handle_at("notifications:copy-folder:0").is_some());
        h.step(&serde_json::json!({"click":{"handle":"notifications:dismiss-all"}})).unwrap();
        assert!(h.app.messages.pending().is_none());
        assert_eq!(h.app.messages.iter().len(), 1);
    }

    /// Status messages land in the list, a click on the chip and Help › Messages… open the window,
    /// and `ui.messages` reads, opens and clears it.
    #[test]
    fn status_messages_are_kept_and_the_window_opens() {
        use serde_json::json;
        let mut h = crate::scenario::Harness::new();
        h.app.home.open = false;
        h.app.set_status("STEP export needs a solid body", true);
        h.app.set_status("Box done", false);
        h.frames(2);
        let chip = crate::scenario::handle_at("status:chip").expect("the chip is drawn");
        h.call("ui.click", json!({"x": chip.x, "y": chip.y}));
        h.frames(2);
        assert!(h.app.messages.open, "a click on the chip opens the messages");
        assert!(crate::scenario::handle_at("messages:copy").is_some());
        let out = h.call("ui.messages", json!({}));
        let texts: Vec<&str> = out["result"]["messages"].as_array().into_iter().flatten().filter_map(|m| m["text"].as_str()).collect();
        assert_eq!(texts, ["STEP export needs a solid body", "Box done"], "{out}");
        assert_eq!(out["result"]["messages"][0]["error"], true);
        let out = h.call("ui.messages", json!({"clear": true, "open": false}));
        assert_eq!((out["result"]["messages"].as_array().map(Vec::len), out["result"]["open"].as_bool()), (Some(0), Some(false)));
        crate::help::run(&mut h.app, &h.ctx.clone(), "messages");
        assert!(h.app.messages.open, "Help › Messages… opens it too");
    }
}
