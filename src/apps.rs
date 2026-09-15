//! Desktop entries for the launcher: scanning `applications` directories,
//! matching a query against them, resolving their icons through the icon
//! theme, and launching them.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// One launchable application.
#[derive(Clone, Debug)]
pub struct App {
    pub name: String,
    /// GenericName or Comment, shown under the name
    pub subtitle: String,
    pub exec: String,
    pub terminal: bool,
    pub icon: Option<String>,
    /// lower-cased haystacks the query is matched against
    keys: Vec<String>,
    /// how often it has been launched from here
    pub uses: u32,
    pub id: String,
}

fn data_dirs() -> Vec<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    let mut dirs = Vec::new();
    let user = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".local/share"));
    dirs.push(user);
    let system = std::env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".into());
    dirs.extend(system.split(':').filter(|s| !s.is_empty()).map(PathBuf::from));
    dirs
}

/// The current locale's language, e.g. "ru" (for `Name[ru]`).
fn lang() -> Option<String> {
    for var in ["LC_ALL", "LC_MESSAGES", "LANG"] {
        if let Ok(v) = std::env::var(var) {
            let v = v.split('.').next().unwrap_or("").to_string();
            if !v.is_empty() && v != "C" && v != "POSIX" {
                return Some(v);
            }
        }
    }
    None
}

/// Every visible desktop entry, best first (most used, then by name).
pub fn scan() -> Vec<App> {
    let lang = lang();
    let short = lang.as_ref().and_then(|l| l.split('_').next().map(str::to_string));
    let usage = Usage::load();
    let mut seen: HashMap<String, App> = HashMap::new();
    for dir in data_dirs() {
        let apps = dir.join("applications");
        let Ok(entries) = std::fs::read_dir(&apps) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("desktop") {
                continue;
            }
            let id = path.file_name().unwrap_or_default().to_string_lossy().to_string();
            // earlier directories win (the user's own entries override the
            // system ones with the same id)
            if seen.contains_key(&id) {
                continue;
            }
            if let Some(mut app) = parse(&path, lang.as_deref(), short.as_deref()) {
                app.id = id.clone();
                app.uses = usage.get(&id);
                seen.insert(id, app);
            }
        }
    }
    let mut apps: Vec<App> = seen.into_values().collect();
    apps.sort_by(|a, b| {
        b.uses
            .cmp(&a.uses)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            // among namesakes the one that describes itself goes first
            .then_with(|| b.subtitle.is_empty().cmp(&a.subtitle.is_empty()))
    });
    // the same program often arrives twice (Telegram ships both
    // telegram-desktop.desktop and its reverse-DNS twin): one row each
    apps.dedup_by(|a, b| a.name.eq_ignore_ascii_case(&b.name));
    apps
}

fn parse(path: &Path, lang: Option<&str>, short: Option<&str>) -> Option<App> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut in_entry = false;
    let (mut name, mut generic, mut comment): (Option<(u8, String)>, Option<(u8, String)>, Option<(u8, String)>) = (None, None, None);
    let (mut exec, mut icon) = (None, None);
    let (mut terminal, mut hidden) = (false, false);
    let mut keywords = Vec::new();
    let mut try_exec: Option<String> = None;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_entry = line == "[Desktop Entry]";
            continue;
        }
        if !in_entry || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else { continue };
        let (key, value) = (key.trim(), value.trim());
        // Name / Name[ru_RU] / Name[ru]: the most specific tag wins
        let (base, tag) = match key.split_once('[') {
            Some((base, rest)) => (base, Some(rest.trim_end_matches(']'))),
            None => (key, None),
        };
        let rank = match tag {
            Some(t) if Some(t) == lang => 3,
            Some(t) if Some(t) == short => 2,
            None => 1,
            _ => 0,
        };
        let take = |slot: &mut Option<(u8, String)>| {
            if rank > 0 && slot.as_ref().is_none_or(|(r, _)| rank > *r) {
                *slot = Some((rank, value.to_string()));
            }
        };
        match base {
            "Name" => take(&mut name),
            "GenericName" => take(&mut generic),
            "Comment" => take(&mut comment),
            "Exec" => exec = Some(value.to_string()),
            "TryExec" => try_exec = Some(value.to_string()),
            "Icon" => icon = Some(value.to_string()),
            "Terminal" => terminal = value == "true",
            "NoDisplay" | "Hidden" => hidden |= value == "true",
            "Keywords" => keywords.extend(value.split(';').filter(|s| !s.is_empty()).map(str::to_string)),
            _ => {}
        }
    }
    if hidden {
        return None;
    }
    let name = name?.1;
    let exec = exec?;
    if let Some(try_exec) = try_exec {
        if !executable_exists(&try_exec) {
            return None;
        }
    }
    let subtitle = generic.or(comment).map(|(_, v)| v).unwrap_or_default();
    let mut keys = vec![name.to_lowercase()];
    if !subtitle.is_empty() {
        keys.push(subtitle.to_lowercase());
    }
    keys.extend(keywords.iter().map(|k| k.to_lowercase()));
    if let Some(bin) = exec.split_whitespace().next() {
        keys.push(bin.rsplit('/').next().unwrap_or(bin).to_lowercase());
    }
    Some(App { name, subtitle, exec, terminal, icon, keys, uses: 0, id: String::new() })
}

fn executable_exists(cmd: &str) -> bool {
    if cmd.contains('/') {
        return Path::new(cmd).exists();
    }
    std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).any(|p| p.join(cmd).exists()))
        .unwrap_or(false)
}

