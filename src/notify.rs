//! Notifications: instead of a server of our own (dunst keeps the history,
//! the sounds and the fallback pop-ups) the island watches the calls going
//! to `org.freedesktop.Notifications.Notify` on the session bus as a D-Bus
//! monitor, so every notification reaches it natively — no rule scripts, no
//! state files.

use std::collections::HashMap;
use std::thread;

use smithay_client_toolkit::reexports::calloop::channel::Sender;
use zbus::blocking::{Connection, MessageIterator, fdo::MonitoringProxy};
use zbus::zvariant::{OwnedValue, Value};
use zbus::{MatchRule, message::Type};

pub use crate::notify_kinds::AppKind;

#[derive(Clone, Debug, PartialEq)]
pub struct Notification {
    pub app: String,
    pub summary: String,
    pub body: String,
    pub kind: AppKind,
    pub critical: bool,
}

impl Notification {
    /// The two lines the pill shows: the app's name and the message.
    pub fn message(&self) -> String {
        if self.body.is_empty() {
            self.summary.clone()
        } else if self.summary.is_empty() {
            self.body.clone()
        } else {
            format!("{} — {}", self.summary, self.body)
        }
    }
}

/// Build a notification by hand (`island-shell demo <app> <summary>|<body>`,
/// for looking at the icons and the roll without waiting for a real one).
pub fn fabricate(app: &str, summary: &str, body: &str) -> Notification {
    let critical = app.eq_ignore_ascii_case("critical");
    let app_kind = classify(app, "", false);
    let kind = if critical { AppKind::Critical } else { app_kind };
    Notification { app: pretty_app(app, app_kind), summary: summary.into(), body: body.into(), kind, critical }
}

pub fn spawn_watch(tx: Sender<Notification>) {
    thread::Builder::new()
        .name("notify".into())
        .spawn(move || {
            if let Err(e) = watch(&tx) {
                eprintln!("island-shell: notification watch: {e}");
            }
        })
        .expect("spawn notify thread");
}

fn watch(tx: &Sender<Notification>) -> zbus::Result<()> {
    // a monitor connection can only listen; it is separate from the one the
    // MPRIS watcher uses
    let conn = Connection::session()?;
    let rule = MatchRule::builder()
        .msg_type(Type::MethodCall)
        .interface("org.freedesktop.Notifications")?
        .member("Notify")?
        .build();
    MonitoringProxy::new(&conn)?.become_monitor(&[rule], 0)?;
    for msg in MessageIterator::from(conn) {
        let Ok(msg) = msg else { continue };
        if msg.header().member().map(|m| m.as_str()) != Some("Notify") {
            continue;
        }
        // (app_name, replaces_id, app_icon, summary, body, actions, hints, timeout)
        let Ok((app, _id, icon, summary, body, _actions, hints, _timeout)) =
            msg.body()
                .deserialize::<(String, u32, String, String, String, Vec<String>, HashMap<String, OwnedValue>, i32)>()
        else {
            continue;
        };
        let critical = hints.get("urgency").and_then(|v| u8::from_value(v)).is_some_and(|u| u >= 2);
        let transient = hints.get("transient").and_then(|v| bool::from_value(v)).unwrap_or(false);
        if transient {
            continue; // progress pop-ups and the like: not island material
        }
        // the app keeps its own identity even when the alert is critical
        let app_kind = classify(&app, &icon, false);
        let kind = if critical { AppKind::Critical } else { app_kind };
        let _ = tx.send(Notification {
            app: pretty_app(&app, app_kind),
            summary: plain(&summary),
            body: plain(&body),
            kind,
            critical,
        });
    }
    Ok(())
}

/// Notification bodies may carry markup (Telegram sends `<b>name</b>`):
/// the island draws plain text, so drop the tags and unescape the entities.
fn plain(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut depth = 0usize;
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '<' => depth += 1,
            '>' if depth > 0 => depth -= 1,
            _ if depth > 0 => {}
            '&' => {
                let entity: String = chars.by_ref().take_while(|c| *c != ';').collect();
                out.push_str(match entity.as_str() {
                    "lt" => "<",
                    "gt" => ">",
                    "quot" => "\"",
                    "apos" | "#39" => "'",
                    "amp" => "&",
                    _ => "",
                });
            }
            c => out.push(c),
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn classify(app: &str, icon: &str, critical: bool) -> AppKind {
    if critical {
        return AppKind::Critical;
    }
    let hay = format!("{} {}", app.to_ascii_lowercase(), icon.to_ascii_lowercase());
    if hay.contains("telegram") {
        AppKind::Telegram
    } else if app.is_empty() || app == "notify-send" {
        AppKind::System
    } else if hay.contains("firefox") {
        AppKind::Firefox
    } else if hay.contains("discord") {
        AppKind::Discord
    } else if hay.contains("thunderbird") || hay.contains("evolution") || hay.contains("mail") {
        AppKind::Mail
    } else {
        AppKind::Bell
    }
}

fn pretty_app(app: &str, kind: AppKind) -> String {
    match kind {
        AppKind::Telegram => "Telegram".into(),
        AppKind::System if app.is_empty() || app == "notify-send" => "System".into(),
        AppKind::Firefox => "Firefox".into(),
        AppKind::Discord => "Discord".into(),
        _ if app.is_empty() => "Notification".into(),
        _ => app.into(),
    }
}

/// `u8`/`bool` out of a hint that may be wrapped in a variant.
trait FromHint: Sized {
    fn from_value(v: &Value<'_>) -> Option<Self>;
}

impl FromHint for u8 {
    fn from_value(v: &Value<'_>) -> Option<u8> {
        match v {
            Value::U8(b) => Some(*b),
            Value::Value(inner) => u8::from_value(inner),
            _ => None,
        }
    }
}

impl FromHint for bool {
    fn from_value(v: &Value<'_>) -> Option<bool> {
        match v {
            Value::Bool(b) => Some(*b),
            Value::Value(inner) => bool::from_value(inner),
            _ => None,
        }
    }
}
