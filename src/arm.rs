//! Tone arm geometry and drawing. The arm pivots at a fixed point and its
//! angle (degrees, screen coordinates, 0 = +x, 90 = straight down) is animated
//! by the UI: parked on its rest, or riding the groove that matches the
//! track's progress.

use gtk::cairo;
use std::f64::consts::PI;

/// Overlapping copies that stand in for a blurred shadow, and the ink each one
/// lays down. Three at 0.15 land near the old single copy at 0.38 where they
/// overlap, and taper where they do not.
const SHADOW_STEPS: usize = 3;
const SHADOW_INK: f64 = 0.15;

pub struct ArmGeometry {
    pub pivot: (f64, f64),
    pub length: f64,
    pub rest_angle: f64,
    /// Record centre when the record is fully out of the sleeve.
    pub record_center: (f64, f64),
    pub outer_groove: f64,
    pub inner_groove: f64,
}

impl ArmGeometry {
    /// Angle that puts the stylus on the groove at radius `r` from the record centre.
    fn angle_for_radius(&self, r: f64) -> f64 {
        let (px, py) = self.pivot;
        let (cx, cy) = self.record_center;
        let d = ((cx - px).powi(2) + (cy - py).powi(2)).sqrt();
        let l = self.length;
        let cos_t = ((l * l + d * d - r * r) / (2.0 * l * d)).clamp(-1.0, 1.0);
        let theta = cos_t.acos().to_degrees();
        let dir = (cy - py).atan2(cx - px).to_degrees();
        dir - theta
    }

    /// Stylus angle for playback progress in [0, 1].
    pub fn angle_for_progress(&self, progress: f64) -> f64 {
        let p = progress.clamp(0.0, 1.0);
        let r = self.outer_groove + (self.inner_groove - self.outer_groove) * p;
        self.angle_for_radius(r)
    }

    /// Distance from a point to the arm's tube at the given angle.
    pub fn distance_to_arm(&self, angle: f64, x: f64, y: f64) -> f64 {
        let (px, py) = self.pivot;
        let a = angle.to_radians();
        let (dx, dy) = (a.cos(), a.sin());
        let t = ((x - px) * dx + (y - py) * dy).clamp(-16.0, self.length + 16.0);
        let (qx, qy) = (px + dx * t, py + dy * t);
        ((x - qx).powi(2) + (y - qy).powi(2)).sqrt()
    }
}

/// Where the record is at this instant. The arm's shadow falls on the record
/// and nowhere else: off the disc there is nothing under the arm to catch it.
pub struct Disc {
    pub center: (f64, f64),
    pub radius: f64,
}

pub fn draw_arm(cr: &cairo::Context, g: &ArmGeometry, angle: f64, lifted: f64, disc: &Disc) {
    let (px, py) = g.pivot;

    // Shadow first, offset more when the arm is lifted off the record, and
    // smeared over three copies: a lifted arm throws a softer edge, and cairo
    // has no blur.
    cr.save().ok();
    cr.arc(disc.center.0, disc.center.1, disc.radius, 0.0, 2.0 * PI);
    cr.clip();
    let (sx, sy) = (1.5 + 2.5 * lifted, 3.0 + 4.0 * lifted);
    for step in 0..SHADOW_STEPS {
        let t = 0.55 + 0.45 * step as f64 / (SHADOW_STEPS - 1) as f64;
        cr.save().ok();
        cr.translate(px + sx * t, py + sy * t);
        cr.rotate(angle.to_radians());
        arm_shape(cr, g.length, Some(SHADOW_INK));
        cr.restore().ok();
    }
    cr.restore().ok();

    // Base plate (does not rotate).
    let base = cairo::RadialGradient::new(px - 3.0, py - 3.0, 2.0, px, py, 14.0);
    base.add_color_stop_rgb(0.0, 0.32, 0.32, 0.35);
    base.add_color_stop_rgb(1.0, 0.12, 0.12, 0.14);
    cr.set_source(&base).ok();
    cr.arc(px, py, 14.0, 0.0, 2.0 * PI);
    cr.fill().ok();
    cr.set_source_rgba(1.0, 1.0, 1.0, 0.10);
    cr.set_line_width(1.0);
    cr.arc(px, py, 13.5, 0.0, 2.0 * PI);
    cr.stroke().ok();

    cr.save().ok();
    cr.translate(px, py);
    cr.rotate(angle.to_radians());
    arm_shape(cr, g.length, None);
    cr.restore().ok();

    // Bearing cap.
    let cap = cairo::RadialGradient::new(px - 2.0, py - 2.0, 1.0, px, py, 6.5);
    cap.add_color_stop_rgb(0.0, 0.85, 0.85, 0.88);
    cap.add_color_stop_rgb(1.0, 0.45, 0.45, 0.48);
    cr.set_source(&cap).ok();
    cr.arc(px, py, 6.5, 0.0, 2.0 * PI);
    cr.fill().ok();
}

