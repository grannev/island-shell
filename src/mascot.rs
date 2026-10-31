//! The island's companion, after Vorssaint's mascot (redrawn from how it
//! behaves, not from its code): a small coral capsule with two soft black
//! eyes and no mouth that lives at the left end of the closed line.
//!
//! It blinks now and then, glances about, follows the pointer over the
//! island, bobs to the music's bass, and answers what the island sees with
//! a short reaction: a startled hop for a notification, smiling eyes for a
//! match or a song starting, a head shake for a failed Touch ID, two hops
//! when a timer runs out, a hard blink for a screenshot. When the seat goes
//! idle (ext-idle-notify) it falls asleep; it wakes with a stretch when
//! you are back. A click pokes it.
//!
//! Everything is drawn as vectors around (cx, cy) so every part can
//! squash, lean and morph; the island only asks for frames while something
//! moves (a blink is ~10 frames every few seconds).

use cairo::Context;

use crate::anim::{Spring, Time};

/// Room the companion takes on the line, and the gap after it.
pub const WIDTH: f64 = 20.0;
pub const GAP: f64 = 8.0;
const BODY_W: f64 = 18.0;
const BODY_H: f64 = 15.5;
/// The body's lit top and shaded bottom.
const LIT: (f64, f64, f64) = (1.0, 0.745, 0.640);
const SHADE: (f64, f64, f64) = (0.925, 0.490, 0.410);
/// Its colour as one swatch (the Control Center's tile).
pub const COLOUR: (f64, f64, f64) = (0.965, 0.580, 0.495);
const INK: (f64, f64, f64) = (0.086, 0.078, 0.082);

const BLINK: f64 = 0.16;
/// Asleep, a "z" floats up now and then for this long, then it just sleeps.
const DREAM_FOR: f64 = 90.0;
const Z_EVERY: f64 = 4.5;
const Z_RISE: f64 = 1.8;

/// A short answer to something the island saw.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reaction {
    /// a notification, the microphone opening: wide eyes and a hop
    Startle,
    /// a match, a song starting, power plugged in: smiling eyes, a bounce
    Happy,
    /// a timer ran out, a download or a job finished: two hops
    Cheer,
    /// a screenshot: a hard blink, as at a camera's flash
    Flash,
    /// something failed: a head shake with drooping eyes
    Shake,
    /// back from sleep: a stretch, eyes opening
    Wake,
    /// a timer started: a determined little nod
    Nod,
    /// clicked: it squashes and springs back, pleased
    Poke,
}

impl Reaction {
    fn duration(self) -> f64 {
        match self {
            Reaction::Startle => 0.6,
            Reaction::Happy => 1.5,
            Reaction::Cheer => 1.1,
            Reaction::Flash => 0.4,
            Reaction::Shake => 0.75,
            Reaction::Wake => 1.0,
            Reaction::Nod => 0.6,
            Reaction::Poke => 0.9,
        }
    }
}

/// How the companion looks at one instant (the result of all its motions).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pose {
    /// lift above its resting place, px (positive is up)
    pub lift: f64,
    pub scale_x: f64,
    pub scale_y: f64,
    /// lean, radians, clockwise
    pub lean: f64,
    /// where the eyes look, -1..1 each way
    pub look: (f64, f64),
    /// 1 open, 0 shut
    pub open: f64,
    /// 0 plain, 1 smiling (^ ^)
    pub smile: f64,
    /// how big the eyes are (wide with surprise > 1)
    pub eye_scale: f64,
    /// 0 awake, 1 asleep
    pub asleep: f64,
    /// a floating "z": how far through its rise (None: none showing)
    pub z: Option<f64>,
}

pub struct Mascot {
    look_x: Spring,
    look_y: Spring,
    asleep: Spring,
    rng: u64,
    next_blink: Time,
    blink: Option<(Time, bool)>,
    next_glance: Time,
    reaction: Option<(Reaction, Time)>,
    pointer: bool,
    idle: bool,
    slept_at: Time,
}

fn smooth(x: f64) -> f64 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