/// Fuzzy match: the query's characters in order, rewarding matches at the
/// start of the name and of words, then the entry's own usage count.
/// `None` when it does not match at all.
pub fn score(app: &App, query: &str) -> Option<i32> {
    if query.is_empty() {
        return Some(app.uses as i32 * 4);
    }
    let q = query.to_lowercase();
    let mut best = None;
    for (i, key) in app.keys.iter().enumerate() {
        if let Some(mut s) = match_score(key, &q) {
            s -= (i as i32) * 40; // the name matters most, then the rest
            best = Some(best.map_or(s, |b: i32| b.max(s)));
        }
    }
    best.map(|s| s + (app.uses.min(20) as i32) * 6)
}

/// How well `hay` answers `needle`: a literal hit wins (the earlier and
/// the more word-aligned the better), an acronym over word starts comes
/// next ("vsc" for "Visual Studio Code"), and anything more scattered than
/// that is not a match at all — Spotlight would not show it either.
fn match_score(hay: &str, needle: &str) -> Option<i32> {
    if let Some(pos) = hay.find(needle) {
        let at_word = pos == 0 || hay[..pos].ends_with([' ', '-', '_', '.', '/', '(']);
        let bonus = if pos == 0 {
            400
        } else if at_word {
            200
        } else {
            0
        };
        return Some(1000 - pos as i32 * 2 + bonus - (hay.chars().count() as i32).min(40));
    }
    acronym_score(hay, needle)
}

/// Every character of `needle` starting a word of `hay`, in order.
fn acronym_score(hay: &str, needle: &str) -> Option<i32> {
    let mut words = hay.split(|c: char| c == ' ' || c == '-' || c == '_' || c == '.' || c == '/');
    let mut skipped = 0;
    for c in needle.chars() {
        loop {
            let word = words.next()?;
            let Some(first) = word.chars().next() else { continue };
            if first == c {
                break;
            }
            skipped += 1;
        }
    }
    Some(700 - skipped * 30)
}

/// Rank `apps` for `query`, best first.
pub fn search<'a>(apps: &'a [App], query: &str, limit: usize) -> Vec<&'a App> {
    let mut hits: Vec<(i32, &App)> = apps.iter().filter_map(|a| score(a, query).map(|s| (s, a))).collect();
    hits.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.name.to_lowercase().cmp(&b.1.name.to_lowercase())));
    hits.into_iter().take(limit).map(|(_, a)| a).collect()
}

/// Launch the app, detached from the island.
pub fn launch(app: &App) {
    let exec = strip_field_codes(&app.exec);
    let command = if app.terminal {
        let term = std::env::var("TERMINAL").unwrap_or_else(|_| "alacritty".into());
        format!("{term} -e {exec}")
    } else {
        exec
    };
    let spawned = Command::new("sh")
        .arg("-c")
        .arg(format!("exec setsid -f {command}"))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    match spawned {
        Ok(mut child) => {
            let _ = child.wait();
            Usage::load().bump(&app.id);
        }
        Err(e) => eprintln!("island-shell: launch {}: {e}", app.name),
    }
}

/// `%f`, `%U`, `%i`, `%c`… have no meaning without a file to open.
fn strip_field_codes(exec: &str) -> String {
    let mut out = String::with_capacity(exec.len());
    let mut chars = exec.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '%' {
            match chars.next() {
                Some('%') => out.push('%'),
                Some(_) => {}
                None => {}
            }
        } else {
            out.push(c);
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// How often each entry has been launched (Spotlight's "you meant this
/// one"), kept as `id count` lines in the state directory.
pub struct Usage {
    counts: HashMap<String, u32>,
}

impl Usage {
    fn path() -> PathBuf {
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
        let base = std::env::var_os("XDG_STATE_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".local/state"));
        base.join("island-shell").join("usage")
    }

    pub fn load() -> Usage {
        let mut counts = HashMap::new();
        if let Ok(text) = std::fs::read_to_string(Self::path()) {
            for line in text.lines() {
                if let Some((id, n)) = line.rsplit_once(' ') {
                    counts.insert(id.to_string(), n.trim().parse().unwrap_or(0));
                }
            }
        }
        Usage { counts }
    }

    pub fn get(&self, id: &str) -> u32 {
        self.counts.get(id).copied().unwrap_or(0)
    }

    pub fn bump(&mut self, id: &str) {
        *self.counts.entry(id.to_string()).or_insert(0) += 1;
        let path = Self::path();
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let mut text = String::new();
        for (id, n) in &self.counts {
            text.push_str(&format!("{id} {n}\n"));
        }
        let _ = std::fs::write(path, text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(name: &str, keys: &[&str]) -> App {
        App {
            name: name.into(),
            subtitle: String::new(),
            exec: String::new(),
            terminal: false,
            icon: None,
            keys: keys.iter().map(|k| k.to_lowercase()).collect(),
            uses: 0,
            id: name.into(),
        }
    }

    #[test]
    fn prefix_beats_scattered() {
        let fire = app("Firefox", &["firefox"]);
        let files = app("Files", &["files"]);
        assert!(score(&fire, "fir").unwrap() > score(&files, "fir").unwrap_or(0));
    }

    #[test]
    fn matches_literal_and_acronym_but_not_noise() {
        let a = app("Visual Studio Code", &["visual studio code", "code"]);
        assert!(score(&a, "code").is_some());
        assert!(score(&a, "vsc").is_some());
        assert!(score(&a, "zzz").is_none());
        // "fir" must not drag in a name that merely contains f, i and r
        let rofi = app("Rofi Theme Selector", &["rofi theme selector"]);
        assert!(score(&rofi, "fir").is_none());
    }

    #[test]
    fn field_codes_are_stripped() {
        assert_eq!(strip_field_codes("telegram-desktop -- %u"), "telegram-desktop --");
        assert_eq!(strip_field_codes("app %F --flag"), "app --flag");
        assert_eq!(strip_field_codes("app 100%%"), "app 100%");
    }
}
