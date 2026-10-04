//! Notification data shared by the native D-Bus server and the island UI.

pub use crate::notify_kinds::AppKind;

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Notification {
    /// Zero for internal notices; desktop notifications have a live D-Bus ID.
    pub id: u32,
    pub app: String,
    pub summary: String,
    pub body: String,
    pub kind: AppKind,
    pub critical: bool,
    /// notifications sharing a tag replace each other in place instead of
    /// rolling (LSP progress updating itself, say)
    pub tag: Option<String>,
    /// a ring that fills instead of the countdown that empties
    pub progress: Option<f64>,
    pub actions: Vec<(String, String)>,
    pub transient: bool,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub image: Option<String>,
    /// the app takes a typed reply (the "inline-reply" action), with
    /// this placeholder for the field
    #[serde(default)]
    pub reply: Option<String>,
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
    Notification { id: 0, app: pretty_app(app, app_kind), summary: summary.into(), body: body.into(), kind, critical, tag: None, progress: None, actions: Vec::new(), transient: false, icon: None, image: None, reply: None }
}

impl Notification {
    pub fn leading_image(&self) -> Option<&str> {
        self.icon.as_deref().or(self.image.as_deref())
    }
}

/// Blit an already-scaled themed icon or notification image. A missing asset
/// is queued on the icon worker and leaves the caller's vector fallback.
pub fn draw_media(cr: &cairo::Context, cache: &crate::iconload::Icons, name: &str,
    cx: f64, cy: f64, size: f64, scale: f64, alpha: f64) -> bool {
    let px = (size * scale).round().max(1.0) as i32;
    let Some(surface) = cache.get_scaled(name, px) else { return false };
    cr.save().unwrap();
    cr.translate(cx, cy);
    cr.scale(1.0 / scale, 1.0 / scale);
    let _ = cr.set_source_surface(&surface, -(surface.width() as f64) / 2.0, -(surface.height() as f64) / 2.0);
    let _ = cr.paint_with_alpha(alpha);
    cr.restore().unwrap();
    true
}

/// Notification bodies may carry markup (Telegram sends `<b>name</b>`):
/// the island draws plain text, so drop the tags and unescape the entities.
pub(crate) fn plain(s: &str) -> String {
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

pub(crate) fn classify(app: &str, icon: &str, critical: bool) -> AppKind {
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

pub(crate) fn pretty_app(app: &str, kind: AppKind) -> String {
    match kind {
        AppKind::Telegram => "Telegram".into(),
        AppKind::System if app.is_empty() || app == "notify-send" => "System".into(),
        AppKind::Firefox => "Firefox".into(),
        AppKind::Discord => "Discord".into(),
        _ if app.is_empty() => "Notification".into(),
        _ => app.into(),
    }
}