/// One hop: up and back down over `u` in 0..1, with a squash before the
/// take-off and on landing.
fn hop(u: f64, height: f64) -> (f64, f64) {
    if !(0.0..1.0).contains(&u) { return (0.0, 1.0); }
    let crouch = 0.18;
    if u < crouch {
        let k = (std::f64::consts::PI * u / crouch).sin();
        (0.0, 1.0 - 0.14 * k)
    } else if u < 0.82 {
        let k = (u - crouch) / (0.82 - crouch);
        let lift = height * (std::f64::consts::PI * k).sin();
        // stretched on the way up, round at the top
        (lift, 1.0 + 0.10 * (1.0 - 2.0 * k).max(0.0))
    } else {
        let k = ((u - 0.82) / 0.18 * std::f64::consts::PI).sin();
        (0.0, 1.0 - 0.12 * k)
    }
}

impl Mascot {
    pub fn new(now: Time) -> Self {
        let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0x9E37_79B9, |d| d.as_nanos() as u64);
        let mut m = Mascot {
            look_x: Spring::new(0.0, 0.32, 0.72),
            look_y: Spring::new(0.0, 0.32, 0.72),
            asleep: Spring::new(0.0, 0.9, 1.0),
            rng: seed | 1,
            next_blink: now + 2.0,
            blink: None,
            next_glance: now + 5.0,
            reaction: None,
            pointer: false,
            idle: false,
            slept_at: 0.0,
        };
        m.next_blink = now + m.uniform(1.5, 4.0);
        m
    }

    fn uniform(&mut self, lo: f64, hi: f64) -> f64 {
        // xorshift64*
        self.rng ^= self.rng >> 12;
        self.rng ^= self.rng << 25;
        self.rng ^= self.rng >> 27;
        let r = self.rng.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 11;
        lo + (hi - lo) * (r as f64 / (1u64 << 53) as f64)
    }

    /// The pointer over the island, relative to the companion's middle
    /// (None: it left).
    pub fn look_at(&mut self, at: Option<(f64, f64)>, now: Time) -> bool {
        let (tx, ty) = match at {
            Some((dx, dy)) => ((dx / 90.0).clamp(-1.0, 1.0), (dy / 30.0).clamp(-1.0, 1.0)),
            None => (0.0, 0.0),
        };
        self.pointer = at.is_some();
        let moved = (self.look_x.target() - tx).abs() > 0.02 || (self.look_y.target() - ty).abs() > 0.02;
        if moved && !self.idle {
            self.look_x.set(tx, now);
            self.look_y.set(ty, now);
        }
        moved && !self.idle
    }

    pub fn react(&mut self, reaction: Reaction, now: Time) {
        if self.idle && !matches!(reaction, Reaction::Poke | Reaction::Wake) { return; }
        // a reaction in full swing is not cut short by a smaller one
        if let Some((current, since)) = self.reaction {
            let busy = now - since < current.duration() * 0.6;
            if busy && current != reaction && matches!(reaction, Reaction::Nod | Reaction::Flash) { return; }
        }
        if reaction == Reaction::Poke && self.idle {
            // poked awake: it gets up, not just squashed
            self.set_idle(false, now);
            return;
        }
        self.reaction = Some((reaction, now));
        // a reaction looks up toward what happened, then lets go
        if matches!(reaction, Reaction::Startle | Reaction::Cheer) && !self.pointer {
            self.look_y.set(-0.8, now);
            self.next_glance = now + reaction.duration();
        }
    }

    /// The seat went idle (it falls asleep) or came back (it wakes).
    pub fn set_idle(&mut self, idle: bool, now: Time) {
        if idle == self.idle { return; }
        self.idle = idle;
        if idle {
            self.slept_at = now;
            self.reaction = None;
            self.blink = None;
            self.look_x.set(0.0, now);
            self.look_y.set(0.25, now);
            self.asleep.set(1.0, now);
        } else {
            self.asleep.set(0.0, now);
            self.look_y.set(0.0, now);
            self.reaction = Some((Reaction::Wake, now));
            self.next_blink = now + 1.2;
        }
    }

    /// Time-driven changes: a blink coming due, an idle glance. True when
    /// something started moving.
    pub fn advance(&mut self, now: Time) -> bool {
        let mut changed = false;
        if self.reaction.is_some_and(|(r, since)| now - since >= r.duration()) {
            self.reaction = None;
            if !self.pointer && !self.idle {
                self.look_y.set(0.0, now);
            }
            changed = true;
        }
        if self.blink.is_some_and(|(since, double)| now - since >= if double { 2.4 * BLINK } else { BLINK }) {
            self.blink = None;
        }
        if self.idle { return changed; }
        if now >= self.next_blink {
            let double = self.uniform(0.0, 1.0) < 0.18;
            self.blink = Some((now, double));
            self.next_blink = now + self.uniform(2.6, 6.8);
            changed = true;
        }
        if now >= self.next_glance {
            if !self.pointer && self.reaction.is_none() {
                // a look about: aside, back to the front, now and then up
                let pick = self.uniform(0.0, 1.0);
                let (x, y) = if self.look_x.target().abs() > 0.1 || self.look_y.target().abs() > 0.1 {
                    (0.0, 0.0)
                } else if pick < 0.4 {
                    (-0.85, 0.1)
                } else if pick < 0.8 {
                    (0.85, 0.1)
                } else {
                    (0.2, -0.7)
                };
                self.look_x.set(x, now);
                self.look_y.set(y, now);
                changed = true;
            }
            let back = self.look_x.target().abs() > 0.1 || self.look_y.target().abs() > 0.1;
            self.next_glance = now + if back { self.uniform(0.9, 2.2) } else { self.uniform(5.0, 11.0) };
        }
        changed
    }

    /// When something is next due (a blink, a glance, a dreamt "z").
    pub fn deadline(&self, now: Time) -> Option<Time> {
        if self.idle {
            let dreaming = now - self.slept_at;
            if dreaming < DREAM_FOR {
                let next = self.slept_at + ((dreaming / Z_EVERY).floor() + 1.0) * Z_EVERY;
                return Some(next);
            }
            return None;
        }
        let mut next = self.next_blink.min(self.next_glance);
        if let Some((r, since)) = self.reaction { next = next.min(since + r.duration()); }
        Some(next.max(now))
    }

    pub fn is_animating(&self, now: Time) -> bool {
        self.look_x.is_moving(now) || self.look_y.is_moving(now) || self.asleep.is_moving(now)
            || self.blink.is_some_and(|(since, double)| now - since < if double { 2.4 * BLINK } else { BLINK })
            || self.reaction.is_some_and(|(r, since)| now - since < r.duration())
            || self.z_phase(now).is_some()
    }

    fn z_phase(&self, now: Time) -> Option<f64> {
        if !self.idle { return None; }
        let dreaming = now - self.slept_at - 1.0;
        if !(0.0..DREAM_FOR).contains(&dreaming) { return None; }
        let u = (dreaming % Z_EVERY) / Z_RISE;
        (u < 1.0).then_some(u)
    }

    /// Everything the companion's motions add up to at `now`; `beat` is the
    /// music's bass (0 when nothing plays).
    pub fn pose(&self, beat: f64, now: Time) -> Pose {
        let asleep = self.asleep.value(now).clamp(0.0, 1.0);
        let mut pose = Pose {
            lift: 0.0,
            scale_x: 1.0,
            scale_y: 1.0,
            lean: 0.07 * self.look_x.value(now),
            look: (self.look_x.value(now), self.look_y.value(now)),
            open: 1.0,
            smile: 0.0,
            eye_scale: 1.0,
            asleep,
            z: self.z_phase(now),
        };
        if let Some((since, double)) = self.blink {
            let t = now - since;
            let one = |t: f64| if (0.0..BLINK).contains(&t) { 1.0 - (std::f64::consts::PI * t / BLINK).sin() } else { 1.0 };
            pose.open = one(t).min(if double { one(t - 1.4 * BLINK) } else { 1.0 });
        }
        // asleep it breathes no more than it has to: eyes shut, a little lower
        pose.open *= 1.0 - asleep;
        pose.scale_y *= 1.0 - 0.06 * asleep;
        pose.scale_x *= 1.0 + 0.04 * asleep;
        // the bass lifts and squashes it a touch
        let beat = beat.clamp(0.0, 1.0) * (1.0 - asleep);
        pose.lift += 2.2 * beat;
        pose.scale_y *= 1.0 + 0.05 * beat;
        pose.scale_x *= 1.0 - 0.03 * beat;
        if let Some((reaction, since)) = self.reaction {
            let t = now - since;
            let u = (t / reaction.duration()).clamp(0.0, 1.0);
            let fade = 1.0 - smooth((u - 0.7) / 0.3);
            match reaction {
                Reaction::Startle => {
                    let (lift, sy) = hop(t / 0.5, 6.0);
                    pose.lift += lift;
                    pose.scale_y *= sy;
                    pose.scale_x *= 2.0 - sy;
                    pose.eye_scale = 1.0 + 0.38 * smooth(t / 0.08) * fade;
                }
                Reaction::Happy => {
                    pose.smile = smooth(t / 0.15) * fade;
                    let bounce = (std::f64::consts::TAU * 2.0 * u).sin().abs() * (1.0 - u);
                    pose.lift += 2.5 * bounce;
                }
                Reaction::Cheer => {
                    let (a, sa) = hop(t / 0.5, 6.0);
                    let (b, sb) = hop((t - 0.5) / 0.5, 4.5);
                    pose.lift += a + b;
                    pose.scale_y *= sa * sb;
                    pose.scale_x *= 2.0 - sa * sb;
                    pose.smile = smooth(t / 0.12) * fade;
                }
                Reaction::Flash => {
                    let shut = 1.0 - smooth((t - 0.12) / 0.25);
                    pose.open = pose.open.min(1.0 - shut);
                    pose.scale_y *= 1.0 - 0.08 * shut;
                    pose.scale_x *= 1.0 + 0.05 * shut;
                }
                Reaction::Shake => {
                    pose.lean += 0.28 * (3.0 * std::f64::consts::TAU * u).sin() * (1.0 - u);
                    pose.open = pose.open.min(1.0 - 0.4 * fade);
                    pose.look.1 = pose.look.1.max(0.6 * fade);
                }
                Reaction::Wake => {
                    // a stretch: tall and thin, then settling, eyes coming open
                    let s = (std::f64::consts::PI * smooth(u / 0.6)).sin();
                    pose.scale_y *= 1.0 + 0.16 * s;
                    pose.scale_x *= 1.0 - 0.08 * s;
                    pose.open *= smooth((t - 0.3) / 0.35);
                }
                Reaction::Nod => {
                    let n = (std::f64::consts::TAU * u).sin().max(0.0) * (1.0 - u);
                    pose.lift -= 2.0 * n;
                    pose.lean += 0.05 * n;
                    pose.open = pose.open.min(1.0 - 0.3 * fade);
                }
                Reaction::Poke => {
                    // a damped squash, like a jelly sweet pressed
                    let s = (-5.0 * t).exp() * (std::f64::consts::TAU * 2.6 * t).cos();
                    pose.scale_y *= 1.0 - 0.18 * s;
                    pose.scale_x *= 1.0 + 0.12 * s;
                    pose.smile = smooth((t - 0.08) / 0.12) * fade;
                }
            }
        }
        pose
    }

    pub fn draw(&self, cr: &Context, cx: f64, cy: f64, alpha: f64, beat: f64, now: Time) {
        draw_pose(cr, cx, cy, alpha, &self.pose(beat, now));
    }
}

