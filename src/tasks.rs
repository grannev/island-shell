//! Live activities from anywhere: a build, a copy, a download or a backup
//! script puts a job into the island with `island-shell task …`,
//! `island-shell progress …` or `island-shell run -- <command>`. A running
//! job is a small ring beside the clock (its percentage, or the time it has
//! been running); a click unfolds the jobs into a pill; when a job ends the
//! ring turns into a tick or a cross, and a job that took a while announces
//! its end in the pill.
//!
//! Every job has an owner process (by default the shell or script that
//! started it). When the owner goes away without ending its job, the job
//! ends as interrupted, so a killed script never leaves a ring spinning.

use cairo::Context;

use crate::anim::{Spring, Time};
use crate::island::{Font, Renderer, smoothstep};
use crate::menu_style::{self, Rgb};

/// at most this many jobs at once (the oldest finished ones go first)
const LIMIT: usize = 8;
/// how long a finished job's tick or cross stays beside the clock
pub const LINGER: Time = 4.0;
/// a job that ran at least this long announces its end in the pill
pub const ANNOUNCE_AFTER: Time = 3.0;
/// the unfolded pill: width, rows shown, and their geometry
pub const PILL_W: f64 = 420.0;
const PILL_ROWS: usize = 3;
const ROW_H: f64 = 56.0;
const PAD_Y: f64 = 16.0;
const RING_R: f64 = 7.0;
/// the compact ring and the gap to its value
const CHIP_RING: f64 = 2.0 * RING_R + 3.0;
const CHIP_GAP: f64 = 6.0;

