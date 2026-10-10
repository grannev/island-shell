//! Vorssaint's floating buttons: round black buttons beside an open page
//! of the island (the Control Center, the sound page, the notification
//! center, the calendar, the timer). They leave the island's sides like
//! drops of the same black: each starts as a small circle tucked under the
//! edge, swells as it moves out, and the neck that ties it to the island
//! thins and lets go before it settles. They take the pointer only once
//! they have arrived; leaving, they fall back in quickly.
use cairo::Context;

/// A button's size and its distance from the island's side.
pub const DIAMETER: f64 = 44.0;
const GAP: f64 = 12.0;
/// From one button's centre to the next one's.
const ROW: f64 = 54.0;
/// How far the buttons reach out from the island (for the input region and
/// damage).
pub const REACH: f64 = GAP + DIAMETER + 6.0;
/// How far under the edge a button starts.
const TUCK: f64 = 14.0;

/// Where a button goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Button {
    Sections,
    Search,
    Timer,
    Notifications,
    Sound,
    Calendar,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Side {
    Left,
    Right,
}

pub const ALL: [Button; 6] = [Button::Sections, Button::Search, Button::Timer, Button::Notifications, Button::Sound, Button::Calendar];

fn place(button: Button) -> (Side, usize, usize) {
    match button {
        Button::Sections => (Side::Left, 0, 3),
        Button::Search => (Side::Left, 1, 3),
        Button::Timer => (Side::Left, 2, 3),
        Button::Notifications => (Side::Right, 0, 3),
        Button::Sound => (Side::Right, 1, 3),
        Button::Calendar => (Side::Right, 2, 3),
    }
}

/// The open page the buttons stand beside: its sides and its top and
/// bottom edges.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Body {
    pub x0: f64,
    pub x1: f64,
    pub top: f64,
    pub bottom: f64,
}

/// A button's centre `progress` of the way out (1 = arrived). A column
/// starts level with the title row, and a short page lifts it so it stays
/// beside the page.
fn centre(button: Button, body: Body, progress: f64) -> (f64, f64) {
    let (side, index, count) = place(button);
    let phase = progress.clamp(0.0, 1.1);
    let offset = -TUCK + phase * (TUCK + GAP + DIAMETER / 2.0);
    let span = (count - 1) as f64 * ROW;
    let height = body.bottom - body.top;
    let first = body.top + (DIAMETER / 2.0 + GAP).max((56.0f64).min(height / 2.0 - span / 2.0));
    let y = first + index as f64 * ROW;
    match side {
        Side::Left => (body.x0 - offset, y),
        Side::Right => (body.x1 + offset, y),
    }
}

/// The lowest point any button reaches, for the painted bounds.
pub fn bottom(body: Body) -> f64 {
    ALL.iter().map(|b| centre(*b, body, 1.0).1 + DIAMETER / 2.0).fold(body.top, f64::max)
}

/// The button under (px, py), once the buttons have arrived.
pub fn hit(body: Body, px: f64, py: f64) -> Option<Button> {
    ALL.into_iter().find(|b| {
        let (cx, cy) = centre(*b, body, 1.0);
        (px - cx).hypot(py - cy) <= DIAMETER / 2.0
    })
}

/// How a button looks this frame.
pub struct Look {
    /// how far out the buttons are, 0..1 (a little past 1 while settling)
    pub progress: f64,
    /// under the pointer, 0..1
    pub lit: f64,
    /// the page it leads to is the one showing
    pub selected: bool,
    /// seconds since it was pressed
    pub pressed: f64,
}

