//! Content transitions for text, after SwiftUI's `.contentTransition`:
//! when a label's text changes, digits that changed roll (up when the
//! number grows, down when it shrinks — a countdown rolls down) while the
//! rest of the label stays put and its tail slides to the new width;
//! words that change are replaced by the new word rising in as the old
//! one lifts away and fades.
//!
//! Labels are tracked by key in a registry the Renderer owns. Keys carry
//! an instance (a sheet's opening time, an OSD's entrance), so a label
//! seen for the first time just appears; only a change on screen moves.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;

use cairo::Context;

use crate::anim::{Time, min_jerk};
use crate::island::{Font, Renderer, smoothstep};

pub const DURATION: Time = 0.42;
const LIMIT: usize = 160;

struct Slot {
    text: String,
    previous: Option<String>,
    since: Time,
    seen: Time,
    /// the new text comes up from below (a growing number, a new word)
    rising: bool,
}

#[derive(Default)]
pub struct Registry {
    slots: RefCell<HashMap<String, Slot>>,
    now: Cell<Time>,
}

impl Registry {
    /// The time of the frame being drawn.
    pub fn set_time(&self, now: Time) {
        self.now.set(now);
    }

    /// Is any label still in transition?
    pub fn moving(&self, now: Time) -> bool {
        self.slots.borrow().values().any(|s| s.previous.is_some() && now - s.since < DURATION - 1e-9)
    }

    /// Note `text` under `key`; returns the text it is leaving, if it is
    /// still on its way out, the progress 0..1 and the direction.
    fn observe(&self, key: &str, text: &str) -> (Option<String>, f64, bool) {
        let now = self.now.get();
        let mut slots = self.slots.borrow_mut();
        if slots.len() > LIMIT && !slots.contains_key(key) {
            // forget the labels of sheets long closed
            let mut seen: Vec<Time> = slots.values().map(|s| s.seen).collect();
            seen.sort_by(f64::total_cmp);
            let cut = seen[seen.len() / 2];
            slots.retain(|_, s| s.seen > cut);
        }
        let slot = slots.entry(key.to_string()).or_insert_with(|| Slot {
            text: text.to_string(),
            previous: None,
            since: now,
            seen: now,
            rising: true,
        });
        if slot.text != text {
            slot.rising = rising(&slot.text, text);
            slot.previous = Some(std::mem::replace(&mut slot.text, text.to_string()));
            slot.since = now;
        }
        slot.seen = now;
        let k = ((now - slot.since) / DURATION).clamp(0.0, 1.0);
        if k >= 1.0 - 1e-9 {
            slot.previous = None;
        }
        (slot.previous.clone(), k, slot.rising)
    }
}

/// A number counts up from below; anything else arrives from below too.
fn rising(old: &str, new: &str) -> bool {
    let number = |s: &str| s.chars().filter(char::is_ascii_digit).collect::<String>().parse::<u64>().ok();
    match (number(old), number(new)) {
        (Some(a), Some(b)) => b >= a,
        _ => true,
    }
}

/// The longest shared head and tail (in chars) of two strings, the tail
/// never overlapping the head.
fn common(old: &[char], new: &[char]) -> (usize, usize) {
    let head = old.iter().zip(new).take_while(|(a, b)| a == b).count();
    let room = old.len().min(new.len()) - head;
    let tail = old.iter().rev().zip(new.iter().rev()).take(room).take_while(|(a, b)| a == b).count();
    (head, tail)
}

fn width(cr: &Context, renderer: &Renderer, s: &str, font: Font) -> f64 {
    if s.is_empty() {
        return 0.0;
    }
    renderer.layout(cr, s, font).pixel_extents().1.width() as f64
}

fn show(cr: &Context, renderer: &Renderer, s: &str, x: f64, y: f64, font: Font, colour: (f64, f64, f64), alpha: f64, snap: bool) {
    if s.is_empty() || alpha <= 0.002 {
        return;
    }
    let layout = renderer.layout(cr, s, font);
    cr.set_source_rgba(colour.0, colour.1, colour.2, alpha.clamp(0.0, 1.0));
    if snap {
        cr.move_to(renderer.snap(x), renderer.snap(y));
    } else {
        cr.move_to(x, y);
    }
    pangocairo::functions::show_layout(cr, &layout);
    cr.new_path();
}

