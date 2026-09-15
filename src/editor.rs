//! Neovim in the island: the mode chip that sits beside the clock while an
//! editor is focused, and the panel the island opens into when the pointer
//! rests on it — the file, where the cursor is, the diagnostics, git, and
//! the buffer list you can click through.

use std::f64::consts::PI;

use cairo::Context;

use crate::anim::Time;
use crate::island::{Font, Renderer, smoothstep};
use crate::nvim::{Diagnostics, Editor};

pub const PANEL_W: f64 = 470.0;
pub const PANEL_H: f64 = 238.0;
pub const PANEL_R: f64 = 30.0;

const PAD: f64 = 22.0;
const ICON: f64 = 44.0;
const ICON_R: f64 = 11.0;
const STATS_Y: f64 = 88.0;
const LIST_TOP: f64 = 112.0;
const ROW_H: f64 = 30.0;
pub const MAX_BUFFERS: usize = 3;
const BUTTON_Y: f64 = 208.0;
const BUTTON_GAP: f64 = 96.0;

/// The chip's own metrics, in the line's coordinates.
const CHIP_H: f64 = 19.0;
const CHIP_PAD: f64 = 8.0;
const CHIP_GAP: f64 = 6.0;

pub const ERROR_COLOR: (f64, f64, f64) = (1.0, 0.23, 0.19);
pub const WARN_COLOR: (f64, f64, f64) = (1.0, 0.70, 0.25);
pub const INFO_COLOR: (f64, f64, f64) = (0.40, 0.85, 1.0);
pub const GIT_ADD: (f64, f64, f64) = (0.20, 0.78, 0.35);

/// What the pointer can do in the panel.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Action {
    /// switch to this buffer
    Buffer(i64),
    /// jump to the first diagnostic
    Diagnostics,
    Save,
    Format,
    CloseBuffer,
}

/// The chip's text: the mode letter, then whatever is worth saying.
fn chip_parts(editor: &Editor) -> (String, String) {
    let mode = editor.mode();
    let mut extra = String::new();
    if let Some((current, total)) = editor.search {
        extra = format!("{current}/{total}");
    } else {
        let d = &editor.diagnostics;
        if d.errors > 0 {
            extra.push_str(&format!("{}", d.errors));
        }
        if d.warnings > 0 {
            if !extra.is_empty() {
                extra.push(' ');
            }
            extra.push_str(&format!("{}", d.warnings));
        }
    }
    let file = if editor.file.is_empty() { "Untitled" } else { &editor.file };
    let mut name: String = file.chars().take(24).collect();
    if file.chars().count() > 24 {
        name.push('…');
    }
    let label = format!("{}  {}", mode.label(), name);
    (label, extra)
}

pub fn chip_width(cr: &Context, renderer: &Renderer, editor: &Editor) -> f64 {
    let (letter, extra) = chip_parts(editor);
    let mut w = CHIP_PAD * 2.0 + renderer.layout(cr, &letter, Font::Small).pixel_extents().1.width() as f64;
    if editor.modified {
        w += 10.0;
    }
    if !editor.recording.is_empty() {
        w += 12.0;
    }
    if !extra.is_empty() {
        w += 6.0 + renderer.layout(cr, &extra, Font::Small).pixel_extents().1.width() as f64;
        if editor.search.is_none() {
            // room for the little diagnostic dots
            w += 6.0 * (editor.diagnostics.errors.min(1) + editor.diagnostics.warnings.min(1)) as f64;
        }
    }
    w
}

