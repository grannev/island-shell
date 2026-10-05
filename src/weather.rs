//! The weather, for the calendar sheet: now and the days ahead, from
//! open-meteo.com (no key). The place comes from the config's `weather`
//! (`auto`, a city, `lat,lon` or `off`); `auto` takes the city of the
//! system time zone (Europe/Samara → Самара) and finds it with open-meteo's
//! geocoder — no IP lookup. Everything runs on a background thread with
//! `curl`, at most every 30 minutes; the last forecast is cached under
//! `$XDG_CACHE_HOME/island-shell/` so a fresh island has it at once.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use cairo::Context;
use smithay_client_toolkit::reexports::calloop::channel::Sender;

use crate::calendar::Date;

const TTL: u64 = 30 * 60;

#[derive(Clone, Debug, PartialEq)]
pub struct Day {
    pub date: Date,
    pub code: u32,
    pub high: f64,
    pub low: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Forecast {
    /// the place's name ("Самара"), empty for bare coordinates
    pub place: String,
    pub temperature: f64,
    pub code: u32,
    pub day: bool,
    pub days: Vec<Day>,
    fetched: u64,
}

impl Forecast {
    pub fn on(&self, date: Date) -> Option<&Day> {
        self.days.iter().find(|d| d.date == date)
    }
}

#[derive(Default)]
struct State {
    place: String,
    forecast: Option<Arc<Forecast>>,
    busy: bool,
    fresh: bool,
    waker: Option<Sender<()>>,
    loaded_cache: bool,
}

fn state() -> &'static Mutex<State> {
    static STATE: OnceLock<Mutex<State>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(State { place: "auto".into(), ..State::default() }))
}

/// The config's `weather`: `auto`, a city, `lat,lon` or `off`.
pub fn configure(place: &str) {
    if let Ok(mut s) = state().lock() {
        if s.place != place {
            s.place = place.to_string();
            s.forecast = None;
            s.loaded_cache = true;
        }
    }
}

pub fn set_waker(tx: Sender<()>) {
    if let Ok(mut s) = state().lock() { s.waker = Some(tx); }
}

/// The forecast, if there is one (never blocks on the network).
pub fn current() -> Option<Arc<Forecast>> {
    state().lock().ok()?.forecast.clone()
}

/// Did a new forecast arrive since the last call?
pub fn take_fresh() -> bool {
    state().lock().map(|mut s| std::mem::take(&mut s.fresh)).unwrap_or(false)
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

fn cache_dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))?;
    Some(base.join("island-shell"))
}

/// Fetch a new forecast when the last one is older than 30 minutes.
/// Never blocks.
pub fn refresh() {
    let place = {
        let Ok(mut s) = state().lock() else { return };
        if s.place == "off" || s.busy { return; }
        if s.forecast.as_ref().is_some_and(|f| now_secs().saturating_sub(f.fetched) < TTL) { return; }
        s.busy = true;
        s.place.clone()
    };
    std::thread::spawn(move || {
        let cached = {
            let first = state().lock().map(|mut s| !std::mem::replace(&mut s.loaded_cache, true)).unwrap_or(false);
            first.then(|| load_cache(&place)).flatten()
        };
        if let Some(forecast) = cached {
            publish(forecast);
        }
        let stale = current().is_none_or(|f| now_secs().saturating_sub(f.fetched) >= TTL);
        if stale {
            if let Some((json, name)) = fetch(&place) {
                if let Some(forecast) = parse(&json, &name, now_secs()) {
                    if let Some(dir) = cache_dir() {
                        let _ = std::fs::create_dir_all(&dir);
                        let _ = std::fs::write(dir.join("weather.json"), format!("{place}\n{name}\n{json}"));
                    }
                    publish(forecast);
                }
            }
        }
        if let Ok(mut s) = state().lock() { s.busy = false; }
    });
}

fn publish(forecast: Forecast) {
    if let Ok(mut s) = state().lock() {
        s.forecast = Some(Arc::new(forecast));
        s.fresh = true;
        if let Some(tx) = &s.waker { let _ = tx.send(()); }
    }
}