/// The arm along +x from the origin: counterweight behind, tube, headshell.
/// `shadow` is the ink for one shadow pass, or `None` for the arm itself.
fn arm_shape(cr: &cairo::Context, length: f64, shadow: Option<f64>) {
    let head_len = 20.0;
    let bend = 22f64.to_radians();
    let tube_len = length - head_len * 0.55;
    let shadow_ink = shadow.is_some();

    if let Some(ink) = shadow {
        cr.set_source_rgba(0.0, 0.0, 0.0, ink);
    }

    // Counterweight.
    round_rect(cr, -24.0, -6.0, 18.0, 12.0, 3.0);
    if !shadow_ink {
        let cw = cairo::LinearGradient::new(0.0, -6.0, 0.0, 6.0);
        cw.add_color_stop_rgb(0.0, 0.30, 0.30, 0.33);
        cw.add_color_stop_rgb(0.5, 0.16, 0.16, 0.18);
        cw.add_color_stop_rgb(1.0, 0.08, 0.08, 0.10);
        cr.set_source(&cw).ok();
    }
    cr.fill().ok();

    // Tube.
    cr.set_line_cap(cairo::LineCap::Round);
    cr.set_line_width(5.0);
    if !shadow_ink {
        let tube = cairo::LinearGradient::new(0.0, -2.5, 0.0, 2.5);
        tube.add_color_stop_rgb(0.0, 0.92, 0.92, 0.94);
        tube.add_color_stop_rgb(0.45, 0.70, 0.70, 0.73);
        tube.add_color_stop_rgb(1.0, 0.38, 0.38, 0.42);
        cr.set_source(&tube).ok();
    }
    cr.move_to(-6.0, 0.0);
    cr.line_to(tube_len, 0.0);
    cr.stroke().ok();

    // Headshell, bent slightly inward.
    cr.save().ok();
    cr.translate(tube_len, 0.0);
    cr.rotate(bend);
    round_rect(cr, -2.0, -4.0, head_len, 8.0, 2.5);
    if !shadow_ink {
        let hs = cairo::LinearGradient::new(0.0, -4.0, 0.0, 4.0);
        hs.add_color_stop_rgb(0.0, 0.22, 0.22, 0.25);
        hs.add_color_stop_rgb(1.0, 0.06, 0.06, 0.08);
        cr.set_source(&hs).ok();
    }
    cr.fill().ok();
    if !shadow_ink {
        // Cartridge and stylus.
        cr.set_source_rgb(0.55, 0.55, 0.58);
        round_rect(cr, head_len - 9.0, -2.5, 6.0, 5.0, 1.0);
        cr.fill().ok();
        cr.set_source_rgb(0.9, 0.9, 0.92);
        cr.arc(head_len - 2.0, 2.5, 1.2, 0.0, 2.0 * PI);
        cr.fill().ok();
    }
    cr.restore().ok();
}

fn round_rect(cr: &cairo::Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -PI / 2.0, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, PI / 2.0);
    cr.arc(x + r, y + h - r, r, PI / 2.0, PI);
    cr.arc(x + r, y + r, r, PI, 3.0 * PI / 2.0);
    cr.close_path();
}