/// Draw the chip with its left edge at `x`, centred on `cy`.
pub fn draw_chip(cr: &Context, renderer: &Renderer, editor: &Editor, x: f64, cy: f64, w: f64, alpha: f64, now: Time) {
    if alpha <= 0.004 || w <= 1.0 {
        return;
    }
    let mode = editor.mode();
    let (r, g, b) = mode.colour();
    let y = cy - CHIP_H / 2.0;
    capsule(cr, x, y, w, CHIP_H, CHIP_H / 2.0);
    cr.set_source_rgba(r, g, b, 0.20 * alpha);
    let _ = cr.fill();

    let mut cx = x + CHIP_PAD;
    // recording a macro: the red dot breathes, like a recording light
    if !editor.recording.is_empty() {
        let pulse = 0.55 + 0.45 * (0.5 - 0.5 * (2.0 * PI * now / 1.2).cos());
        cr.new_path();
        cr.arc(cx + 3.0, cy, 3.4, 0.0, 2.0 * PI);
        cr.set_source_rgba(1.0, 0.23, 0.19, alpha * pulse);
        let _ = cr.fill();
        cx += 12.0;
    }

    let (letter, extra) = chip_parts(editor);
    let layout = renderer.layout(cr, &letter, Font::Small);
    let (_, ext) = layout.pixel_extents();
    cr.set_source_rgba(r, g, b, alpha);
    cr.move_to(renderer.snap(cx), renderer.snap(cy - ext.height() as f64 / 2.0 - ext.y() as f64));
    pangocairo::functions::show_layout(cr, &layout);
    cx += ext.width() as f64;

    // an unsaved buffer gets a dot in the mode's colour
    if editor.modified {
        cr.new_path();
        cr.arc(cx + 5.0, cy, 2.4, 0.0, 2.0 * PI);
        cr.set_source_rgba(r, g, b, alpha * 0.9);
        let _ = cr.fill();
        cx += 10.0;
    }

    if !extra.is_empty() {
        cx += 6.0;
        match editor.search {
            Some(_) => {
                let layout = renderer.layout(cr, &extra, Font::Small);
                let (_, e) = layout.pixel_extents();
                cr.set_source_rgba(1.0, 1.0, 1.0, 0.75 * alpha);
                cr.move_to(renderer.snap(cx), renderer.snap(cy - e.height() as f64 / 2.0 - e.y() as f64));
                pangocairo::functions::show_layout(cr, &layout);
            }
            None => {
                let d = &editor.diagnostics;
                for (count, colour) in [(d.errors, ERROR_COLOR), (d.warnings, WARN_COLOR)] {
                    if count == 0 {
                        continue;
                    }
                    cr.new_path();
                    cr.arc(cx + 2.4, cy, 2.4, 0.0, 2.0 * PI);
                    cr.set_source_rgba(colour.0, colour.1, colour.2, alpha);
                    let _ = cr.fill();
                    cx += 8.0;
                    let layout = renderer.layout(cr, &count.to_string(), Font::Small);
                    let (_, e) = layout.pixel_extents();
                    cr.set_source_rgba(colour.0, colour.1, colour.2, alpha);
                    cr.move_to(renderer.snap(cx), renderer.snap(cy - e.height() as f64 / 2.0 - e.y() as f64));
                    pangocairo::functions::show_layout(cr, &layout);
                    cx += e.width() as f64 + 6.0;
                }
            }
        }
    }
}

/// Where the buttons sit, relative to the panel's top-left corner.
fn button_centres(w: f64) -> [(Action, f64, f64); 3] {
    let cx = w / 2.0;
    [
        (Action::Save, cx - BUTTON_GAP, BUTTON_Y),
        (Action::Format, cx, BUTTON_Y),
        (Action::CloseBuffer, cx + BUTTON_GAP, BUTTON_Y),
    ]
}

/// Hit test in the panel's own coordinates.
pub fn hit(editor: &Editor, w: f64, x: f64, y: f64) -> Option<Action> {
    for (action, bx, by) in button_centres(w) {
        if (x - bx).abs() <= 34.0 && (y - by).abs() <= 20.0 {
            return Some(action);
        }
    }
    if y >= LIST_TOP && y < LIST_TOP + ROW_H * MAX_BUFFERS as f64 {
        let index = ((y - LIST_TOP) / ROW_H) as usize;
        if let Some(buffer) = editor.buffers.iter().take(MAX_BUFFERS).nth(index) {
            return Some(Action::Buffer(buffer.id));
        }
    }
    if editor.diagnostics.any() && (y - STATS_Y).abs() < 16.0 && x > w * 0.55 {
        return Some(Action::Diagnostics);
    }
    None
}