#[derive(Clone, Debug, PartialEq)]
pub enum Op {
    /// start (or restart) a job with this title
    Start { title: String },
    /// how far it has got (0..1) and/or what it is doing now
    Set { progress: Option<f64>, detail: Option<String> },
    /// `progress <title> <n>%`: start if new, set, and end at 100 %
    Progress { title: String, progress: f64 },
    Done { detail: Option<String> },
    Fail { detail: Option<String> },
    /// remove without a word
    Cancel,
    List,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Request {
    pub id: String,
    pub op: Op,
    /// the process whose exit ends the job; `None` when detached
    pub owner: Option<u32>,
}

/// `n%`, `n.n%` or a bare fraction-free percentage in 0..=100.
fn percent(word: &str) -> Option<f64> {
    let value: f64 = word.strip_suffix('%')?.replace(',', ".").parse().ok()?;
    value.is_finite().then(|| (value / 100.0).clamp(0.0, 1.0))
}

fn text(words: &[&str]) -> Option<String> {
    let joined = words.join(" ");
    let trimmed: String = joined.trim().chars().filter(|c| !c.is_control()).take(120).collect();
    (!trimmed.is_empty()).then_some(trimmed)
}

/// The words after `task`, or after `progress` when `shorthand` is set.
pub fn parse(words: &[&str], shorthand: bool) -> Option<Request> {
    let (owner, words) = match words.first().and_then(|w| w.strip_prefix("--pid=")) {
        Some(pid) => (Some(pid.parse::<u32>().ok()?).filter(|pid| *pid != 0), &words[1..]),
        None => (None, words),
    };
    if shorthand {
        let (last, title) = words.split_last()?;
        let title = text(title)?;
        return Some(Request { id: title.clone(), op: Op::Progress { title, progress: percent(last)? }, owner });
    }
    let request = |id: &str, op| Some(Request { id: id.to_string(), op, owner });
    match words {
        ["list"] => request("", Op::List),
        ["start", id, title @ ..] => {
            let title = text(title).unwrap_or_else(|| id.to_string());
            request(id, Op::Start { title })
        }
        ["set", id, rest @ ..] => {
            let (progress, rest) = match rest.split_first() {
                Some((first, rest)) if percent(first).is_some() => (percent(first), rest),
                _ => (None, rest),
            };
            let detail = text(rest);
            if progress.is_none() && detail.is_none() { return None; }
            request(id, Op::Set { progress, detail })
        }
        ["done", id, rest @ ..] => request(id, Op::Done { detail: text(rest) }),
        ["fail", id, rest @ ..] => request(id, Op::Fail { detail: text(rest) }),
        ["cancel", id] => request(id, Op::Cancel),
        _ => None,
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct End {
    pub at: Time,
    pub ok: bool,
}

#[derive(Clone, Debug)]
pub struct Task {
    pub id: String,
    pub title: String,
    pub detail: Option<String>,
    pub progress: Option<f64>,
    /// the progress as drawn: it eases toward each new value
    eased: Spring,
    pub started: Time,
    pub updated: Time,
    pub ended: Option<End>,
    pub owner: Option<u32>,
    /// distinguishes a restarted job's labels from the old ones
    serial: u64,
}

impl Task {
    fn shown(&self, now: Time) -> f64 {
        match self.ended {
            Some(_) => 1.0,
            None => self.eased.value(now).clamp(0.0, 1.0),
        }
    }

    fn colour(&self) -> Rgb {
        match self.ended {
            Some(End { ok: false, .. }) => menu_style::RED,
            Some(_) => menu_style::GREEN,
            None => menu_style::BLUE,
        }
    }

    /// The compact value: a percentage, the running time, or the outcome.
    fn value(&self, now: Time) -> String {
        match (self.ended, self.progress) {
            (Some(End { ok: true, .. }), _) => "Готово".into(),
            (Some(_), _) => "Ошибка".into(),
            (None, Some(p)) => format!("{}%", (p * 100.0).floor()),
            (None, None) => elapsed(now - self.started),
        }
    }

    /// The pill's second line.
    fn status(&self, now: Time) -> String {
        let took = elapsed(self.ended.map_or(now, |e| e.at) - self.started);
        match (&self.ended, &self.detail) {
            (Some(End { ok: true, .. }), None) => format!("Готово за {took}"),
            (Some(End { ok: false, .. }), None) => format!("Ошибка · {took}"),
            (Some(End { ok: true, .. }), Some(detail)) => format!("{detail} · {took}"),
            (Some(End { ok: false, .. }), Some(detail)) => format!("Ошибка: {detail} · {took}"),
            (None, Some(detail)) => detail.clone(),
            (None, None) => format!("Выполняется · {took}"),
        }
    }
}

/// `0:07`, `12:30`, `1:02:03`.
pub fn elapsed(seconds: f64) -> String {
    let s = seconds.max(0.0).floor() as u64;
    if s >= 3600 { format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60) } else { format!("{}:{:02}", s / 60, s % 60) }
}

/// What a request did, for the island to follow up on.
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    Changed,
    /// a job ended after running a while: worth a look in the pill
    Ended { announce: bool },
    /// the reply text for `list`
    Listed(String),
}

#[derive(Default)]
pub struct Tasks {
    list: Vec<Task>,
    serial: u64,
}

impl Tasks {
    pub fn is_empty(&self) -> bool { self.list.is_empty() }
    pub fn running(&self) -> usize { self.list.iter().filter(|t| t.ended.is_none()).count() }