fn load_cache(place: &str) -> Option<Forecast> {
    let path = cache_dir()?.join("weather.json");
    let modified = path.metadata().ok()?.modified().ok()?.duration_since(UNIX_EPOCH).ok()?.as_secs();
    let text = std::fs::read_to_string(path).ok()?;
    let mut lines = text.splitn(3, '\n');
    if lines.next()? != place { return None; }
    let name = lines.next()?.to_string();
    parse(lines.next()?, &name, modified)
}

fn curl(url: &str) -> Option<String> {
    let out = std::process::Command::new("curl").args(["-fsS", "--max-time", "8", url])
        .stdin(std::process::Stdio::null()).stderr(std::process::Stdio::null()).output().ok()?;
    out.status.success().then(|| String::from_utf8(out.stdout).ok()).flatten()
}

/// The time zone's city: Europe/Samara → "Samara".
fn timezone_city() -> Option<String> {
    let zone = std::env::var("TZ").ok().filter(|z| z.contains('/'))
        .or_else(|| std::fs::read_link("/etc/localtime").ok().map(|p| p.to_string_lossy().into_owned()))?;
    let city = zone.rsplit('/').next()?.replace('_', " ");
    (!city.is_empty() && city != "UTC").then_some(city)
}

fn percent_encode(s: &str) -> String {
    s.bytes().map(|b| match b {
        b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => (b as char).to_string(),
        _ => format!("%{b:02X}"),
    }).collect()
}

/// Where `place` is: (lat, lon, name).
fn locate(place: &str) -> Option<(f64, f64, String)> {
    if let Some((lat, lon)) = place.split_once(',') {
        if let (Ok(lat), Ok(lon)) = (lat.trim().parse::<f64>(), lon.trim().parse::<f64>()) {
            return Some((lat, lon, String::new()));
        }
    }
    let city = if place == "auto" { timezone_city()? } else { place.to_string() };
    let json = curl(&format!("https://geocoding-api.open-meteo.com/v1/search?count=1&language=ru&format=json&name={}",
        percent_encode(&city)))?;
    let value: serde_json::Value = serde_json::from_str(&json).ok()?;
    let hit = value.get("results")?.get(0)?;
    Some((hit.get("latitude")?.as_f64()?, hit.get("longitude")?.as_f64()?, hit.get("name")?.as_str()?.to_string()))
}

fn fetch(place: &str) -> Option<(String, String)> {
    let (lat, lon, name) = locate(place)?;
    let json = curl(&format!(
        "https://api.open-meteo.com/v1/forecast?latitude={lat:.4}&longitude={lon:.4}\
         &current=temperature_2m,weather_code,is_day&daily=weather_code,temperature_2m_max,temperature_2m_min\
         &timezone=auto&forecast_days=7"))?;
    Some((json, name))
}

fn parse(json: &str, place: &str, fetched: u64) -> Option<Forecast> {
    let value: serde_json::Value = serde_json::from_str(json).ok()?;
    let current = value.get("current")?;
    let daily = value.get("daily")?;
    let column = |key: &str| daily.get(key).and_then(|v| v.as_array()).cloned().unwrap_or_default();
    let (times, codes, highs, lows) = (column("time"), column("weather_code"), column("temperature_2m_max"), column("temperature_2m_min"));
    let days = times.iter().enumerate().filter_map(|(i, time)| {
        let mut parts = time.as_str()?.split('-').map(|p| p.parse::<i64>().ok());
        let date = Date { year: parts.next()?? as i32, month: parts.next()?? as u32, day: parts.next()?? as u32 };
        Some(Day { date, code: codes.get(i)?.as_u64()? as u32, high: highs.get(i)?.as_f64()?, low: lows.get(i)?.as_f64()? })
    }).collect();
    Some(Forecast {
        place: place.to_string(),
        temperature: current.get("temperature_2m")?.as_f64()?,
        code: current.get("weather_code")?.as_u64()? as u32,
        day: current.get("is_day").and_then(|v| v.as_u64()).unwrap_or(1) == 1,
        days,
        fetched,
    })
}

