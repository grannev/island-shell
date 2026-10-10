//! The island's sheet design system: Apple's dark-mode palette, text
//! styles and controls, shared by the quick menu and every sheet opened
//! from it (and by the launcher, the player and the editor panel). One
//! drawing helper per role; see docs/interface-style.md.
use cairo::Context;

use crate::island::{Font, Renderer};

pub type Rgb = (f64, f64, f64);

// iOS / macOS dark-mode system colours.
pub const BLUE: Rgb = (0.039, 0.518, 1.0); // #0A84FF
pub const GREEN: Rgb = (0.188, 0.820, 0.345); // #30D158
pub const ORANGE: Rgb = (1.0, 0.624, 0.039); // #FF9F0A
pub const RED: Rgb = (1.0, 0.271, 0.227); // #FF453A
pub const PURPLE: Rgb = (0.749, 0.353, 0.949); // #BF5AF2
pub const TEAL: Rgb = (0.392, 0.824, 1.0); // #64D2FF
pub const GRAY: Rgb = (0.388, 0.388, 0.400); // #636366 (systemGray2)

/// Label opacities on black: primary, secondary, tertiary.
pub const PRIMARY: f64 = 1.0;
pub const SECONDARY: f64 = 0.6;
pub const TERTIARY: f64 = 0.3;

/// secondarySystemGroupedBackground (#1C1C1E): cards, lists, tiles
pub const GROUPED: Rgb = (0.110, 0.110, 0.118);
const GROUPED_HOVER: Rgb = (0.165, 0.165, 0.176);

/// Outer content rail of every sheet.
pub const RAIL: f64 = 22.0;
/// Vertical centre line of a sheet's header row.
pub const HEADER_MID: f64 = 40.0;
/// Where content starts below the header.
pub const CONTENT_TOP: f64 = 76.0;
pub const CLOSE_SIZE: f64 = 30.0;
/// Header icon tile.
pub const TILE: f64 = 36.0;
/// Corner radius of grouped lists and cards.
pub const GROUP_R: f64 = 12.0;
/// Height of a regular / small capsule button.
pub const BUTTON_H: f64 = 34.0;
pub const SMALL_BUTTON_H: f64 = 30.0;

#[derive(Clone, Copy, PartialEq)]
pub enum ButtonKind {
    /// grey fill: the default
    Neutral,
    /// filled system blue: the main action
    Primary,
    /// orange-tinted: the timer's chosen duration / resume
    Warm,
    /// red-tinted: removal, cancelling
    Destructive,
    /// no fill, blue text: navigation ("‹ Назад") and inline toggles
    Plain,
}

pub fn rounded(cr: &Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    let r = r.min(w / 2.0).min(h / 2.0).max(0.0);
    cr.new_path();
    cr.arc(x + w - r, y + r, r, -std::f64::consts::FRAC_PI_2, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, std::f64::consts::FRAC_PI_2);
    cr.arc(x + r, y + h - r, r, std::f64::consts::FRAC_PI_2, std::f64::consts::PI);
    cr.arc(x + r, y + r, r, std::f64::consts::PI, 3.0 * std::f64::consts::FRAC_PI_2);
    cr.close_path();
}

fn mix(a: Rgb, b: Rgb, k: f64) -> Rgb {
    (a.0 + (b.0 - a.0) * k, a.1 + (b.1 - a.1) * k, a.2 + (b.2 - a.2) * k)
}

fn lighten(c: Rgb, k: f64) -> Rgb {
    mix(c, (1.0, 1.0, 1.0), k)
}

fn set(cr: &Context, c: Rgb, alpha: f64) {
    cr.set_source_rgba(c.0, c.1, c.2, alpha.clamp(0.0, 1.0));
}

/// Text in `colour`, ellipsized to `w`, its layout box's top-left at x, y.
#[allow(clippy::too_many_arguments)]
pub fn text(cr: &Context, renderer: &Renderer, s: &str, x: f64, y: f64, w: f64, font: Font, colour: Rgb, alpha: f64) {
    let layout = renderer.layout_ellipsized(cr, s, font, w.max(4.0), 1);
    set(cr, colour, alpha);
    cr.move_to(renderer.snap(x), renderer.snap(y));
    pangocairo::functions::show_layout(cr, &layout);
    cr.new_path();
}

