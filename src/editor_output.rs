//! Full command output, shaped once on receipt and painted a viewport at a time.
use cairo::Context;
use crate::island::{Font, Renderer};

pub const HEIGHT: f64 = 238.0;
const ROWS: usize = 8;
const LINE_H: f64 = 22.0;

pub struct Output {
    pub id: String,
    pub title: String,
    pub body: String,
    pub error: bool,
    lines: Vec<pango::LayoutLine>,
    scroll: usize,
}

impl Output {
    pub fn new(id: String, title: String, body: String, error: bool, renderer: &Renderer) -> Self {
        let layout = renderer.output_layout(&body);
        Self { id, title, body, error, lines: layout.lines_readonly(), scroll: 0 }
    }
    pub fn append(&mut self, text: &str, renderer: &Renderer) {
        self.body.push_str(text);
        self.lines = renderer.output_layout(&self.body).lines_readonly();
    }
    pub fn scroll(&mut self, rows: f64) {
        self.scroll = (self.scroll as f64 + rows * 3.0).round()
            .clamp(0.0, self.lines.len().saturating_sub(ROWS) as f64) as usize;
    }
    pub fn hold(&self) -> f64 { if self.lines.len() > 2 { 12.0 } else { 4.0 } }
    pub fn draw(&self, cr: &Context, renderer: &Renderer, x: f64, y: f64, w: f64, alpha: f64) {
        let title = renderer.layout_ellipsized(cr, &self.title, Font::Tiny, w - 90.0, 1);
        cr.set_source_rgba(if self.error { 1.0 } else { 0.65 }, 0.65, 0.70, alpha);
        cr.move_to(renderer.snap(x + 22.0), renderer.snap(y + 12.0));
        pangocairo::functions::show_layout(cr, &title);
        let close = renderer.layout(cr, "×", Font::Text);
        cr.set_source_rgba(1.0, 1.0, 1.0, 0.55 * alpha);
        cr.move_to(renderer.snap(x + w - 36.0), renderer.snap(y + 9.0));
        pangocairo::functions::show_layout(cr, &close);
        cr.save().unwrap();
        cr.rectangle(x + 22.0, y + 36.0, w - 44.0, ROWS as f64 * LINE_H);
        cr.clip();
        cr.set_source_rgba(0.92, 0.94, 0.97, alpha);
        for (row, line) in self.lines.iter().skip(self.scroll).take(ROWS).enumerate() {
            cr.move_to(renderer.snap(x + 22.0), renderer.snap(y + 51.0 + row as f64 * LINE_H));
            pangocairo::functions::show_layout_line(cr, line);
        }
        cr.restore().unwrap();
        let end = (self.scroll + ROWS).min(self.lines.len());
        let hint = format!("{}–{} / {}   ·   Scroll", self.scroll + 1, end, self.lines.len());
        let hint = renderer.layout(cr, &hint, Font::Tiny);
        cr.set_source_rgba(1.0, 1.0, 1.0, 0.35 * alpha);
        cr.move_to(renderer.snap(x + 22.0), renderer.snap(y + HEIGHT - 22.0));
        pangocairo::functions::show_layout(cr, &hint);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn full_unicode_output_wraps_and_scrolls_to_last_line() {
        let renderer = Renderer::new(1.5);
        let body = format!("{}\nEND", "Привет 世界 ".repeat(2000));
        let mut output = Output::new("one".into(), ":echo".into(), body.clone(), false, &renderer);
        assert_eq!(output.body, body);
        assert!(output.lines.len() > ROWS);
        output.scroll(1e9);
        assert_eq!(output.scroll + ROWS, output.lines.len());
        output.append("\nAFTER", &renderer);
        assert!(output.body.ends_with("END\nAFTER"));
        output.scroll(-1e9);
        assert_eq!(output.scroll, 0);
    }
}