/// The panel: the file, where the cursor is, git, diagnostics, buffers.
#[allow(clippy::too_many_arguments)]
pub fn draw(
    cr: &Context,
    renderer: &Renderer,
    editor: &Editor,
    x: f64,
    y: f64,
    w: f64,
    alpha: f64,
    hovered: Option<Action>,
    pressed: Option<(Action, Time)>,
    now: Time,
    opened: Time,
) {
    if alpha <= 0.003 {
        return;
    }
    let t = now - opened;
    let stage = |delay: f64| {
        let k = smoothstep(((t - delay) / 0.28).clamp(0.0, 1.0));
        (alpha * k, 8.0 * (1.0 - k))
    };

    // the file card: a tile in the language's colour with its initials
    let (ha, ho) = stage(0.0);
    let icon_x = x + PAD;
    let icon_y = y + PAD + ho;
    language_tile(cr, renderer, &editor.lang, icon_x, icon_y, ICON, ha);

    let text_x = renderer.snap(icon_x + ICON + 14.0);
    let text_w = w - (text_x - x) - PAD;
    let name = if editor.file.is_empty() { "[No Name]".to_string() } else { editor.file.clone() };
    let title = renderer.layout_ellipsized(cr, &name, Font::Text, text_w - 40.0, 1);
    let (_, t_ext) = title.pixel_extents();
    cr.set_source_rgba(1.0, 1.0, 1.0, ha);
    cr.move_to(text_x + ho, renderer.snap(y + PAD + 2.0 - t_ext.y() as f64));
    pangocairo::functions::show_layout(cr, &title);
    if editor.modified {
        let (r, g, b) = editor.mode().colour();
        cr.new_path();
        cr.arc(text_x + t_ext.width() as f64 + 10.0, y + PAD + 2.0 + t_ext.height() as f64 / 2.0, 3.0, 0.0, 2.0 * PI);
        cr.set_source_rgba(r, g, b, ha);
        let _ = cr.fill();
    }
    let dir = renderer.layout_ellipsized(cr, &editor.dir, Font::Small, text_w, 1);
    let (_, d_ext) = dir.pixel_extents();
    cr.set_source_rgba(1.0, 1.0, 1.0, 0.45 * ha);
    cr.move_to(text_x + ho, renderer.snap(y + PAD + 2.0 + t_ext.height() as f64 + 2.0 - d_ext.y() as f64));
    pangocairo::functions::show_layout(cr, &dir);

    // the stats line: mode, position, git, diagnostics
    let (sa, so) = stage(0.06);
    let mode = editor.mode();
    let (mr, mg, mb) = mode.colour();
    let mut sx = x + PAD;
    let sy = y + STATS_Y + so;
    let mode_label = renderer.layout(cr, mode.name(), Font::Tiny);
    let (_, m_ext) = mode_label.pixel_extents();
    capsule(cr, sx, sy - 9.0, m_ext.width() as f64 + 16.0, 18.0, 9.0);
    cr.set_source_rgba(mr, mg, mb, 0.20 * sa);
    let _ = cr.fill();
    cr.set_source_rgba(mr, mg, mb, sa);
    cr.move_to(renderer.snap(sx + 8.0), renderer.snap(sy - m_ext.height() as f64 / 2.0 - m_ext.y() as f64));
    pangocairo::functions::show_layout(cr, &mode_label);
    sx += m_ext.width() as f64 + 16.0 + 14.0;

    // "Top" and "End" read better than 0% and 100%, as in Vim's ruler
    let place = if editor.line <= 1 {
        "Top".to_string()
    } else if editor.line >= editor.lines {
        "End".to_string()
    } else {
        format!("{}%", editor.percent)
    };
    let position = format!("{}:{}  ·  {}  ·  {}", editor.line, editor.col, place, editor.indent);
    let layout = renderer.layout_ellipsized(cr, &position, Font::Tiny, w * 0.4, 1);
    let (_, p_ext) = layout.pixel_extents();
    cr.set_source_rgba(1.0, 1.0, 1.0, 0.55 * sa);
    cr.move_to(renderer.snap(sx), renderer.snap(sy - p_ext.height() as f64 / 2.0 - p_ext.y() as f64));
    pangocairo::functions::show_layout(cr, &layout);

    // diagnostics on the right of the same line, clickable
    let mut rx = x + w - PAD;
    if editor.diagnostics.any() {
        let lit = hovered == Some(Action::Diagnostics);
        rx = draw_diagnostics(cr, renderer, &editor.diagnostics, rx, sy, sa * if lit { 1.0 } else { 0.85 });
    }
    if !editor.branch.is_empty() {
        let text = if editor.added + editor.removed > 0 {
            format!("{}  +{} −{}", editor.branch, editor.added, editor.removed)
        } else {
            editor.branch.clone()
        };
        let layout = renderer.layout_ellipsized(cr, &text, Font::Tiny, w * 0.35, 1);
        let (_, e) = layout.pixel_extents();
        let bx = rx - e.width() as f64 - 14.0;
        cr.set_source_rgba(GIT_ADD.0, GIT_ADD.1, GIT_ADD.2, 0.85 * sa);
        cr.move_to(renderer.snap(bx), renderer.snap(sy - e.height() as f64 / 2.0 - e.y() as f64));
        pangocairo::functions::show_layout(cr, &layout);
    }

    // the buffer list
    for (i, buffer) in editor.buffers.iter().take(MAX_BUFFERS).enumerate() {
        let (ba, bo) = stage(0.1 + 0.04 * i as f64);
        let ry = y + LIST_TOP + ROW_H * i as f64 + bo;
        let lit = hovered == Some(Action::Buffer(buffer.id));
        if buffer.current || lit {
            capsule(cr, x + PAD - 8.0, ry + 1.0, w - 2.0 * (PAD - 8.0), ROW_H - 4.0, 9.0);
            cr.set_source_rgba(1.0, 1.0, 1.0, if lit { 0.12 } else { 0.07 } * ba);
            let _ = cr.fill();
        }
        let mark = renderer.layout(cr, if buffer.current { "▸" } else { " " }, Font::Tiny);
        let (_, k_ext) = mark.pixel_extents();
        cr.set_source_rgba(mr, mg, mb, ba);
        cr.move_to(renderer.snap(x + PAD), renderer.snap(ry + (ROW_H - k_ext.height() as f64) / 2.0 - k_ext.y() as f64));
        pangocairo::functions::show_layout(cr, &mark);
        let label = renderer.layout_ellipsized(cr, &buffer.name, Font::Small, w - 2.0 * PAD - 60.0, 1);
        let (_, l_ext) = label.pixel_extents();
        cr.set_source_rgba(1.0, 1.0, 1.0, if buffer.current { 0.95 } else { 0.6 } * ba);
        cr.move_to(renderer.snap(x + PAD + 18.0), renderer.snap(ry + (ROW_H - l_ext.height() as f64) / 2.0 - l_ext.y() as f64));
        pangocairo::functions::show_layout(cr, &label);
        if buffer.modified {
            cr.new_path();
            cr.arc(x + PAD + 18.0 + l_ext.width() as f64 + 9.0, ry + ROW_H / 2.0, 2.6, 0.0, 2.0 * PI);
            cr.set_source_rgba(1.0, 1.0, 1.0, 0.5 * ba);
            let _ = cr.fill();
        }
    }

    // the buttons
    let (ca, co) = stage(0.16);
    for (action, bx, by) in button_centres(w) {
        let lit = hovered == Some(action);
        let squish = match pressed {
            Some((a, since)) if a == action => 1.0 - 0.14 * (-(now - since) / 0.12).exp(),
            _ => 1.0,
        };
        let label = match action {
            Action::Save => "Save",
            Action::Format => "Format",
            _ => "Close",
        };
        let layout = renderer.layout(cr, label, Font::Tiny);
        let (_, e) = layout.pixel_extents();
        let bw = (e.width() as f64 + 34.0) * squish;
        capsule(cr, x + bx - bw / 2.0, y + by - 13.0 * squish + co, bw, 26.0 * squish, 13.0);
        cr.set_source_rgba(1.0, 1.0, 1.0, (if lit { 0.16 } else { 0.08 }) * ca);
        let _ = cr.fill();
        cr.set_source_rgba(1.0, 1.0, 1.0, (if lit { 1.0 } else { 0.8 }) * ca);
        cr.move_to(
            renderer.snap(x + bx - e.width() as f64 / 2.0),
            renderer.snap(y + by - e.height() as f64 / 2.0 - e.y() as f64 + co),
        );
        pangocairo::functions::show_layout(cr, &layout);
    }
}