/// White text at a label opacity (PRIMARY / SECONDARY / TERTIARY × alpha).
#[allow(clippy::too_many_arguments)]
pub fn label(cr: &Context, renderer: &Renderer, s: &str, x: f64, y: f64, w: f64, font: Font, alpha: f64) {
    text(cr, renderer, s, x, y, w, font, (1.0, 1.0, 1.0), alpha);
}

/// Text centred on a box.
#[allow(clippy::too_many_arguments)]
pub fn centred(cr: &Context, renderer: &Renderer, s: &str, rect: (f64, f64, f64, f64), font: Font, colour: Rgb, alpha: f64) {
    let (x, y, w, h) = rect;
    let layout = renderer.layout_ellipsized(cr, s, font, (w - 12.0).max(4.0), 1);
    let (_, logical) = layout.pixel_extents();
    set(cr, colour, alpha);
    // the logical line box, not the ink: labels with and without
    // descenders or capitals then share one baseline across a row
    cr.move_to(
        renderer.snap(x + (w - logical.width() as f64) / 2.0 - logical.x() as f64),
        renderer.snap(y + (h - logical.height() as f64) / 2.0 - logical.y() as f64),
    );
    pangocairo::functions::show_layout(cr, &layout);
    cr.new_path();
}

/// The single size of a label's line box for `font`, used to stack
/// title / subtitle pairs identically everywhere.
pub fn line_height(cr: &Context, renderer: &Renderer, font: Font) -> f64 {
    renderer.layout(cr, "Ag", font).pixel_extents().1.height() as f64
}

/// A capsule button. Disabled controls keep their shape at lower opacity.
#[allow(clippy::too_many_arguments)]
pub fn button(cr: &Context, renderer: &Renderer, s: &str, rect: (f64, f64, f64, f64), kind: ButtonKind, hover: f64, alpha: f64) {
    let (x, y, w, h) = rect;
    let hover = hover.clamp(0.0, 1.0);
    let alpha = alpha.clamp(0.0, 1.0);
    let (fill, fill_alpha, ink) = match kind {
        ButtonKind::Neutral => ((1.0, 1.0, 1.0), 0.14 + 0.06 * hover, (1.0, 1.0, 1.0)),
        ButtonKind::Primary => (lighten(BLUE, 0.12 * hover), 1.0, (1.0, 1.0, 1.0)),
        ButtonKind::Warm => (ORANGE, 0.22 + 0.08 * hover, lighten(ORANGE, 0.15)),
        ButtonKind::Destructive => (RED, 0.20 + 0.08 * hover, lighten(RED, 0.18)),
        ButtonKind::Plain => (BLUE, 0.0, lighten(BLUE, 0.25 * hover)),
    };
    if fill_alpha > 0.0 {
        rounded(cr, x, y, w, h, h / 2.0);
        set(cr, fill, fill_alpha * alpha);
        let _ = cr.fill();
    }
    centred(cr, renderer, s, rect, Font::Label, ink, alpha);
}

/// The close control: a grey circle with an xmark, the same in every
/// header; whether it dismisses or returns is up to the sheet.
pub fn close(cr: &Context, x: f64, y: f64, size: f64, hover: f64, alpha: f64) {
    let hover = hover.clamp(0.0, 1.0);
    let alpha = alpha.clamp(0.0, 1.0);
    let (cx, cy, r) = (x + size / 2.0, y + size / 2.0, size / 2.0);
    cr.new_path();
    cr.arc(cx, cy, r, 0.0, std::f64::consts::TAU);
    cr.set_source_rgba(1.0, 1.0, 1.0, (0.14 + 0.08 * hover) * alpha);
    let _ = cr.fill();
    let arm = size * 0.16;
    cr.set_line_width(2.0);
    cr.set_line_cap(cairo::LineCap::Round);
    cr.set_source_rgba(1.0, 1.0, 1.0, (0.62 + 0.38 * hover) * alpha);
    cr.move_to(cx - arm, cy - arm);
    cr.line_to(cx + arm, cy + arm);
    cr.move_to(cx + arm, cy - arm);
    cr.line_to(cx - arm, cy + arm);
    let _ = cr.stroke();
    cr.new_path();
}