/// The body's outline, a soft capsule standing on its flat-ish bottom,
/// centred on (0, 0).
fn body_path(cr: &Context, w: f64, h: f64) {
    let (hw, hh) = (w / 2.0, h / 2.0);
    // rounder on top than below, so it sits
    let top = hw.min(hh) * 0.98;
    let bottom = hw.min(hh) * 0.72;
    let k = 0.5523;
    cr.new_path();
    cr.move_to(-hw + top, -hh);
    cr.line_to(hw - top, -hh);
    cr.curve_to(hw - top + k * top, -hh, hw, -hh + top - k * top, hw, -hh + top);
    cr.line_to(hw, hh - bottom);
    cr.curve_to(hw, hh - bottom + k * bottom, hw - bottom + k * bottom, hh, hw - bottom, hh);
    cr.line_to(-hw + bottom, hh);
    cr.curve_to(-hw + bottom - k * bottom, hh, -hw, hh - bottom + k * bottom, -hw, hh - bottom);
    cr.line_to(-hw, -hh + top);
    cr.curve_to(-hw, -hh + top - k * top, -hw + top - k * top, -hh, -hw + top, -hh);
    cr.close_path();
}

/// Draw a pose with the companion's resting middle at (cx, cy).
pub fn draw_pose(cr: &Context, cx: f64, cy: f64, alpha: f64, pose: &Pose) {
    if alpha <= 0.0 { return; }
    let (w, h) = (BODY_W * pose.scale_x, BODY_H * pose.scale_y);
    // it squashes and stretches about its feet, which stay on the floor
    let floor = cy + BODY_H / 2.0;
    cr.save().unwrap();
    cr.translate(cx, floor - pose.lift);
    cr.rotate(pose.lean);
    cr.translate(0.0, -h / 2.0);
    body_path(cr, w, h);
    let gradient = cairo::LinearGradient::new(0.0, -h / 2.0, 0.0, h / 2.0);
    gradient.add_color_stop_rgba(0.0, LIT.0, LIT.1, LIT.2, alpha);
    gradient.add_color_stop_rgba(1.0, SHADE.0, SHADE.1, SHADE.2, alpha);
    let _ = cr.set_source(&gradient);
    let _ = cr.fill();
    // a soft highlight on the upper left of its curve
    cr.save().unwrap();
    cr.translate(-w * 0.2, -h * 0.26);
    cr.rotate(-0.5);
    cr.scale(w * 0.17, h * 0.1);
    cr.new_path();
    cr.arc(0.0, 0.0, 1.0, 0.0, std::f64::consts::TAU);
    cr.restore().unwrap();
    cr.set_source_rgba(1.0, 1.0, 1.0, 0.38 * alpha);
    let _ = cr.fill();

    // the eyes
    let (lx, ly) = pose.look;
    let spread = 3.6 * pose.scale_x;
    let ey = -0.8 + 1.3 * ly + 0.9 * pose.asleep;
    let (ew, eh) = (2.5 * pose.eye_scale, 4.2 * pose.eye_scale);
    let smile = pose.smile.clamp(0.0, 1.0);
    let open = (pose.open * (1.0 - smile)).clamp(0.0, 1.0);
    // an open eye is an upright oval; nearly shut it becomes a line, which
    // bends up into a smile (^) or down, asleep (‿)
    let shut = smooth((0.42 - open) / 0.3);
    for side in [-1.0, 1.0] {
        let ex = side * spread + 2.0 * lx;
        if shut < 1.0 {
            let hgt = (eh * open).max(ew * 0.55);
            cr.save().unwrap();
            cr.translate(ex, ey);
            cr.scale(ew / 2.0, hgt / 2.0);
            cr.new_path();
            cr.arc(0.0, 0.0, 1.0, 0.0, std::f64::consts::TAU);
            cr.restore().unwrap();
            cr.set_source_rgba(INK.0, INK.1, INK.2, 0.94 * alpha * (1.0 - shut));
            let _ = cr.fill();
        }
        if shut > 0.0 {
            let half = 1.75 * pose.eye_scale.min(1.2);
            let bend = 1.6 - 3.4 * smile;
            let ends = 0.2 + 0.9 * smile;
            cr.new_path();
            cr.move_to(ex - half, ey + ends);
            cr.curve_to(ex - half * 0.5, ey + ends + bend, ex + half * 0.5, ey + ends + bend, ex + half, ey + ends);
            cr.set_line_width(1.35);
            cr.set_line_cap(cairo::LineCap::Round);
            cr.set_source_rgba(INK.0, INK.1, INK.2, 0.94 * alpha * shut);
            let _ = cr.stroke();
        }
    }
    cr.restore().unwrap();

    // asleep, a small "z" drifts up off its head
    if let Some(u) = pose.z {
        let a = alpha * (smooth(u / 0.2) * (1.0 - smooth((u - 0.6) / 0.4)));
        let (zx, zy) = (cx + BODY_W / 2.0 + 1.0 + 3.0 * u, cy - BODY_H / 2.0 - 6.0 * u);
        let s = 2.2 + 1.2 * u;
        cr.new_path();
        cr.move_to(zx - s / 2.0, zy - s / 2.0);
        cr.line_to(zx + s / 2.0, zy - s / 2.0);
        cr.line_to(zx - s / 2.0, zy + s / 2.0);
        cr.line_to(zx + s / 2.0, zy + s / 2.0);
        cr.set_line_width(1.1);
        cr.set_line_join(cairo::LineJoin::Round);
        cr.set_line_cap(cairo::LineCap::Round);
        cr.set_source_rgba(1.0, 1.0, 1.0, 0.75 * a);
        let _ = cr.stroke();
    }
}

