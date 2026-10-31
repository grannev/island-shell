//! Neovim in the island: the mode chip that sits beside the clock while an
//! editor is focused, and the panel the island opens into when the pointer
//! rests on it — the file, where the cursor is, the diagnostics, git, and
//! the buffer list you can click through.

use std::f64::consts::PI;

use cairo::Context;

use crate::anim::{Time, min_jerk};
use crate::island::{Font, Renderer, smoothstep};
use crate::nvim::{Diagnostics, Editor};

pub const PANEL_W: f64 = 470.0;
pub const PANEL_H: f64 = BUTTON_Y + crate::menu_style::SMALL_BUTTON_H + PAD;
pub const PANEL_R: f64 = crate::menu_style::tight(30.0);

const PAD: f64 = 22.0;
const ICON: f64 = crate::menu_style::TILE;
const ICON_R: f64 = 9.0;
const STATS_Y: f64 = 86.0;
const LIST_TOP: f64 = 106.0;
const ROW_H: f64 = 34.0;
pub const MAX_BUFFERS: usize = 3;
const BUTTON_Y: f64 = LIST_TOP + ROW_H * MAX_BUFFERS as f64 + 12.0;
const BUTTON_GAP: f64 = 8.0;

/// The chip's own metrics, in the line's coordinates.
const CHIP_H: f64 = 19.0;
const CHIP_PAD: f64 = 8.0;
const CHIP_GAP: f64 = 6.0;
/// How long a write lights the chip up, seconds.
pub const SAVE_FLASH: f64 = 0.8;

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
    Diagnostic(usize),
    Save,
    Format,
    CloseBuffer,
    CloseOutput,
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