    pub fn apply(&mut self, request: Request, now: Time) -> Result<Outcome, String> {
        let Request { id, op, owner } = request;
        let done = matches!(op, Op::Done { .. });
        match op {
            Op::List => {
                let lines: Vec<String> = self.list.iter().map(|t| format!("{}\t{}\t{}\t{}", t.id,
                    match t.ended { None => "running", Some(End { ok: true, .. }) => "done", Some(_) => "failed" },
                    t.progress.map_or_else(|| "-".into(), |p| format!("{:.0}%", p * 100.0)), t.title)).collect();
                Ok(Outcome::Listed(if lines.is_empty() { "no tasks".into() } else { lines.join("\n") }))
            }
            Op::Start { title } => {
                self.list.retain(|t| t.id != id);
                self.serial += 1;
                self.list.push(Task {
                    id, title, detail: None, progress: None, eased: Spring::new(0.0, 0.5, 1.0),
                    started: now, updated: now, ended: None, owner, serial: self.serial,
                });
                // too many: the oldest finished ones go, then the oldest
                while self.list.len() > LIMIT {
                    let index = self.list.iter().position(|t| t.ended.is_some()).unwrap_or(0);
                    self.list.remove(index);
                }
                Ok(Outcome::Changed)
            }
            Op::Progress { title, progress } => {
                if !self.list.iter().any(|t| t.id == id && t.ended.is_none()) {
                    self.apply(Request { id: id.clone(), op: Op::Start { title }, owner }, now)?;
                }
                self.apply(Request { id: id.clone(), op: Op::Set { progress: Some(progress), detail: None }, owner: None }, now)?;
                if progress >= 1.0 {
                    return self.apply(Request { id, op: Op::Done { detail: None }, owner: None }, now);
                }
                Ok(Outcome::Changed)
            }
            Op::Set { progress, detail } => {
                let task = self.live(&id)?;
                if let Some(p) = progress {
                    if task.progress.is_none() {
                        // a job that starts reporting: the ring fills from empty
                        task.eased = Spring::new(0.0, 0.5, 1.0);
                    }
                    task.progress = Some(p);
                    task.eased.set(p, now);
                }
                if detail.is_some() { task.detail = detail; }
                task.updated = now;
                Ok(Outcome::Changed)
            }
            Op::Done { detail } | Op::Fail { detail } => {
                let ok = done;
                let task = self.live(&id)?;
                if detail.is_some() { task.detail = detail; }
                if ok && task.progress.is_some() { task.progress = Some(1.0); }
                task.ended = Some(End { at: now, ok });
                task.updated = now;
                Ok(Outcome::Ended { announce: now - task.started >= ANNOUNCE_AFTER })
            }
            Op::Cancel => {
                let before = self.list.len();
                self.list.retain(|t| t.id != id);
                if self.list.len() == before { return Err(format!("no task {id:?}")); }
                Ok(Outcome::Changed)
            }
        }
    }

    fn live(&mut self, id: &str) -> Result<&mut Task, String> {
        self.list.iter_mut().find(|t| t.id == id && t.ended.is_none()).ok_or_else(|| format!("no running task {id:?}"))
    }

    /// The owners of running jobs, to check that they are still alive.
    pub fn owners(&self) -> Vec<(String, u32)> {
        self.list.iter().filter(|t| t.ended.is_none()).filter_map(|t| Some((t.id.clone(), t.owner?))).collect()
    }

    /// The owner of `id` exited without ending it.
    pub fn orphaned(&mut self, id: &str, now: Time) -> Option<Outcome> {
        let task = self.live(id).ok()?;
        task.detail = Some("прервано".into());
        task.ended = Some(End { at: now, ok: false });
        task.updated = now;
        Some(Outcome::Ended { announce: now - task.started >= ANNOUNCE_AFTER })
    }

    /// Finished jobs leave after their tick has been seen. True when the
    /// list changed.
    pub fn advance(&mut self, now: Time) -> bool {
        let before = self.list.len();
        self.list.retain(|t| t.ended.is_none_or(|e| now - e.at < LINGER));
        before != self.list.len()
    }

    /// The ring needs frames: it eases, or an outcome pops in.
    pub fn is_animating(&self, now: Time) -> bool {
        self.list.iter().any(|t| t.eased.is_moving(now) || t.ended.is_some_and(|e| now - e.at < 0.6))
    }

    /// An open-ended job's arc is turning (redrawn at a calm 30 Hz rather
    /// than on every frame).
    pub fn turning(&self) -> bool {
        self.primary().is_some_and(|t| t.ended.is_none() && t.progress.is_none())
    }

    /// When the next time-driven change is due (the running seconds tick,
    /// a finished job leaves).
    pub fn next_change(&self, now: Time) -> Option<Time> {
        self.list.iter().filter_map(|t| match t.ended {
            Some(e) => Some(e.at + LINGER),
            None => Some(now + 1.0 - ((now - t.started).fract())),
        }).reduce(f64::min)
    }

