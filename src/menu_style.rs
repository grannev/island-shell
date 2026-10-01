//! Shared controls for the island's modal sheets. Navigation tiles, switches,
//! segmented tabs and inline row actions have their own visual roles.
use cairo::Context;

use crate::island::{Font, Renderer};

#[derive(Clone, Copy)]
pub enum ButtonKind {
    Neutral,
    Primary,
    Warm,
    Destructive,
}

fn rounded(cr: &Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    cr.new_path();
    cr.arc(x + w - r, y + r, r, -std::f64::consts::FRAC_PI_2, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, std::f64::consts::FRAC_PI_2);
    cr.arc(x + r, y + h - r, r, std::f64::consts::FRAC_PI_2, std::f64::consts::PI);
    cr.arc(x + r, y + r, r, std::f64::consts::PI, 3.0 * std::f64::consts::FRAC_PI_2);
    cr.close_path();
}

pub fn button(
    cr: &Context, renderer: &Renderer, text: &str, rect: (f64, f64, f64, f64),
    font: Font, kind: ButtonKind, hover: f64, alpha: f64,
) {
    let (x, y, w, h) = rect;
    let hover = hover.clamp(0.0, 1.0);
    let alpha = alpha.clamp(0.0, 1.0);
    let radius = 10.0_f64.min(h / 2.0);
    let (red, green, blue) = match kind {
        ButtonKind::Neutral | ButtonKind::Destructive =>
            (0.15 + 0.07 * hover, 0.15 + 0.07 * hover, 0.16 + 0.07 * hover),
        ButtonKind::Primary => (0.13 + 0.07 * hover, 0.42 + 0.08 * hover, 0.78 + 0.07 * hover),
        ButtonKind::Warm => (0.36 + 0.10 * hover, 0.25 + 0.07 * hover, 0.13 + 0.04 * hover),
    };
    rounded(cr, x, y, w, h, radius);
    cr.set_source_rgba(red, green, blue, alpha);
    let _ = cr.fill();
    rounded(cr, x + 0.5, y + 0.5, w - 1.0, h - 1.0, radius - 0.5);
    cr.set_line_width(1.0);
    match kind {
        ButtonKind::Warm => cr.set_source_rgba(1.0, 0.68, 0.32, (0.17 + 0.14 * hover) * alpha),
        _ => cr.set_source_rgba(1.0, 1.0, 1.0, (0.07 + 0.11 * hover) * alpha),
    }
    let _ = cr.stroke();
    let layout = renderer.layout_ellipsized(cr, text, font, (w - 16.0).max(4.0), 1);
    let (_, ink) = layout.pixel_extents();
    if matches!(kind, ButtonKind::Destructive) {
        cr.set_source_rgba(1.0, 0.46 + 0.16 * hover, 0.44 + 0.16 * hover, alpha * (0.82 + 0.18 * hover));
    } else {
        cr.set_source_rgba(1.0, 1.0, 1.0, alpha * (0.82 + 0.18 * hover));
    }
    cr.move_to(
        renderer.snap(x + (w - ink.width() as f64) / 2.0 - ink.x() as f64),
        renderer.snap(y + (h - ink.height() as f64) / 2.0 - ink.y() as f64),
    );
    pangocairo::functions::show_layout(cr, &layout);
    cr.new_path();
}

pub fn close(cr: &Context, x: f64, y: f64, size: f64, hover: f64, alpha: f64) {
    let hover = hover.clamp(0.0, 1.0);
    let alpha = alpha.clamp(0.0, 1.0);
    let radius = 10.0_f64.min(size / 2.0);
    rounded(cr, x, y, size, size, radius);
    cr.set_source_rgba(0.15 + 0.07 * hover, 0.15 + 0.07 * hover, 0.16 + 0.07 * hover, alpha);
    let _ = cr.fill();
    rounded(cr, x + 0.5, y + 0.5, size - 1.0, size - 1.0, radius - 0.5);
    cr.set_line_width(1.0);
    cr.set_source_rgba(1.0, 1.0, 1.0, (0.07 + 0.11 * hover) * alpha);
    let _ = cr.stroke();
    let inset = size * 0.31;
    cr.set_line_width(1.8);
    cr.set_line_cap(cairo::LineCap::Round);
    cr.set_source_rgba(1.0, 1.0, 1.0, (0.72 + 0.28 * hover) * alpha);
    cr.move_to(x + inset, y + inset);
    cr.line_to(x + size - inset, y + size - inset);
    cr.move_to(x + size - inset, y + inset);
    cr.line_to(x + inset, y + size - inset);
    let _ = cr.stroke();
    cr.new_path();
}