/// Where the close control of a sheet `width` wide sits (x, y, size).
pub fn close_rect(width: f64) -> (f64, f64, f64) {
    (width - RAIL - CLOSE_SIZE, HEADER_MID - CLOSE_SIZE / 2.0, CLOSE_SIZE)
}

/// An app-icon tile: a coloured rounded square with a faint top light.
pub fn tile(cr: &Context, x: f64, y: f64, size: f64, colour: Rgb, alpha: f64) {
    let r = size * 0.25;
    rounded(cr, x, y, size, size, r);
    let gradient = cairo::LinearGradient::new(0.0, y, 0.0, y + size);
    let top = lighten(colour, 0.10);
    gradient.add_color_stop_rgba(0.0, top.0, top.1, top.2, alpha);
    gradient.add_color_stop_rgba(1.0, colour.0, colour.1, colour.2, alpha);
    let _ = cr.set_source(&gradient);
    let _ = cr.fill();
}

/// A round icon well: accent-filled when active, grey otherwise (Control
/// Center's module buttons, list rows' leading symbols).
pub fn well(cr: &Context, cx: f64, cy: f64, r: f64, colour: Option<Rgb>, alpha: f64) {
    cr.new_path();
    cr.arc(cx, cy, r, 0.0, std::f64::consts::TAU);
    match colour {
        Some(c) => set(cr, c, alpha),
        None => cr.set_source_rgba(1.0, 1.0, 1.0, 0.14 * alpha),
    }
    let _ = cr.fill();
}

/// A sheet header whose subtitle animates its changes (motion.rs) under
/// `key` (include the sheet's instance so a fresh sheet does not animate).
#[allow(clippy::too_many_arguments)]
pub fn header_keyed(
    cr: &Context, renderer: &Renderer, x: f64, y: f64, width: f64,
    icon: Option<(Rgb, &dyn Fn(&Context, f64, f64))>, title: &str, subtitle: Option<&str>,
    reserve: f64, alpha: f64, key: &str,
) {
    header_with(cr, renderer, x, y, width, icon, title, subtitle, reserve, alpha, Some(key));
}

#[allow(clippy::too_many_arguments)]
fn header_with(
    cr: &Context, renderer: &Renderer, x: f64, y: f64, width: f64,
    icon: Option<(Rgb, &dyn Fn(&Context, f64, f64))>, title: &str, subtitle: Option<&str>,
    reserve: f64, alpha: f64, key: Option<&str>,
) {
    let mut tx = x + RAIL;
    if let Some((colour, glyph)) = icon {
        tile(cr, tx, y + HEADER_MID - TILE / 2.0, TILE, colour, alpha);
        cr.save().unwrap();
        glyph(cr, tx + TILE / 2.0, y + HEADER_MID);
        cr.restore().unwrap();
        tx += TILE + 12.0;
    }
    let w = (x + width - reserve - tx).max(10.0);
    let title_h = line_height(cr, renderer, Font::Title);
    match subtitle {
        Some(sub) => {
            let sub_h = line_height(cr, renderer, Font::Subhead);
            let top = y + HEADER_MID - (title_h + sub_h) / 2.0;
            label(cr, renderer, title, tx, top, w, Font::Title, PRIMARY * alpha);
            match key {
                Some(key) => crate::motion::text(cr, renderer, key, sub, tx, top + title_h, w, 0.0, Font::Subhead,
                    (1.0, 1.0, 1.0), SECONDARY * alpha),
                None => label(cr, renderer, sub, tx, top + title_h, w, Font::Subhead, SECONDARY * alpha),
            }
        }
        None => label(cr, renderer, title, tx, y + HEADER_MID - title_h / 2.0, w, Font::Title, PRIMARY * alpha),
    }
}

/// A grouped list / card background.
pub fn group(cr: &Context, x: f64, y: f64, w: f64, h: f64, hover: f64, alpha: f64) {
    rounded(cr, x, y, w, h, GROUP_R);
    set(cr, mix(GROUPED, GROUPED_HOVER, hover.clamp(0.0, 1.0)), alpha);
    let _ = cr.fill();
}