/// One button: its drop of black, then its symbol.
pub fn draw(cr: &Context, button: Button, body: Body, look: &Look, alpha: f64) {
    let phase = look.progress.clamp(0.0, 1.1);
    if phase <= 0.001 || alpha <= 0.002 {
        return;
    }
    let (side, _, _) = place(button);
    let (cx, cy) = centre(button, body, phase);
    let radius = DIAMETER / 2.0 * (0.4 + 0.6 * phase);
    cr.save().unwrap();
    // drawn as if on the left side; the right side is its mirror
    if side == Side::Right {
        cr.translate(body.x1, 0.0);
        cr.scale(-1.0, 1.0);
        cr.translate(-body.x0, 0.0);
    }
    let (cx_l, edge) = match side {
        Side::Left => (cx, body.x0),
        Side::Right => (body.x0 - (cx - body.x1), body.x0),
    };
    cr.new_path();
    cr.arc(cx_l, cy, radius, 0.0, std::f64::consts::TAU);
    // the neck thins away before the circle has finished moving; its other
    // end sits under the island, so there is no seam
    let neck = radius * (1.0 - phase / 0.84).max(0.0);
    if neck > 0.0 && cx_l + radius < edge {
        cr.move_to(edge + 2.0, cy + neck);
        cr.curve_to(edge - 7.0, cy + neck, cx_l + radius, cy + neck * 0.15, cx_l + radius * 0.55, cy + neck * 0.75);
        cr.line_to(cx_l + radius * 0.55, cy - neck * 0.75);
        cr.curve_to(cx_l + radius, cy - neck * 0.15, edge - 7.0, cy - neck, edge + 2.0, cy - neck);
        cr.close_path();
    }
    cr.set_source_rgba(0.0, 0.0, 0.0, alpha);
    let _ = cr.fill();
    cr.restore().unwrap();

    // the symbol and the light under the pointer ride on the circle
    let k = (phase.min(1.0) - 0.25).max(0.0) / 0.75;
    let a = alpha * k;
    if a <= 0.002 {
        return;
    }
    let press = crate::menu_style::press_scale(look.pressed);
    let wash = (0.09 * look.lit.clamp(0.0, 1.0)).max(if look.selected { 0.16 } else { 0.0 });
    if wash > 0.0 {
        cr.new_path();
        cr.arc(cx, cy, radius * press, 0.0, std::f64::consts::TAU);
        cr.set_source_rgba(1.0, 1.0, 1.0, wash * a);
        let _ = cr.fill();
    }
    let scale = (0.4 + 0.6 * phase.min(1.0)) * press;
    crate::menu_style::transformed(cr, cx, cy, scale, 0.0, || {
        let white = (1.0, 1.0, 1.0, a);
        match button {
            Button::Sections => crate::quick::glyph(cr, "grid", cx, cy, 1.0, white),
            Button::Search => crate::quick::icon(cr, crate::quick::Action::Launcher, cx, cy, a, 0.62),
            Button::Timer => crate::quick::icon(cr, crate::quick::Action::Timer, cx, cy, a, 0.62),
            Button::Notifications => crate::quick::icon(cr, crate::quick::Action::Notifications, cx, cy, a, 0.66),
            Button::Sound => crate::quick::glyph(cr, "speaker", cx + 1.5, cy, 1.0, white),
            Button::Calendar => crate::quick::glyph(cr, "calendar", cx, cy, 1.0, white),
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    const BODY: Body = Body { x0: 200.0, x1: 760.0, top: 0.0, bottom: 336.0 };

    #[test]
    fn buttons_stand_outside_the_page_and_answer_only_there() {
        for button in ALL {
            let (cx, cy) = centre(button, BODY, 1.0);
            assert!(cx + DIAMETER / 2.0 < BODY.x0 || cx - DIAMETER / 2.0 > BODY.x1, "{button:?} beside the page");
            assert!(cx - DIAMETER / 2.0 >= BODY.x0 - REACH && cx + DIAMETER / 2.0 <= BODY.x1 + REACH);
            assert_eq!(hit(BODY, cx, cy), Some(button));
        }
        assert_eq!(hit(BODY, 480.0, 100.0), None);
        // the start is tucked under the edge
        let (cx, _) = centre(Button::Search, BODY, 0.0);
        assert!(cx > BODY.x0);
    }

    #[test]
    fn a_short_page_lifts_its_column() {
        let short = Body { bottom: 176.0, ..BODY };
        assert!(bottom(short) <= short.bottom + 0.5, "the right column fits beside the timer");
        let (_, tall) = centre(Button::Notifications, BODY, 1.0);
        let (_, lifted) = centre(Button::Notifications, short, 1.0);
        assert!(lifted < tall);
    }
}