    /// The job the compact ring speaks for: the latest running one, else
    /// the latest finished one.
    fn primary(&self) -> Option<&Task> {
        self.list.iter().filter(|t| t.ended.is_none()).max_by(|a, b| a.updated.total_cmp(&b.updated))
            .or_else(|| self.list.iter().max_by(|a, b| a.ended.map_or(0.0, |e| e.at).total_cmp(&b.ended.map_or(0.0, |e| e.at))))
    }

    /// Width of the compact chip: the ring and its value (digits reserved
    /// so the clock never twitches as they change).
    pub fn chip_width(&self, cr: &Context, renderer: &Renderer, now: Time) -> f64 {
        let Some(task) = self.primary() else { return 0.0 };
        let value: String = format!("{}{}", task.value(now), self.more(task))
            .chars().map(|c| if c.is_ascii_digit() { '8' } else { c }).collect();
        CHIP_RING + CHIP_GAP + renderer.layout(cr, &value, Font::Small).pixel_extents().1.width() as f64
    }

    /// " +2" after the value while other jobs run too.
    fn more(&self, task: &Task) -> String {
        let others = self.running() - usize::from(task.ended.is_none());
        if others > 0 { format!("  +{others}") } else { String::new() }
    }

    /// The compact chip: a ring (filling, or an arc turning when the job
    /// does not say how far it is), a count when there are several, and
    /// the value; a finished job's ring closes into a tick or a cross.
    pub fn draw_chip(&self, cr: &Context, renderer: &Renderer, x: f64, cy: f64, alpha: f64, now: Time) {
        let Some(task) = self.primary() else { return };
        let rx = x + RING_R + 1.5;
        ring(cr, task, rx, cy, RING_R, 2.2, alpha, now);
        let label = task.value(now);
        let layout = renderer.layout(cr, &label, Font::Small);
        let (_, ext) = layout.pixel_extents();
        let tx = x + CHIP_RING + CHIP_GAP - ext.x() as f64;
        crate::motion::text(cr, renderer, &format!("task-chip{}", task.serial), &label, tx,
            cy - ext.height() as f64 / 2.0 - ext.y() as f64, ext.width() as f64 + 40.0, 0.0, Font::Small,
            (1.0, 1.0, 1.0), 0.92 * alpha);
        let more = self.more(task);
        if !more.is_empty() {
            let mx = tx + ext.x() as f64 + renderer.layout(cr, &label, Font::Small).pixel_extents().1.width() as f64;
            crate::motion::text(cr, renderer, "task-more", &more, mx, cy - ext.height() as f64 / 2.0 - ext.y() as f64,
                60.0, 0.0, Font::Small, (1.0, 1.0, 1.0), menu_style::SECONDARY * alpha);
        }
    }

    /// The unfolded pill's size for the jobs it shows.
    pub fn pill_size(&self) -> (f64, f64) {
        let rows = self.list.len().clamp(1, PILL_ROWS) as f64;
        let more = if self.list.len() > PILL_ROWS { 22.0 } else { 0.0 };
        (PILL_W, 2.0 * PAD_Y + rows * ROW_H + more)
    }

