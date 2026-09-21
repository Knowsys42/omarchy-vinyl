//! Control-button glyphs, drawn in cairo.
//!
//! The icon theme is not a dependency. A desktop pointed at an icon theme that
//! isn't installed (Omarchy ships `Yaru-gray` as a GTK theme name, with no
//! matching icon theme) leaves GTK with only the handful of icons compiled into
//! libgtk; `media-skip-*` and `view-restore-symbolic` are not among them, so
//! those buttons render as broken-image placeholders. Drawing them here costs
//! less than shipping an icon theme and can't break on someone else's box.

use gtk::cairo;
use gtk::prelude::*;

#[derive(Clone, Copy)]
pub enum Glyph {
    Play,
    Pause,
    Prev,
    Next,
    Fullscreen,
    Restore,
    Pressing,
}

/// A glyph sized by CSS (`.glyph` min-width/min-height), inked in the CSS
/// colour, so hover, `.play` and `:disabled` all keep working.
pub fn icon(g: Glyph) -> gtk::DrawingArea {
    let area = gtk::DrawingArea::new();
    area.add_css_class("glyph");
    set(&area, g);
    area
}

pub fn set(area: &gtk::DrawingArea, g: Glyph) {
    area.set_draw_func(move |a, cr, w, h| {
        let c = a.color();
        cr.set_source_rgba(c.red() as f64, c.green() as f64, c.blue() as f64, c.alpha() as f64);
        let s = (w.min(h)) as f64;
        cr.translate((w as f64 - s) / 2.0, (h as f64 - s) / 2.0);
        cr.scale(s, s);
        cr.set_line_width(0.1);
        cr.set_line_cap(cairo::LineCap::Round);
        cr.set_line_join(cairo::LineJoin::Round);
        draw(g, cr);
    });
    area.queue_draw();
}

fn tri(cr: &cairo::Context, tip: f64, back: f64) {
    cr.move_to(back, 0.18);
    cr.line_to(tip, 0.5);
    cr.line_to(back, 0.82);
    cr.close_path();
}

/// Two lines meeting at `(cx, cy)`, reaching `len` towards the box's middle
/// (negative `len` reaches away from it) — one corner of a fullscreen bracket.
fn corner(cr: &cairo::Context, cx: f64, cy: f64, len: f64) {
    let sx = if cx < 0.5 { len } else { -len };
    let sy = if cy < 0.5 { len } else { -len };
    cr.move_to(cx, cy + sy);
    cr.line_to(cx, cy);
    cr.line_to(cx + sx, cy);
}

fn draw(g: Glyph, cr: &cairo::Context) {
    match g {
        Glyph::Play => {
            tri(cr, 0.84, 0.28);
            let _ = cr.fill();
        }
        Glyph::Pause => {
            cr.rectangle(0.3, 0.2, 0.14, 0.6);
            cr.rectangle(0.56, 0.2, 0.14, 0.6);
            let _ = cr.fill();
        }
        Glyph::Prev => {
            cr.rectangle(0.18, 0.2, 0.11, 0.6);
            tri(cr, 0.36, 0.86);
            let _ = cr.fill();
        }
        Glyph::Next => {
            cr.rectangle(0.71, 0.2, 0.11, 0.6);
            tri(cr, 0.64, 0.14);
            let _ = cr.fill();
        }
        Glyph::Fullscreen => {
            for (x, y) in [(0.14, 0.14), (0.86, 0.14), (0.14, 0.86), (0.86, 0.86)] {
                corner(cr, x, y, 0.26);
            }
            let _ = cr.stroke();
        }
        Glyph::Restore => {
            for (x, y) in [(0.35, 0.35), (0.65, 0.35), (0.35, 0.65), (0.65, 0.65)] {
                corner(cr, x, y, -0.22);
            }
            let _ = cr.stroke();
        }
        // A record, for the button that steps through pressings.
        Glyph::Pressing => {
            cr.arc(0.5, 0.5, 0.4, 0.0, std::f64::consts::TAU);
            let _ = cr.stroke();
            cr.arc(0.5, 0.5, 0.24, 0.0, std::f64::consts::TAU);
            let _ = cr.stroke();
            cr.arc(0.5, 0.5, 0.08, 0.0, std::f64::consts::TAU);
            let _ = cr.fill();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every glyph must put ink on the surface, inside its box. A typo in the
    /// path data shows up as an empty (or flooded) icon, which is exactly the
    /// blank-button failure this module exists to prevent.
    #[test]
    fn glyphs_ink_their_box() {
        let n = 32;
        for (name, g) in [
            ("play", Glyph::Play),
            ("pause", Glyph::Pause),
            ("prev", Glyph::Prev),
            ("next", Glyph::Next),
            ("fullscreen", Glyph::Fullscreen),
            ("restore", Glyph::Restore),
            ("pressing", Glyph::Pressing),
        ] {
            let mut surface = cairo::ImageSurface::create(cairo::Format::ARgb32, n, n).unwrap();
            let cr = cairo::Context::new(&surface).unwrap();
            cr.scale(n as f64, n as f64);
            cr.set_line_width(0.1);
            draw(g, &cr);
            drop(cr);
            let data = surface.data().unwrap();
            let inked = data.chunks(4).filter(|p| p[3] > 0).count() as f64 / (n * n) as f64;
            assert!(inked > 0.05 && inked < 0.7, "{name}: {inked:.3} of the box inked");
        }
    }
}