/// A tiny face for the Control Center's tile: the body's outline and two
/// eyes, in the current source colour, about 24 px tall at scale 1.
pub fn symbol(cr: &Context, cx: f64, cy: f64, scale: f64) {
    cr.save().unwrap();
    cr.translate(cx, cy);
    cr.scale(scale, scale);
    body_path(cr, 22.0, 19.0);
    cr.set_line_width(2.0);
    let _ = cr.stroke();
    for side in [-1.0, 1.0] {
        cr.save().unwrap();
        cr.translate(side * 4.4, -0.5);
        cr.scale(1.5, 2.5);
        cr.new_path();
        cr.arc(0.0, 0.0, 1.0, 0.0, std::f64::consts::TAU);
        cr.restore().unwrap();
        let _ = cr.fill();
    }
    cr.restore().unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_blinks_now_and_then_and_rests_in_between() {
        let mut m = Mascot::new(0.0);
        let mut blinks = 0;
        let mut t = 0.0;
        while t < 60.0 {
            if let Some(d) = m.deadline(t) { t = d.max(t + 0.001); } else { break; }
            if m.advance(t) && m.blink.is_some_and(|(s, _)| s == t) { blinks += 1; }
        }
        assert!((7..=25).contains(&blinks), "{blinks} blinks a minute");
        // between blinks it asks for no frames
        let mut m = Mascot::new(0.0);
        m.advance(0.5);
        assert!(!m.is_animating(1.0));
    }

    #[test]
    fn asleep_it_shuts_its_eyes_and_wakes_with_a_stretch() {
        let mut m = Mascot::new(0.0);
        m.set_idle(true, 10.0);
        assert!(m.pose(0.0, 13.0).open < 0.05);
        // reactions wait until it wakes
        m.react(Reaction::Startle, 13.0);
        assert!(m.reaction.is_none());
        // it stops dreaming after a while: no more wake-ups for it
        assert!(m.deadline(10.0 + DREAM_FOR + 5.0).is_none());
        m.set_idle(false, 200.0);
        assert_eq!(m.reaction.map(|r| r.0), Some(Reaction::Wake));
        assert!(m.pose(0.0, 200.4).scale_y > 1.05);
        assert!(m.pose(0.0, 203.0).open > 0.95);
    }

    #[test]
    fn a_startle_hops_and_lands() {
        let mut m = Mascot::new(0.0);
        m.react(Reaction::Startle, 1.0);
        let top = (0..30).map(|i| m.pose(0.0, 1.0 + i as f64 / 60.0).lift).fold(0.0, f64::max);
        assert!(top > 4.0);
        assert!(m.pose(0.0, 1.7).lift.abs() < 0.01);
    }
}