/// Put a forecast in place (the gallery).
#[cfg(test)]
pub fn inject(json: &str, place: &str) {
    if let Some(forecast) = parse(json, place, now_secs()) { publish(forecast); }
}

/// What the sky is doing, in words (WMO weather codes).
pub fn condition(code: u32) -> &'static str {
    match code {
        0 => "Ясно",
        1 => "Малооблачно",
        2 => "Облачно",
        3 => "Пасмурно",
        45 | 48 => "Туман",
        51..=57 => "Морось",
        61..=67 => "Дождь",
        71..=77 => "Снег",
        80..=82 => "Ливень",
        85 | 86 => "Снегопад",
        95..=99 => "Гроза",
        _ => "",
    }
}

/// "12°", "−3°" (a real minus).
pub fn degrees(t: f64) -> String {
    let t = t.round() as i64;
    if t < 0 { format!("−{}°", -t) } else { format!("{t}°") }
}

const YELLOW: (f64, f64, f64) = (1.0, 0.839, 0.039); // #FFD60A

fn sun(cr: &Context, cx: f64, cy: f64, s: f64, alpha: f64) {
    cr.set_source_rgba(YELLOW.0, YELLOW.1, YELLOW.2, alpha);
    cr.new_path();
    cr.arc(cx, cy, 0.22 * s, 0.0, std::f64::consts::TAU);
    let _ = cr.fill();
    cr.set_line_width(0.07 * s);
    cr.set_line_cap(cairo::LineCap::Round);
    for i in 0..8 {
        let a = i as f64 * std::f64::consts::TAU / 8.0;
        cr.move_to(cx + 0.32 * s * a.cos(), cy + 0.32 * s * a.sin());
        cr.line_to(cx + 0.43 * s * a.cos(), cy + 0.43 * s * a.sin());
    }
    let _ = cr.stroke();
}

fn moon(cr: &Context, cx: f64, cy: f64, s: f64, alpha: f64) {
    cr.save().unwrap();
    cr.set_fill_rule(cairo::FillRule::EvenOdd);
    cr.new_path();
    cr.arc(cx, cy, 0.3 * s, 0.0, std::f64::consts::TAU);
    cr.new_sub_path();
    cr.arc(cx + 0.17 * s, cy - 0.13 * s, 0.26 * s, 0.0, std::f64::consts::TAU);
    cr.set_source_rgba(1.0, 1.0, 1.0, 0.92 * alpha);
    let _ = cr.fill();
    cr.restore().unwrap();
}

fn cloud(cr: &Context, cx: f64, cy: f64, s: f64, shade: f64, alpha: f64) {
    // one shape: the puffs and the flat base painted as a group, so the
    // overlaps do not darken
    cr.push_group();
    cr.set_source_rgba(shade, shade, shade + 0.02, 1.0);
    for (dx, dy, r) in [(-0.17, 0.04, 0.17), (0.04, -0.06, 0.23), (0.24, 0.07, 0.15)] {
        cr.new_path();
        cr.arc(cx + dx * s, cy + dy * s, r * s, 0.0, std::f64::consts::TAU);
        let _ = cr.fill();
    }
    crate::menu_style::rounded(cr, cx - 0.34 * s, cy + 0.02 * s, 0.73 * s, 0.2 * s, 0.1 * s);
    let _ = cr.fill();
    let _ = cr.pop_group_to_source();
    let _ = cr.paint_with_alpha(alpha);
}