/// A list row's hover / selection wash, inset inside its group.
pub fn highlight(cr: &Context, x: f64, y: f64, w: f64, h: f64, amount: f64, alpha: f64) {
    if amount <= 0.002 {
        return;
    }
    rounded(cr, x + 4.0, y + 2.0, w - 8.0, h - 4.0, 8.0);
    cr.set_source_rgba(1.0, 1.0, 1.0, 0.07 * amount.clamp(0.0, 1.0) * alpha);
    let _ = cr.fill();
}

/// A hairline separator, one device pixel thick and on the pixel grid.
pub fn separator(cr: &Context, renderer: &Renderer, x0: f64, x1: f64, y: f64, alpha: f64) {
    let px = 1.0 / renderer.scale();
    let y = (y * renderer.scale()).floor() / renderer.scale() + px / 2.0;
    cr.new_path();
    cr.move_to(x0, y);
    cr.line_to(x1, y);
    cr.set_line_width(px);
    cr.set_source_rgba(1.0, 1.0, 1.0, 0.11 * alpha);
    let _ = cr.stroke();
}

/// A switch, 46 × 28: grey off, system blue on, the knob riding between.
pub fn toggle(cr: &Context, x: f64, y: f64, on: f64, hover: f64, alpha: f64) {
    let on = on.clamp(0.0, 1.0);
    let (w, h) = (46.0, 28.0);
    rounded(cr, x, y, w, h, h / 2.0);
    let off = (0.24 + 0.04 * hover, 0.24 + 0.04 * hover, 0.25 + 0.04 * hover);
    set(cr, mix(off, BLUE, on), alpha);
    let _ = cr.fill();
    let r = h / 2.0 - 2.0;
    let cx = x + 2.0 + r + (w - 4.0 - 2.0 * r) * on;
    cr.new_path();
    cr.arc(cx, y + h / 2.0 + 0.6, r, 0.0, std::f64::consts::TAU);
    cr.set_source_rgba(0.0, 0.0, 0.0, 0.18 * alpha);
    let _ = cr.fill();
    cr.new_path();
    cr.arc(cx, y + h / 2.0, r, 0.0, std::f64::consts::TAU);
    cr.set_source_rgba(1.0, 1.0, 1.0, alpha);
    let _ = cr.fill();
}

/// A segmented control; `slide` is the selected index, animated.
#[allow(clippy::too_many_arguments)]
pub fn segmented(
    cr: &Context, renderer: &Renderer, rect: (f64, f64, f64, f64), labels: &[&str],
    slide: f64, hover: &[f64], alpha: f64,
) {
    let (x, y, w, h) = rect;
    let n = labels.len().max(1) as f64;
    rounded(cr, x, y, w, h, 9.0);
    cr.set_source_rgba(1.0, 1.0, 1.0, 0.10 * alpha);
    let _ = cr.fill();
    let seg = (w - 4.0) / n;
    rounded(cr, x + 2.0 + slide * seg, y + 2.0, seg, h - 4.0, 7.0);
    set(cr, GRAY, alpha);
    let _ = cr.fill();
    for (i, s) in labels.iter().enumerate() {
        let selected = 1.0 - (slide - i as f64).abs().min(1.0);
        let lit = hover.get(i).copied().unwrap_or(0.0).clamp(0.0, 1.0);
        let a = SECONDARY + (PRIMARY - SECONDARY) * selected.max(0.5 * lit);
        centred(cr, renderer, s, (x + 2.0 + i as f64 * seg, y, seg, h), Font::Label, (1.0, 1.0, 1.0), a * alpha);
    }
}

/// A text field well; focused fields carry a blue ring.
pub fn field(cr: &Context, x: f64, y: f64, w: f64, h: f64, focused: bool, alpha: f64) {
    rounded(cr, x, y, w, h, 10.0);
    cr.set_source_rgba(1.0, 1.0, 1.0, 0.09 * alpha);
    let _ = cr.fill();
    if focused {
        rounded(cr, x - 1.0, y - 1.0, w + 2.0, h + 2.0, 11.0);
        cr.set_line_width(2.0);
        set(cr, BLUE, 0.85 * alpha);
        let _ = cr.stroke();
    }
}