#[cfg(test)]
mod gallery {
    use super::*;

    /// `ISLAND_GALLERY=1 cargo test mascot_poses -- --ignored`: its moods
    /// side by side on a closed line, at 1.5× and zoomed 4×.
    #[test]
    #[ignore]
    fn mascot_poses() {
        let scale = 1.5 * 4.0;
        let cell = 44.0;
        let moods: [(&str, Option<(Reaction, f64)>, bool, f64); 9] = [
            ("rest", None, false, 0.0),
            ("glance", None, false, 0.0),
            ("startle", Some((Reaction::Startle, 0.25)), false, 0.0),
            ("happy", Some((Reaction::Happy, 0.3)), false, 0.0),
            ("shake", Some((Reaction::Shake, 0.1)), false, 0.0),
            ("flash", Some((Reaction::Flash, 0.08)), false, 0.0),
            ("poke", Some((Reaction::Poke, 0.02)), false, 0.0),
            ("beat", None, false, 1.0),
            ("asleep", None, true, 0.0),
        ];
        let w = cell * moods.len() as f64;
        let surface = cairo::ImageSurface::create(cairo::Format::Rgb24, (w * scale) as i32, (cell * scale) as i32).unwrap();
        let cr = Context::new(&surface).unwrap();
        cr.scale(scale, scale);
        cr.set_source_rgb(0.0, 0.0, 0.0);
        cr.paint().unwrap();
        for (i, (_, reaction, asleep, beat)) in moods.iter().enumerate() {
            let mut m = Mascot::new(0.0);
            m.next_blink = 1e9;
            let mut now = 5.0;
            if i == 1 { m.look_x.set(-1.0, 0.0); m.look_y.set(-0.3, 0.0); }
            if *asleep { m.set_idle(true, 0.0); now = 3.3; }
            if let Some((r, t)) = reaction { m.react(*r, now - t); }
            m.draw(&cr, cell * i as f64 + cell / 2.0, cell / 2.0 + 4.0, 1.0, *beat, now);
        }
        std::fs::create_dir_all("target/gallery").unwrap();
        let mut f = std::fs::File::create("target/gallery/mascot.png").unwrap();
        surface.write_to_png(&mut f).unwrap();
    }
}