/// The sky's symbol for `code`, `size` across, centred on (cx, cy).
pub fn icon(cr: &Context, code: u32, day: bool, cx: f64, cy: f64, size: f64, alpha: f64) {
    cr.save().unwrap();
    let s = size;
    let teal = crate::menu_style::TEAL;
    match code {
        0 => if day { sun(cr, cx, cy, s, alpha) } else { moon(cr, cx, cy, s, alpha) },
        1 | 2 => {
            if day { sun(cr, cx - 0.14 * s, cy - 0.14 * s, 0.8 * s, alpha) } else { moon(cr, cx - 0.12 * s, cy - 0.14 * s, 0.8 * s, alpha) }
            cloud(cr, cx + 0.06 * s, cy + 0.1 * s, 0.85 * s, 0.95, alpha);
        }
        3 => cloud(cr, cx, cy, s, 0.86, alpha),
        45 | 48 => {
            cr.set_source_rgba(0.85, 0.85, 0.88, alpha);
            cr.set_line_width(0.08 * s);
            cr.set_line_cap(cairo::LineCap::Round);
            for (i, w) in [0.62, 0.8, 0.5].iter().enumerate() {
                let y = cy - 0.2 * s + i as f64 * 0.2 * s;
                cr.move_to(cx - w * s / 2.0, y);
                cr.line_to(cx + w * s / 2.0, y);
            }
            let _ = cr.stroke();
        }
        _ => {
            cloud(cr, cx, cy - 0.12 * s, s, 0.86, alpha);
            let below = cy + 0.22 * s;
            match code {
                71..=77 | 85 | 86 => {
                    cr.set_source_rgba(1.0, 1.0, 1.0, alpha);
                    for dx in [-0.18, 0.0, 0.18] {
                        cr.new_path();
                        cr.arc(cx + dx * s, below + if dx == 0.0 { 0.08 * s } else { 0.0 }, 0.045 * s, 0.0, std::f64::consts::TAU);
                        let _ = cr.fill();
                    }
                }
                95..=99 => {
                    cr.set_source_rgba(YELLOW.0, YELLOW.1, YELLOW.2, alpha);
                    cr.new_path();
                    cr.move_to(cx + 0.04 * s, below - 0.1 * s);
                    cr.line_to(cx - 0.1 * s, below + 0.08 * s);
                    cr.line_to(cx + 0.0 * s, below + 0.08 * s);
                    cr.line_to(cx - 0.06 * s, below + 0.24 * s);
                    cr.line_to(cx + 0.12 * s, below + 0.02 * s);
                    cr.line_to(cx + 0.02 * s, below + 0.02 * s);
                    cr.close_path();
                    let _ = cr.fill();
                }
                _ => {
                    // drizzle, rain, showers: slanted drops
                    cr.set_source_rgba(teal.0, teal.1, teal.2, alpha);
                    cr.set_line_width(0.065 * s);
                    cr.set_line_cap(cairo::LineCap::Round);
                    let drops = if (51..=57).contains(&code) { [-0.12, 0.12].as_slice() } else { [-0.2, 0.0, 0.2].as_slice() };
                    for dx in drops {
                        cr.move_to(cx + dx * s + 0.04 * s, below - 0.04 * s);
                        cr.line_to(cx + dx * s - 0.04 * s, below + 0.14 * s);
                    }
                    let _ = cr.stroke();
                }
            }
        }
    }
    cr.restore().unwrap();
    cr.new_path();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_open_meteo() {
        let json = r#"{"current":{"temperature_2m":11.6,"weather_code":3,"is_day":1},
            "daily":{"time":["2026-10-05","2026-10-06"],"weather_code":[3,61],
            "temperature_2m_max":[14.2,9.8],"temperature_2m_min":[5.1,-1.6]}}"#;
        let f = parse(json, "Самара", 0).unwrap();
        assert_eq!((f.temperature, f.code, f.day), (11.6, 3, true));
        assert_eq!(f.days.len(), 2);
        let tomorrow = f.on(Date { year: 2026, month: 10, day: 6 }).unwrap();
        assert_eq!((degrees(tomorrow.high), degrees(tomorrow.low)), ("10°".to_string(), "−2°".to_string()));
        assert_eq!(condition(tomorrow.code), "Дождь");
        assert_eq!(percent_encode("a b"), "a%20b");
        assert_eq!(percent_encode("Ой"), "%D0%9E%D0%B9");
    }
}