/// The counts, right-aligned, each with its dot; returns the new right edge.
fn draw_diagnostics(cr: &Context, renderer: &Renderer, d: &Diagnostics, right: f64, cy: f64, alpha: f64) -> f64 {
    let mut x = right;
    for (count, colour) in [(d.hints, INFO_COLOR), (d.info, INFO_COLOR), (d.warnings, WARN_COLOR), (d.errors, ERROR_COLOR)] {
        if count == 0 {
            continue;
        }
        let layout = renderer.layout(cr, &count.to_string(), Font::Tiny);
        let (_, e) = layout.pixel_extents();
        x -= e.width() as f64;
        cr.set_source_rgba(colour.0, colour.1, colour.2, alpha);
        cr.move_to(renderer.snap(x), renderer.snap(cy - e.height() as f64 / 2.0 - e.y() as f64));
        pangocairo::functions::show_layout(cr, &layout);
        x -= 7.0;
        cr.new_path();
        cr.arc(x, cy, 2.6, 0.0, 2.0 * PI);
        let _ = cr.fill();
        x -= 12.0;
    }
    x
}

/// A tile with the language's initials, coloured from its name so every
/// language keeps its own tint.
fn language_tile(cr: &Context, renderer: &Renderer, lang: &str, x: f64, y: f64, size: f64, alpha: f64) {
    let (initials, hue) = match lang {
        "rust" => ("RS", 18.0),
        "lua" => ("LUA", 235.0),
        "python" => ("PY", 210.0),
        "c" => ("C", 205.0),
        "cpp" => ("C++", 205.0),
        "javascript" => ("JS", 50.0),
        "typescript" => ("TS", 205.0),
        "go" => ("GO", 190.0),
        "sh" | "bash" | "zsh" => ("SH", 120.0),
        "markdown" => ("MD", 0.0),
        "toml" => ("TOML", 25.0),
        "json" => ("JSON", 60.0),
        "yaml" => ("YAML", 300.0),
        "html" => ("HTML", 15.0),
        "css" | "scss" => ("CSS", 220.0),
        "vim" => ("VIM", 120.0),
        "sql" => ("SQL", 195.0),
        "" => ("··", 220.0),
        other => {
            let hue = (other.bytes().fold(0u32, |h, b| h.wrapping_mul(31).wrapping_add(b as u32)) % 360) as f64;
            (other, hue)
        }
    };
    let (r, g, b) = hsl(hue, 0.45, 0.45);
    capsule(cr, x, y, size, size, ICON_R);
    let grad = cairo::LinearGradient::new(x, y, x, y + size);
    grad.add_color_stop_rgba(0.0, r * 1.15, g * 1.15, b * 1.15, alpha);
    grad.add_color_stop_rgba(1.0, r * 0.8, g * 0.8, b * 0.8, alpha);
    let _ = cr.set_source(&grad);
    let _ = cr.fill();
    // four letters still fit the tile; longer names are cut to two
    let text: String = if initials.chars().count() <= 4 {
        initials.to_uppercase()
    } else {
        initials.chars().take(2).collect::<String>().to_uppercase()
    };
    let layout = renderer.layout(cr, &text, Font::Tiny);
    let (_, e) = layout.pixel_extents();
    cr.set_source_rgba(1.0, 1.0, 1.0, alpha);
    cr.move_to(
        renderer.snap(x + (size - e.width() as f64) / 2.0),
        renderer.snap(y + (size - e.height() as f64) / 2.0 - e.y() as f64),
    );
    pangocairo::functions::show_layout(cr, &layout);
}

fn hsl(h: f64, s: f64, l: f64) -> (f64, f64, f64) {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = l - c / 2.0;
    let (r, g, b) = match h as u32 / 60 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    (r + m, g + m, b + m)
}

fn capsule(cr: &Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    let r = r.min(w / 2.0).min(h / 2.0);
    cr.new_path();
    cr.arc(x + r, y + r, r, PI, 1.5 * PI);
    cr.arc(x + w - r, y + r, r, 1.5 * PI, 2.0 * PI);
    cr.arc(x + w - r, y + h - r, r, 0.0, 0.5 * PI);
    cr.arc(x + r, y + h - r, r, 0.5 * PI, PI);
    cr.close_path();
}

/// The gap between the chip and the clock.
pub const GAP: f64 = CHIP_GAP;