    /// The jobs, newest first: a ring well, the title and its status, the
    /// percentage on the right, a bar under a job that reports progress.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_pill(&self, cr: &Context, renderer: &Renderer, x: f64, y: f64, w: f64, alpha: f64, since: Time, now: Time) {
        let mut order: Vec<&Task> = self.list.iter().collect();
        order.sort_by(|a, b| b.started.total_cmp(&a.started));
        let title_h = menu_style::line_height(cr, renderer, Font::Headline);
        let sub_h = menu_style::line_height(cr, renderer, Font::Subhead);
        for (index, task) in order.iter().take(PILL_ROWS).enumerate() {
            // rows arrive in a wave, as in the Control Center
            let k = smoothstep((now - since - 0.04 * index as f64) / 0.3);
            let a = alpha * k;
            let top = y + PAD_Y + index as f64 * ROW_H + 5.0 * (1.0 - k);
            let mid = top + ROW_H / 2.0;
            let (ix, tx) = (x + 26.0 + 18.0, x + 26.0 + 36.0 + 12.0);
            menu_style::well(cr, ix, mid, 18.0, None, a);
            ring(cr, task, ix, mid, 10.0, 2.6, a, now);
            let value = task.progress.filter(|_| task.ended.is_none()).map(|p| format!("{}%", (p * 100.0).floor()));
            let value_w = if value.is_some() { 52.0 } else { 0.0 };
            let tw = x + w - 26.0 - value_w - tx;
            let bar = task.progress.is_some() && task.ended.is_none();
            let block = title_h + sub_h + if bar { 9.0 } else { 0.0 };
            let line1 = mid - block / 2.0;
            menu_style::label(cr, renderer, &task.title, tx, line1, tw, Font::Headline, a);
            crate::motion::text(cr, renderer, &format!("task-status{}", task.serial), &task.status(now), tx, line1 + title_h,
                tw, 0.0, Font::Subhead, (1.0, 1.0, 1.0), menu_style::SECONDARY * a);
            if let Some(value) = value {
                crate::motion::text(cr, renderer, &format!("task-value{}", task.serial), &value, x + w - 26.0 - value_w,
                    line1, value_w, 1.0, Font::Headline, (1.0, 1.0, 1.0), menu_style::SECONDARY * a);
            }
            if bar {
                let (bx, by, bw) = (tx, line1 + title_h + sub_h + 5.0, x + w - 26.0 - tx);
                menu_style::rounded(cr, bx, by, bw, 4.0, 2.0);
                cr.set_source_rgba(1.0, 1.0, 1.0, 0.16 * a);
                let _ = cr.fill();
                let fill = bw * task.shown(now);
                if fill > 1.0 {
                    let c = task.colour();
                    menu_style::rounded(cr, bx, by, fill, 4.0, 2.0);
                    cr.set_source_rgba(c.0, c.1, c.2, a);
                    let _ = cr.fill();
                }
            }
        }
        if self.list.len() > PILL_ROWS {
            let more = format!("Ещё {}", self.list.len() - PILL_ROWS);
            menu_style::text(cr, renderer, &more, x + 26.0 + 36.0 + 12.0, y + PAD_Y + PILL_ROWS as f64 * ROW_H,
                w - 120.0, Font::Footnote, (1.0, 1.0, 1.0), menu_style::TERTIARY * alpha);
        }
    }
}