/// Draw the chip with its left edge at `x`, centred on `cy`. `saved` is
/// how long ago the editor wrote its file, while that is still worth a
/// flash (under `SAVE_FLASH`).
#[allow(clippy::too_many_arguments)]
pub fn draw_chip(cr: &Context, renderer: &Renderer, editor: &Editor, x: f64, cy: f64, w: f64, alpha: f64, saved: Option<f64>, now: Time) {
    if alpha <= 0.004 || w <= 1.0 {
        return;
    }
    let mode = editor.mode();
    let (r, g, b) = mode.colour();
    let y = cy - CHIP_H / 2.0;
    capsule(cr, x, y, w, CHIP_H, CHIP_H / 2.0);
    cr.set_source_rgba(r, g, b, 0.20 * alpha);
    let _ = cr.fill();
    if let Some(t) = saved {
        save_flash(cr, x, y, w, t, alpha);
    }

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

/// The write's flash: the capsule fills green for an instant and a soft
/// band of light sweeps across it, left to right, under the text. Fast
/// in, slow out — the whole thing is over before it can get in the way.
fn save_flash(cr: &Context, x: f64, y: f64, w: f64, t: f64, alpha: f64) {
    let k = (t / SAVE_FLASH).clamp(0.0, 1.0);
    let glow = smoothstep((t / 0.08).min(1.0)) * (1.0 - smoothstep(k));
    if glow <= 0.004 {
        return;
    }
    let (gr, gg, gb) = GIT_ADD;
    cr.save().unwrap();
    capsule(cr, x, y, w, CHIP_H, CHIP_H / 2.0);
    cr.clip();
    cr.set_source_rgba(gr, gg, gb, 0.28 * glow * alpha);
    let _ = cr.paint();
    // the band starts off the left edge and leaves through the right
    let band = 26.0;
    let sx = x - band + (w + 2.0 * band) * min_jerk(k);
    let grad = cairo::LinearGradient::new(sx - band, 0.0, sx + band, 0.0);
    grad.add_color_stop_rgba(0.0, 1.0, 1.0, 1.0, 0.0);
    grad.add_color_stop_rgba(0.5, 1.0, 1.0, 1.0, 0.35 * glow * alpha);
    grad.add_color_stop_rgba(1.0, 1.0, 1.0, 1.0, 0.0);
    let _ = cr.set_source(&grad);
    cr.rectangle(sx - band, y, 2.0 * band, CHIP_H);
    let _ = cr.fill();
    cr.restore().unwrap();
}

/// Where the buttons sit (x, y, w), relative to the panel's top-left corner:
/// three equal capsules across the content rail.
fn buttons(w: f64) -> [(Action, f64, f64, f64); 3] {
    let bw = (w - 2.0 * PAD - 2.0 * BUTTON_GAP) / 3.0;
    [
        (Action::Save, PAD, BUTTON_Y, bw),
        (Action::Format, PAD + bw + BUTTON_GAP, BUTTON_Y, bw),
        (Action::CloseBuffer, PAD + 2.0 * (bw + BUTTON_GAP), BUTTON_Y, bw),
    ]
}

/// Hit test in the panel's own coordinates.
pub fn hit(editor: &Editor, w: f64, x: f64, y: f64, diagnostic_view: bool) -> Option<Action> {
    for (action, bx, by, bw) in buttons(w) {
        if x >= bx && x <= bx + bw && y >= by - 3.0 && y <= by + crate::menu_style::SMALL_BUTTON_H + 3.0 {
            return Some(action);
        }
    }
    if y >= LIST_TOP && y < LIST_TOP + ROW_H * MAX_BUFFERS as f64 {
        let index = ((y - LIST_TOP) / ROW_H) as usize;
        if diagnostic_view {
            return editor.diagnostic_items.get(index).map(|_| Action::Diagnostic(index));
        }
        if let Some(buffer) = editor.buffers.iter().take(MAX_BUFFERS).nth(index) {
            return Some(Action::Buffer(buffer.id));
        }
    }
    if (diagnostic_view || editor.diagnostics.any()) && (y - STATS_Y).abs() < 16.0 && x > w * 0.55 {
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
    diagnostic_view: bool,
    command: Option<&crate::nvim::Cmdline>,
    now: Time,
    opened: Time,
) {
    if alpha <= 0.003 {
        return;
    }
    if let Some(command) = command {
        draw_command(cr, renderer, command, x, y, w, alpha);
        return;
    }
    let t = now - opened;
    let stage = |delay: f64| {
        let k = smoothstep(((t - delay) / 0.28).clamp(0.0, 1.0));
        (alpha * k, 8.0 * (1.0 - k))
    };

    use crate::menu_style::{self, ButtonKind};
    // the header: a tile in the language's colour with its initials, the
    // file and where it lives
    let (ha, ho) = stage(0.0);
    let icon_x = x + PAD;
    let icon_y = y + menu_style::HEADER_MID - ICON / 2.0 + ho;
    language_tile(cr, renderer, &editor.lang, icon_x, icon_y, ICON, ha);
    let text_x = icon_x + ICON + 12.0;
    let text_w = w - (text_x - x) - PAD;
    let name = if editor.file.is_empty() { "[No Name]".to_string() } else { editor.file.clone() };
    let title_h = menu_style::line_height(cr, renderer, Font::Title);
    let sub_h = menu_style::line_height(cr, renderer, Font::Subhead);
    let top = y + menu_style::HEADER_MID - (title_h + sub_h) / 2.0 + ho;
    let title = renderer.layout_ellipsized(cr, &name, Font::Title, text_w - 24.0, 1);
    let title_w = title.pixel_extents().1.width() as f64;
    menu_style::label(cr, renderer, &name, text_x, top, text_w - 24.0, Font::Title, ha);
    if editor.modified {
        let (r, g, b) = editor.mode().colour();
        cr.new_path();
        cr.arc(text_x + title_w + 10.0, top + title_h / 2.0, 3.5, 0.0, 2.0 * PI);
        cr.set_source_rgba(r, g, b, ha);
        let _ = cr.fill();
    }
    menu_style::label(cr, renderer, &editor.dir, text_x, top + title_h, text_w, Font::Subhead, menu_style::SECONDARY * ha);

    // the stats line: mode, position, git, diagnostics
    let (sa, so) = stage(0.06);
    let mode = editor.mode();
    let (mr, mg, mb) = mode.colour();
    let mut sx = x + PAD;
    let sy = y + STATS_Y + so;
    let mode_label = renderer.layout(cr, mode.name(), Font::Footnote);
    let (_, m_ext) = mode_label.pixel_extents();
    let mode_w = m_ext.width() as f64 + 16.0;
    capsule(cr, sx, sy - 10.0, mode_w, 20.0, 10.0);
    cr.set_source_rgba(mr, mg, mb, 0.22 * sa);
    let _ = cr.fill();
    menu_style::centred(cr, renderer, mode.name(), (sx, sy - 10.0, mode_w, 20.0), Font::Footnote, (mr, mg, mb), sa);
    sx += mode_w + 12.0;

    // "Top" and "End" read better than 0% and 100%, as in Vim's ruler
    let place = if editor.line <= 1 {
        "Top".to_string()
    } else if editor.line >= editor.lines {
        "End".to_string()
    } else {
        format!("{}%", editor.percent)
    };
    let position = format!("{}:{}  ·  {}  ·  {}", editor.line, editor.col, place, editor.indent);
    let foot_h = menu_style::line_height(cr, renderer, Font::Subhead);
    menu_style::label(cr, renderer, &position, sx, sy - foot_h / 2.0, w * 0.42, Font::Subhead, menu_style::SECONDARY * sa);

    // diagnostics on the right of the same line, clickable
    let mut rx = x + w - PAD;
    if diagnostic_view {
        let back = "Буферы";
        let bw = renderer.layout(cr, back, Font::Subhead).pixel_extents().1.width() as f64;
        menu_style::text(cr, renderer, back, rx - bw, sy - foot_h / 2.0, bw + 2.0, Font::Subhead, menu_style::BLUE, sa);
        menu_style::chevron(cr, rx - bw - 9.0, sy, 4.5, PI, sa * 0.9);
        rx -= bw + 24.0;
    } else if editor.diagnostics.any() {
        let lit = hovered == Some(Action::Diagnostics);
        rx = draw_diagnostics(cr, renderer, &editor.diagnostics, rx, sy, sa * if lit { 1.0 } else { 0.85 });
    }
    if !editor.branch.is_empty() {
        let text = if editor.added + editor.removed > 0 {
            format!("{}  +{} −{}", editor.branch, editor.added, editor.removed)
        } else {
            editor.branch.clone()
        };
        let layout = renderer.layout_ellipsized(cr, &text, Font::Subhead, w * 0.35, 1);
        let bw = layout.pixel_extents().1.width() as f64;
        menu_style::text(cr, renderer, &text, rx - bw - 12.0, sy - foot_h / 2.0, bw + 2.0, Font::Subhead, menu_style::GREEN, 0.9 * sa);
    }

    // the list: buffers, or this file's problems — one grouped list
    let (la, lo) = stage(0.1);
    let list_h = ROW_H * MAX_BUFFERS as f64;
    menu_style::group(cr, x + PAD, y + LIST_TOP + lo, w - 2.0 * PAD, list_h, 0.0, la);
    let body_h = menu_style::line_height(cr, renderer, Font::Body);
    if diagnostic_view {
        for (i, d) in editor.diagnostic_items.iter().take(MAX_BUFFERS).enumerate() {
            let ry = y + LIST_TOP + ROW_H * i as f64 + lo;
            if i > 0 {
                menu_style::separator(cr, renderer, x + PAD + 30.0, x + w - PAD, ry, la);
            }
            menu_style::highlight(cr, x + PAD, ry, w - 2.0 * PAD, ROW_H, f64::from(hovered == Some(Action::Diagnostic(i))), la);
            let colour = match d.severity { 1 => ERROR_COLOR, 2 => WARN_COLOR, _ => INFO_COLOR };
            cr.new_path();
            cr.arc(x + PAD + 15.0, ry + ROW_H / 2.0, 3.5, 0.0, 2.0 * PI);
            cr.set_source_rgba(colour.0, colour.1, colour.2, la);
            let _ = cr.fill();
            let place = format!("{}:{}", d.line, d.col);
            let pw = renderer.layout(cr, &place, Font::Subhead).pixel_extents().1.width() as f64;
            menu_style::label(cr, renderer, &place, x + w - PAD - 12.0 - pw, ry + (ROW_H - foot_h) / 2.0, pw + 2.0,
                Font::Subhead, menu_style::SECONDARY * la);
            menu_style::label(cr, renderer, &d.message, x + PAD + 30.0, ry + (ROW_H - body_h) / 2.0,
                w - 2.0 * PAD - 54.0 - pw, Font::Body, la);
        }
        if editor.diagnostic_items.is_empty() {
            menu_style::centred(cr, renderer, "Проблем нет", (x + PAD, y + LIST_TOP + lo, w - 2.0 * PAD, list_h),
                Font::Body, (1.0, 1.0, 1.0), menu_style::SECONDARY * la);
        }
    } else {
        for (i, buffer) in editor.buffers.iter().take(MAX_BUFFERS).enumerate() {
            let (ba, bo) = stage(0.1 + 0.04 * i as f64);
            let ry = y + LIST_TOP + ROW_H * i as f64 + bo;
            if i > 0 {
                menu_style::separator(cr, renderer, x + PAD + 14.0, x + w - PAD, ry, ba);
            }
            menu_style::highlight(cr, x + PAD, ry, w - 2.0 * PAD, ROW_H, f64::from(hovered == Some(Action::Buffer(buffer.id))), ba);
            let room = w - 2.0 * PAD - 28.0 - 30.0;
            let layout = renderer.layout_ellipsized(cr, &buffer.name, Font::Body, room, 1);
            let nw = layout.pixel_extents().1.width() as f64;
            menu_style::label(cr, renderer, &buffer.name, x + PAD + 14.0, ry + (ROW_H - body_h) / 2.0, room, Font::Body,
                if buffer.current { 1.0 } else { 0.85 } * ba);
            if buffer.modified {
                cr.new_path();
                cr.arc(x + PAD + 14.0 + nw + 9.0, ry + ROW_H / 2.0, 3.0, 0.0, 2.0 * PI);
                cr.set_source_rgba(1.0, 1.0, 1.0, menu_style::SECONDARY * ba);
                let _ = cr.fill();
            }
            if buffer.current {
                // the iOS selection mark, in the mode's colour
                let (cx, cy) = (x + w - PAD - 22.0, ry + ROW_H / 2.0);
                cr.new_path();
                cr.move_to(cx - 5.5, cy + 0.5);
                cr.line_to(cx - 1.5, cy + 4.5);
                cr.line_to(cx + 6.0, cy - 4.5);
                cr.set_line_width(2.2);
                cr.set_line_cap(cairo::LineCap::Round);
                cr.set_line_join(cairo::LineJoin::Round);
                cr.set_source_rgba(mr, mg, mb, ba);
                let _ = cr.stroke();
            }
        }
    }

    // the buttons
    let (ca, co) = stage(0.16);
    for (action, bx, by, bw) in buttons(w) {
        let lit = hovered == Some(action);
        let squish = match pressed {
            Some((a, since)) if a == action => 1.0 - 0.06 * (-(now - since) / 0.12).exp(),
            _ => 1.0,
        };
        let (label, kind) = match action {
            Action::Save => ("Сохранить", if editor.modified { ButtonKind::Primary } else { ButtonKind::Neutral }),
            Action::Format => ("Формат", ButtonKind::Neutral),
            _ => ("Закрыть", ButtonKind::Neutral),
        };
        let h = menu_style::SMALL_BUTTON_H;
        let (sw, sh) = (bw * squish, h * squish);
        menu_style::button(cr, renderer, label, (x + bx + (bw - sw) / 2.0, y + by + (h - sh) / 2.0 + co, sw, sh),
            kind, f64::from(lit), ca);
    }
}

/// The counts, right-aligned, each with its dot; returns the new right edge.
fn draw_diagnostics(cr: &Context, renderer: &Renderer, d: &Diagnostics, right: f64, cy: f64, alpha: f64) -> f64 {
    let mut x = right;
    for (count, colour) in [(d.hints, INFO_COLOR), (d.info, INFO_COLOR), (d.warnings, WARN_COLOR), (d.errors, ERROR_COLOR)] {
        if count == 0 {
            continue;
        }
        let layout = renderer.layout(cr, &count.to_string(), Font::Subhead);
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
    let layout = renderer.layout(cr, &text, if text.chars().count() > 2 { Font::Tiny } else { Font::Label });
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
    let r = crate::menu_style::corner(r, w, h);
    cr.new_path();
    cr.arc(x + r, y + r, r, PI, 1.5 * PI);
    cr.arc(x + w - r, y + r, r, 1.5 * PI, 2.0 * PI);
    cr.arc(x + w - r, y + h - r, r, 0.0, 0.5 * PI);
    cr.arc(x + r, y + h - r, r, 0.5 * PI, PI);
    cr.close_path();
}

/// The gap between the chip and the clock.
pub const GAP: f64 = CHIP_GAP;

/// Pango uses byte offsets too, so Unicode and proportional glyphs share one caret.
fn draw_command(cr: &Context, renderer: &Renderer, command: &crate::nvim::Cmdline, x: f64, y: f64, w: f64, alpha: f64) {
    let label = match command.prefix.as_str() { "/" | "?" => "Search", _ => "Command" };
    let title = renderer.layout(cr, label, Font::Tiny);
    cr.set_source_rgba(1.0, 1.0, 1.0, 0.45 * alpha);
    cr.move_to(renderer.snap(x + PAD), renderer.snap(y + 14.0));
    pangocairo::functions::show_layout(cr, &title);
    let prefix = format!("{} ", command.prefix);
    let text = format!("{}{}", prefix, command.text);
    let mut pos = command.pos.min(command.text.len());
    while !command.text.is_char_boundary(pos) { pos -= 1; }
    let layout = renderer.layout(cr, &text, Font::Text);
    let caret = layout.cursor_pos((prefix.len() + pos) as i32).0;
    let caret_x = caret.x() as f64 / pango::SCALE as f64;
    let available = w - 2.0 * PAD;
    let scroll = (caret_x - available + 8.0).max(0.0);
    cr.save().unwrap();
    cr.rectangle(x + PAD, y + 35.0, available, 30.0);
    cr.clip();
    cr.set_source_rgba(1.0, 1.0, 1.0, 0.94 * alpha);
    cr.move_to(renderer.snap(x + PAD - scroll), renderer.snap(y + 40.0));
    pangocairo::functions::show_layout(cr, &layout);
    cr.new_path();
    cr.rectangle(renderer.snap(x + PAD + caret_x - scroll), renderer.snap(y + 40.0), 1.5, 20.0);
    cr.set_source_rgba(0.65, 0.8, 1.0, alpha);
    let _ = cr.fill();
    cr.restore().unwrap();
}

#[cfg(test)]
mod command_tests {
    use super::*;
    #[test]
    fn diagnostics_hit_the_exact_row_and_back_control() {
        let e = Editor { diagnostic_items: vec![crate::nvim::DiagnosticItem {
            buffer: 42, line: 10, col: 3, severity: 1, message: "Undefined name".into(),
        }], ..Editor::default() };
        assert_eq!(hit(&e, PANEL_W, 30.0, LIST_TOP + 5.0, true), Some(Action::Diagnostic(0)));
        assert_eq!(hit(&e, PANEL_W, 440.0, STATS_Y, true), Some(Action::Diagnostics));
        assert_eq!(hit(&e, PANEL_W, 30.0, LIST_TOP + ROW_H + 5.0, true), None);
    }
    #[test]
    fn command_renders_unicode_and_long_input_at_fractional_scale() {
        let renderer = Renderer::new(1.5);
        let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, 705, 126).unwrap();
        let cr = Context::new(&surface).unwrap();
        cr.scale(1.5, 1.5);
        for text in ["echo 'Привет, мир'".to_string(), "界".repeat(200)] {
            for pos in [0, 1, text.len(), usize::MAX] {
                let command = crate::nvim::Cmdline { text: text.clone(), prefix: ":".into(), pos };
                draw_command(&cr, &renderer, &command, 0.0, 0.0, PANEL_W, 1.0);
                assert!(cr.status().is_ok());
            }
        }
        cr.set_source_rgb(0.0, 0.0, 0.0);
        cr.paint().unwrap();
        draw_command(&cr, &renderer, &crate::nvim::Cmdline { text: "Telescope find_files".into(), prefix: ":".into(), pos: 19 }, 0.0, 0.0, PANEL_W, 1.0);
        if let Ok(path) = std::env::var("ISLAND_COMMAND_PREVIEW") {
            surface.write_to_png(&mut std::fs::File::create(path).unwrap()).unwrap();
        }
    }
}