/// Draw `text` in the box (x, y, w) — y is the top of the line box,
/// `align` 0 left, 0.5 centre, 1 right — animating any change since the
/// last frame that drew `key`.
#[allow(clippy::too_many_arguments)]
pub fn text(
    cr: &Context, renderer: &Renderer, key: &str, text: &str, x: f64, y: f64, w: f64, align: f64,
    font: Font, colour: (f64, f64, f64), alpha: f64,
) {
    let (previous, k, rising) = renderer.motion.observe(key, text);
    let Some(old) = previous else {
        let layout = renderer.layout_ellipsized(cr, text, font, w.max(4.0), 1);
        let tw = layout.pixel_extents().1.width() as f64;
        cr.set_source_rgba(colour.0, colour.1, colour.2, alpha.clamp(0.0, 1.0));
        cr.move_to(renderer.snap(x + (w - tw).max(0.0) * align), renderer.snap(y));
        pangocairo::functions::show_layout(cr, &layout);
        cr.new_path();
        return;
    };
    let e = min_jerk(k);
    let travel = renderer.layout(cr, "Ag", font).pixel_extents().1.height() as f64 * 0.55;
    let dir = if rising { 1.0 } else { -1.0 };
    // the leaving text is gone before the arriving one is fully there
    let out_alpha = alpha * (1.0 - smoothstep(k / 0.42));
    let in_alpha = alpha * smoothstep((k - 0.18) / 0.62);
    let out_y = y - dir * travel * e;
    let in_y = y + dir * travel * (1.0 - e);

    let old_chars: Vec<char> = old.chars().collect();
    let new_chars: Vec<char> = text.chars().collect();
    let (head, tail) = common(&old_chars, &new_chars);
    let mid_old: String = old_chars[head..old_chars.len() - tail].iter().collect();
    let mid_new: String = new_chars[head..new_chars.len() - tail].iter().collect();
    // a time's separators roll with its digits ("9:59" → "10:00")
    let numeric = |s: &str| s.chars().all(|c| c.is_ascii_digit() || c == ':' || c == '.');
    if numeric(&mid_old) && numeric(&mid_new) && !(mid_old.is_empty() && mid_new.is_empty()) {
        // numeric: only the digits that changed roll
        let prefix: String = new_chars[..head].iter().collect();
        let suffix: String = new_chars[new_chars.len() - tail..].iter().collect();
        let wp = width(cr, renderer, &prefix, font);
        let (wo, wn) = (width(cr, renderer, &mid_old, font), width(cr, renderer, &mid_new, font));
        let ws = width(cr, renderer, &suffix, font);
        let wm = wo + (wn - wo) * e;
        let x0 = x + (w - (wp + wm + ws)).max(0.0) * align;
        show(cr, renderer, &prefix, x0, y, font, colour, alpha, true);
        show(cr, renderer, &mid_old, x0 + wp + (wm - wo) * 0.5, out_y, font, colour, out_alpha, false);
        show(cr, renderer, &mid_new, x0 + wp + (wm - wn) * 0.5, in_y, font, colour, in_alpha, false);
        show(cr, renderer, &suffix, x0 + wp + wm, y, font, colour, alpha, false);
    } else {
        // words: the new one rises in as the old lifts away
        for (s, sy, a) in [(old.as_str(), out_y, out_alpha), (text, in_y, in_alpha)] {
            if a <= 0.002 {
                continue;
            }
            let layout = renderer.layout_ellipsized(cr, s, font, w.max(4.0), 1);
            let tw = layout.pixel_extents().1.width() as f64;
            cr.set_source_rgba(colour.0, colour.1, colour.2, a);
            cr.move_to(x + (w - tw).max(0.0) * align, sy);
            pangocairo::functions::show_layout(cr, &layout);
            cr.new_path();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digits_roll_in_the_number_s_direction() {
        assert!(rising("3 новых", "4 новых"));
        assert!(!rising("24:58", "24:57"));
        assert!(rising("Вкл.", "Выкл."));
        let c = |s: &str| s.chars().collect::<Vec<_>>();
        assert_eq!(common(&c("24:58"), &c("24:57")), (4, 0));
        assert_eq!(common(&c("9:59 AM"), &c("10:00 AM")), (0, 3));
        assert_eq!(common(&c("aa"), &c("aaa")), (2, 0));
    }

    #[test]
    fn a_label_moves_only_when_it_changes_on_screen() {
        let r = Registry::default();
        r.set_time(1.0);
        assert!(r.observe("k", "A").0.is_none(), "first sight just appears");
        r.set_time(2.0);
        let (old, k, _) = r.observe("k", "B");
        assert_eq!((old.as_deref(), k), (Some("A"), 0.0));
        assert!(r.moving(2.1));
        r.set_time(2.0 + DURATION);
        assert!(r.observe("k", "B").0.is_none());
        assert!(!r.moving(2.0 + DURATION));
    }
}