/// The message field (iMessage's capsule): the text or a dim
/// placeholder, a blinking blue caret, and a round send button on the
/// right that turns blue once there is something to send. `typed` is when
/// the text last changed (the caret holds still while typing). Long text
/// scrolls left to keep the caret in view. The send button is a circle of
/// radius 13 centred `REPLY_SEND_INSET` from the right end.
#[allow(clippy::too_many_arguments)]
pub fn reply_field(
    cr: &Context, renderer: &Renderer, rect: (f64, f64, f64, f64), text: &str, placeholder: &str,
    typed: f64, now: f64, alpha: f64,
) {
    let (x, y, w, h) = rect;
    rounded(cr, x, y, w, h, h / 2.0);
    cr.set_source_rgba(1.0, 1.0, 1.0, 0.10 * alpha);
    let _ = cr.fill_preserve();
    cr.set_line_width(1.0);
    cr.set_source_rgba(1.0, 1.0, 1.0, 0.16 * alpha);
    let _ = cr.stroke();
    let room = w - 16.0 - 36.0;
    let ty = y + (h - line_height(cr, renderer, Font::Body)) / 2.0;
    let (shown, colour) = if text.is_empty() { (placeholder, TERTIARY) } else { (text, PRIMARY) };
    let layout = renderer.layout(cr, shown, Font::Body);
    let text_w = if text.is_empty() { 0.0 } else { layout.pixel_extents().1.width() as f64 };
    cr.save().unwrap();
    cr.rectangle(x + 12.0, y, room + 4.0, h);
    cr.clip();
    let scroll = (text_w - room).max(0.0);
    cr.set_source_rgba(1.0, 1.0, 1.0, colour * alpha);
    cr.move_to(renderer.snap(x + 14.0 - scroll), renderer.snap(ty));
    pangocairo::functions::show_layout(cr, &layout);
    if now - typed < 0.6 || ((now - typed) * 1.6).fract() < 0.55 {
        cr.rectangle(renderer.snap(x + 14.0 - scroll + text_w + 1.0), y + 9.0, 2.0, h - 18.0);
        set(cr, BLUE, alpha);
        let _ = cr.fill();
    }
    cr.restore().unwrap();
    let (cx, cy) = (x + w - REPLY_SEND_INSET, y + h / 2.0);
    let ready = !text.trim().is_empty();
    well(cr, cx, cy, if ready { 13.0 } else { 11.7 }, ready.then_some(BLUE), alpha);
    cr.new_path();
    cr.set_line_width(2.2);
    cr.set_line_cap(cairo::LineCap::Round);
    cr.set_line_join(cairo::LineJoin::Round);
    cr.set_source_rgba(1.0, 1.0, 1.0, alpha * if ready { 1.0 } else { 0.5 });
    cr.move_to(cx, cy + 6.0);
    cr.line_to(cx, cy - 6.0);
    cr.move_to(cx - 5.0, cy - 1.0);
    cr.line_to(cx, cy - 6.0);
    cr.line_to(cx + 5.0, cy - 1.0);
    let _ = cr.stroke();
    cr.new_path();
}

/// The reply field's height and its send button's distance from the
/// field's right end.
pub const REPLY_FIELD_H: f64 = 36.0;
pub const REPLY_SEND_INSET: f64 = 18.0;

/// A chevron: `angle` 0 points right, π/2 down.
pub fn chevron(cr: &Context, cx: f64, cy: f64, size: f64, angle: f64, alpha: f64) {
    cr.save().unwrap();
    cr.translate(cx, cy);
    cr.rotate(angle);
    cr.new_path();
    cr.move_to(-size * 0.35, -size * 0.7);
    cr.line_to(size * 0.35, 0.0);
    cr.line_to(-size * 0.35, size * 0.7);
    cr.set_line_width(2.0);
    cr.set_line_cap(cairo::LineCap::Round);
    cr.set_line_join(cairo::LineJoin::Round);
    cr.set_source_rgba(1.0, 1.0, 1.0, alpha);
    let _ = cr.stroke();
    cr.restore().unwrap();
}