/// A job's ring: a track, the filled share (or a turning arc), and on
/// its end a full ring in the outcome's colour with a tick or a cross that
/// pops in.
#[allow(clippy::too_many_arguments)]
fn ring(cr: &Context, task: &Task, cx: f64, cy: f64, r: f64, line: f64, alpha: f64, now: Time) {
    use std::f64::consts::{FRAC_PI_2, TAU};
    cr.save().unwrap();
    cr.new_path();
    cr.set_line_cap(cairo::LineCap::Round);
    cr.set_line_join(cairo::LineJoin::Round);
    cr.set_line_width(line);
    cr.set_source_rgba(1.0, 1.0, 1.0, 0.18 * alpha);
    cr.arc(cx, cy, r, 0.0, TAU);
    let _ = cr.stroke();
    let c = task.colour();
    cr.set_source_rgba(c.0, c.1, c.2, alpha);
    match (task.ended, task.progress) {
        (Some(end), _) => {
            // the ring closes, then the symbol pops (overshooting a touch)
            let t = now - end.at;
            let close = smoothstep(t / 0.25);
            let from = if task.progress.is_some() { task.eased.value(end.at).clamp(0.0, 1.0) } else { 0.0 };
            let share = from + (1.0 - from) * close;
            cr.arc(cx, cy, r, -FRAC_PI_2, -FRAC_PI_2 + TAU * share);
            let _ = cr.stroke();
            let pop = menu_style::bounce_scale(t - 0.12).max(0.0) * smoothstep((t - 0.12) / 0.12);
            if pop > 0.0 {
                let s = r * 0.5 * pop;
                cr.set_line_width(line * 0.95);
                if end.ok {
                    cr.move_to(cx - s, cy + 0.05 * s);
                    cr.line_to(cx - 0.3 * s, cy + 0.7 * s);
                    cr.line_to(cx + s, cy - 0.65 * s);
                } else {
                    cr.move_to(cx - 0.7 * s, cy - 0.7 * s);
                    cr.line_to(cx + 0.7 * s, cy + 0.7 * s);
                    cr.move_to(cx + 0.7 * s, cy - 0.7 * s);
                    cr.line_to(cx - 0.7 * s, cy + 0.7 * s);
                }
                let _ = cr.stroke();
            }
        }
        (None, Some(_)) => {
            let share = task.shown(now);
            if share > 0.002 {
                cr.arc(cx, cy, r, -FRAC_PI_2, -FRAC_PI_2 + TAU * share);
                let _ = cr.stroke();
            }
        }
        (None, None) => {
            // no percentage: an arc turns, once a second and a bit
            let turn = (now - task.started) * 0.85 * TAU;
            cr.arc(cx, cy, r, turn, turn + TAU * 0.28);
            let _ = cr.stroke();
        }
    }
    cr.restore().unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(line: &str) -> Option<Request> {
        let words: Vec<&str> = line.split_whitespace().collect();
        match words.split_first() {
            Some((&"task", rest)) => parse(rest, false),
            Some((&"progress", rest)) => parse(rest, true),
            _ => None,
        }
    }

    #[test]
    fn parses_task_commands() {
        assert_eq!(request("task --pid=42 start build Сборка проекта"),
            Some(Request { id: "build".into(), op: Op::Start { title: "Сборка проекта".into() }, owner: Some(42) }));
        assert_eq!(request("task set build 40% linking"),
            Some(Request { id: "build".into(), op: Op::Set { progress: Some(0.4), detail: Some("linking".into()) }, owner: None }));
        assert_eq!(request("task set build 12,5%").map(|r| r.op), Some(Op::Set { progress: Some(0.125), detail: None }));
        assert_eq!(request("task set build"), None);
        assert_eq!(request("task fail build exit 2").map(|r| r.op), Some(Op::Fail { detail: Some("exit 2".into()) }));
        assert_eq!(request("progress --pid=0 Копирование фото 150%"),
            Some(Request { id: "Копирование фото".into(), op: Op::Progress { title: "Копирование фото".into(), progress: 1.0 }, owner: None }));
        assert_eq!(request("progress Копирование"), None);
    }

    #[test]
    fn a_job_runs_ends_and_leaves() {
        let mut tasks = Tasks::default();
        let start = |id: &str| Request { id: id.into(), op: Op::Start { title: id.into() }, owner: Some(7) };
        tasks.apply(start("a"), 1.0).unwrap();
        assert!(tasks.apply(Request { id: "b".into(), op: Op::Done { detail: None }, owner: None }, 1.0).is_err());
        assert_eq!(tasks.primary().unwrap().value(66.5), "1:05");
        let set = Request { id: "a".into(), op: Op::Set { progress: Some(0.4), detail: None }, owner: None };
        tasks.apply(set, 2.0).unwrap();
        assert_eq!(tasks.primary().unwrap().value(2.0), "40%");
        assert_eq!(tasks.owners(), vec![("a".to_string(), 7)]);
        assert_eq!(tasks.orphaned("a", 5.0), Some(Outcome::Ended { announce: true }));
        assert_eq!(tasks.primary().unwrap().status(5.0), "Ошибка: прервано · 0:04");
        assert!(tasks.owners().is_empty());
        assert!(!tasks.advance(5.0 + LINGER - 0.1));
        assert!(tasks.advance(5.0 + LINGER));
        assert!(tasks.is_empty());
        // a quick job ends without a fuss
        let progress = |p| Request { id: "copy".into(), op: Op::Progress { title: "Copy".into(), progress: p }, owner: None };
        assert_eq!(tasks.apply(progress(0.5), 10.0), Ok(Outcome::Changed));
        assert_eq!(tasks.apply(progress(1.0), 11.0), Ok(Outcome::Ended { announce: false }));
        assert_eq!(tasks.primary().unwrap().value(11.0), "Готово");
    }
}