/// A small badge capsule with a count or short word.
pub fn badge(cr: &Context, renderer: &Renderer, s: &str, x: f64, cy: f64, colour: Rgb, alpha: f64) -> f64 {
    let layout = renderer.layout(cr, s, Font::Footnote);
    let (_, logical) = layout.pixel_extents();
    let h = 18.0;
    let w = (logical.width() as f64 + 12.0).max(h);
    rounded(cr, x, cy - h / 2.0, w, h, h / 2.0);
    set(cr, colour, alpha);
    let _ = cr.fill();
    centred(cr, renderer, s, (x, cy - h / 2.0, w, h), Font::Footnote, (1.0, 1.0, 1.0), alpha);
    w
}

/// The activity indicator: a ring segment turning.
pub fn spinner(cr: &Context, cx: f64, cy: f64, r: f64, now: f64, alpha: f64) {
    cr.save().unwrap();
    cr.new_path();
    cr.set_line_width(2.0);
    cr.set_line_cap(cairo::LineCap::Round);
    cr.arc(cx, cy, r, now * 5.0, now * 5.0 + 4.3);
    cr.set_source_rgba(1.0, 1.0, 1.0, 0.7 * alpha);
    let _ = cr.stroke();
    cr.restore().unwrap();
}

/// Black gradients over the island's own black at the top and bottom of a
/// scrolling viewport, so content dissolves into its edges; `top` and
/// `bottom` (0..1) say how much there is to scroll to on each side.
#[allow(clippy::too_many_arguments)]
pub fn edge_fades(cr: &Context, x: f64, y: f64, w: f64, h: f64, top: f64, bottom: f64, depth: f64) {
    for (amount, from, to) in [(top, y, y + depth), (bottom, y + h, y + h - depth)] {
        if amount <= 0.0 {
            continue;
        }
        let g = cairo::LinearGradient::new(0.0, from, 0.0, to);
        g.add_color_stop_rgba(0.0, 0.0, 0.0, 0.0, amount.clamp(0.0, 1.0));
        g.add_color_stop_rgba(1.0, 0.0, 0.0, 0.0, 0.0);
        let _ = cr.set_source(&g);
        cr.rectangle(x, from.min(to), w, depth);
        let _ = cr.fill();
    }
}

/// A press: the control dips and springs back past rest (iOS's tap
/// feedback); `t` seconds since the press, 1 when it is over.
pub fn press_scale(t: f64) -> f64 {
    const DEPTH: f64 = 0.05;
    if !(0.0..0.7).contains(&t) {
        1.0
    } else if t < 0.08 {
        1.0 - DEPTH * crate::anim::min_jerk(t / 0.08)
    } else {
        let t = t - 0.08;
        1.0 - DEPTH * (-t / 0.09).exp() * (std::f64::consts::TAU * t / 0.5).cos()
    }
}

/// SF Symbols' bounce: the symbol swells and settles.
pub fn bounce_scale(t: f64) -> f64 {
    if !(0.0..0.9).contains(&t) {
        return 1.0;
    }
    1.0 + 0.3 * (-t / 0.13).exp() * (std::f64::consts::TAU * t / 0.34).sin()
}

/// SF Symbols' wiggle: a rotation (radians) that shakes out.
pub fn wiggle_angle(t: f64) -> f64 {
    if !(0.0..1.0).contains(&t) {
        return 0.0;
    }
    0.2 * (-t / 0.22).exp() * (std::f64::consts::TAU * t / 0.16).sin()
}

/// Scale (and turn) whatever `draw` paints about (cx, cy).
pub fn transformed(cr: &Context, cx: f64, cy: f64, scale: f64, angle: f64, draw: impl FnOnce()) {
    if (scale - 1.0).abs() < 1e-4 && angle.abs() < 1e-4 {
        draw();
        return;
    }
    cr.save().unwrap();
    cr.translate(cx, cy);
    cr.rotate(angle);
    cr.scale(scale, scale);
    cr.translate(-cx, -cy);
    draw();
    cr.restore().unwrap();
}
