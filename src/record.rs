//! A `gdk::Paintable` that draws the vinyl: a pressed disc in one of several
//! styles, the album art as the centre label, and a spindle hole, all rotated
//! by `angle`.
//!
//! Rotating inside the paintable means a new angle only invalidates the
//! picture's render node; nothing else in the window is laid out or redrawn.

use gtk::cairo;
use gtk::gdk;
use gtk::glib;
use gtk::graphene;
use gtk::gsk;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use std::f64::consts::PI;

/// Label diameter as a fraction of the record diameter.
pub const LABEL_FRACTION: f64 = 56.0 / 150.0;
const HOLE_FRACTION: f64 = 7.0 / 150.0;

type Rgb = [f64; 3];

/// How the disc itself is coloured.
#[derive(Clone, Debug, PartialEq)]
pub enum VinylStyle {
    /// Classic black pressing.
    Black,
    /// A solid colour pressing.
    Solid(Rgb),
    /// Solid pressing in the album art's dominant colour.
    Art,
    /// Marbled pressing swirled from the album art's palette.
    Marble,
}

impl VinylStyle {
    /// Parse `black`, `marble`, `art`, or any CSS colour.
    pub fn parse(s: &str) -> Result<Self, String> {
        match s.to_ascii_lowercase().as_str() {
            "black" => Ok(Self::Black),
            "marble" => Ok(Self::Marble),
            "art" => Ok(Self::Art),
            other => gdk::RGBA::parse(other)
                .map(|c| Self::Solid([c.red() as f64, c.green() as f64, c.blue() as f64]))
                .map_err(|_| format!("'{s}' is not black, marble, art, or a CSS colour")),
        }
    }

    fn needs_art(&self) -> bool {
        matches!(self, Self::Art | Self::Marble)
    }
}

mod imp {
    use super::*;
    use std::cell::{Cell, RefCell};

    pub struct RecordPaintable {
        pub disc: RefCell<Option<gdk::Texture>>,
        pub label: RefCell<Option<gdk::Texture>>,
        pub style: RefCell<VinylStyle>,
        pub palette: RefCell<Vec<Rgb>>,
        pub seed: Cell<u32>,
        pub angle: Cell<f64>,
        pub size: Cell<i32>,
        pub scale: Cell<i32>,
    }

    impl Default for RecordPaintable {
        fn default() -> Self {
            Self {
                disc: RefCell::new(None),
                label: RefCell::new(None),
                style: RefCell::new(VinylStyle::Black),
                palette: RefCell::new(Vec::new()),
                seed: Cell::new(7),
                angle: Cell::new(0.0),
                size: Cell::new(150),
                scale: Cell::new(2),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for RecordPaintable {
        const NAME: &'static str = "VinylRecordPaintable";
        type Type = super::RecordPaintable;
        type Interfaces = (gdk::Paintable,);
    }

    impl ObjectImpl for RecordPaintable {}

    impl PaintableImpl for RecordPaintable {
        fn flags(&self) -> gdk::PaintableFlags {
            gdk::PaintableFlags::STATIC_SIZE
        }
        fn intrinsic_width(&self) -> i32 {
            self.size.get()
        }
        fn intrinsic_height(&self) -> i32 {
            self.size.get()
        }
        fn intrinsic_aspect_ratio(&self) -> f64 {
            1.0
        }

        fn snapshot(&self, snapshot: &gdk::Snapshot, width: f64, height: f64) {
            let snapshot = snapshot.downcast_ref::<gtk::Snapshot>().expect("gtk snapshot");
            let (cx, cy) = ((width / 2.0) as f32, (height / 2.0) as f32);

            snapshot.save();
            snapshot.translate(&graphene::Point::new(cx, cy));
            snapshot.rotate(self.angle.get() as f32);
            snapshot.translate(&graphene::Point::new(-cx, -cy));

            if let Some(disc) = self.disc.borrow().as_ref() {
                snapshot.append_texture(disc, &graphene::Rect::new(0.0, 0.0, width as f32, height as f32));
            }

            let label = (width * LABEL_FRACTION) as f32;
            let r = label / 2.0;
            let label_rect = graphene::Rect::new(cx - r, cy - r, label, label);
            snapshot.push_rounded_clip(&gsk::RoundedRect::from_rect(label_rect, r));
            match self.label.borrow().as_ref() {
                Some(tex) => snapshot.append_texture(tex, &cover_rect(tex, &label_rect)),
                None => snapshot.append_color(&gdk::RGBA::new(0.42, 0.16, 0.16, 1.0), &label_rect),
            }
            snapshot.pop();

            let hole_d = (width * HOLE_FRACTION) as f32;
            let hr = hole_d / 2.0;
            let hole = graphene::Rect::new(cx - hr, cy - hr, hole_d, hole_d);
            snapshot.push_rounded_clip(&gsk::RoundedRect::from_rect(hole, hr));
            snapshot.append_color(&gdk::RGBA::new(0.04, 0.04, 0.05, 1.0), &hole);
            snapshot.pop();

            snapshot.restore();
        }
    }
}

glib::wrapper! {
    pub struct RecordPaintable(ObjectSubclass<imp::RecordPaintable>) @implements gdk::Paintable;
}

impl RecordPaintable {
    pub fn new(size: i32, scale: i32, style: VinylStyle) -> Self {
        let obj: Self = glib::Object::new();
        obj.imp().size.set(size);
        obj.imp().scale.set(scale.max(1));
        obj.imp().style.replace(style);
        obj.render();
        obj
    }

    pub fn set_angle(&self, angle: f64) {
        if (self.imp().angle.get() - angle).abs() > 1e-3 {
            self.imp().angle.set(angle);
            self.invalidate_contents();
        }
    }

    /// New album art (or none). `seed` varies the marbling per album.
    pub fn set_label(&self, tex: Option<gdk::Texture>, seed: u32) {
        let imp = self.imp();
        let palette = tex.as_ref().map(palette_from_texture).unwrap_or_default();
        imp.label.replace(tex);
        imp.palette.replace(palette);
        imp.seed.set(seed);
        if imp.style.borrow().needs_art() {
            self.render();
        }
        self.invalidate_contents();
    }

    pub fn set_style(&self, style: VinylStyle) {
        if *self.imp().style.borrow() != style {
            self.imp().style.replace(style);
            self.render();
            self.invalidate_contents();
        }
    }

    fn render(&self) {
        let imp = self.imp();
        let style = imp.style.borrow().clone();
        let palette = imp.palette.borrow().clone();
        let tex = render_disc(imp.size.get(), imp.scale.get(), &style, &palette, imp.seed.get());
        imp.disc.replace(Some(tex));
    }
}

/// Scale a texture to cover `rect`, centred, like CSS `object-fit: cover`.
fn cover_rect(tex: &gdk::Texture, rect: &graphene::Rect) -> graphene::Rect {
    let (tw, th) = (tex.width() as f32, tex.height() as f32);
    let scale = (rect.width() / tw).max(rect.height() / th);
    let (w, h) = (tw * scale, th * scale);
    graphene::Rect::new(
        rect.x() + (rect.width() - w) / 2.0,
        rect.y() + (rect.height() - h) / 2.0,
        w,
        h,
    )
}

// ---- palette extraction ---------------------------------------------------

/// Up to three distinct, dominant colours of the art, most common first.
fn palette_from_texture(tex: &gdk::Texture) -> Vec<Rgb> {
    let (w, h) = (tex.width() as usize, tex.height() as usize);
    if w == 0 || h == 0 {
        return Vec::new();
    }
    let stride = w * 4;
    let mut buf = vec![0u8; stride * h];
    tex.download(&mut buf, stride);

    // Sample a grid of at most ~48x48 pixels. GDK downloads B8G8R8A8 premultiplied.
    let step = (w.max(h) / 48).max(1);
    let mut samples: Vec<Rgb> = Vec::new();
    for y in (0..h).step_by(step) {
        for x in (0..w).step_by(step) {
            let i = y * stride + x * 4;
            let a = buf[i + 3] as f64 / 255.0;
            if a < 0.5 {
                continue;
            }
            samples.push([
                buf[i + 2] as f64 / 255.0 / a,
                buf[i + 1] as f64 / 255.0 / a,
                buf[i] as f64 / 255.0 / a,
            ]);
        }
    }
    if samples.is_empty() {
        return Vec::new();
    }

    // k-means with farthest-point seeding.
    let k = 6.min(samples.len());
    let mut centers: Vec<Rgb> = vec![samples[samples.len() / 2]];
    while centers.len() < k {
        let far = samples
            .iter()
            .max_by(|a, b| min_dist(a, &centers).partial_cmp(&min_dist(b, &centers)).unwrap())
            .unwrap();
        centers.push(*far);
    }
    let mut counts = vec![0usize; k];
    for _ in 0..10 {
        let mut sums = vec![[0.0; 3]; k];
        counts.iter_mut().for_each(|c| *c = 0);
        for s in &samples {
            let (ci, _) = nearest(s, &centers);
            counts[ci] += 1;
            for c in 0..3 {
                sums[ci][c] += s[c];
            }
        }
        for i in 0..k {
            if counts[i] > 0 {
                centers[i] = [sums[i][0] / counts[i] as f64, sums[i][1] / counts[i] as f64, sums[i][2] / counts[i] as f64];
            }
        }
    }

    let mut ranked: Vec<(usize, Rgb)> = counts.into_iter().zip(centers).collect();
    ranked.sort_by(|a, b| b.0.cmp(&a.0));
    let mut out: Vec<Rgb> = Vec::new();
    for (_, c) in ranked {
        if out.iter().all(|o| dist(o, &c) > 0.18) {
            out.push(c);
        }
        if out.len() == 3 {
            break;
        }
    }
    out
}

fn dist(a: &Rgb, b: &Rgb) -> f64 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}
fn min_dist(s: &Rgb, centers: &[Rgb]) -> f64 {
    centers.iter().map(|c| dist(s, c)).fold(f64::MAX, f64::min)
}
fn nearest(s: &Rgb, centers: &[Rgb]) -> (usize, f64) {
    let mut best = (0, f64::MAX);
    for (i, c) in centers.iter().enumerate() {
        let d = dist(s, c);
        if d < best.1 {
            best = (i, d);
        }
    }
    best
}

fn luma(c: &Rgb) -> f64 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

fn scale_rgb(c: &Rgb, f: f64) -> Rgb {
    [(c[0] * f).clamp(0.0, 1.0), (c[1] * f).clamp(0.0, 1.0), (c[2] * f).clamp(0.0, 1.0)]
}

/// Push a colour towards something that reads well as pressed vinyl: a bit
/// more saturated, never blown out, never pitch black.
fn vinylize(c: &Rgb) -> Rgb {
    let g = luma(c);
    let mut out = [0.0; 3];
    for i in 0..3 {
        out[i] = g + (c[i] - g) * 1.35;
    }
    let l = luma(&out);
    let target = l.clamp(0.16, 0.62);
    if l > 1e-4 {
        out = scale_rgb(&out, target / l);
    } else {
        out = [target; 3];
    }
    out
}

fn lerp3(a: &Rgb, b: &Rgb, t: f64) -> Rgb {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

/// Three colours to marble with for the given style.
fn marble_palette(style: &VinylStyle, art: &[Rgb]) -> [Rgb; 3] {
    let black = [0.075, 0.075, 0.085];
    match style {
        VinylStyle::Black => [black, [0.055, 0.055, 0.065], [0.10, 0.10, 0.11]],
        VinylStyle::Solid(c) => {
            let c = vinylize(c);
            [c, scale_rgb(&c, 0.82), scale_rgb(&c, 1.16)]
        }
        VinylStyle::Art => match art.first() {
            Some(c) => {
                let c = vinylize(c);
                [c, scale_rgb(&c, 0.82), scale_rgb(&c, 1.16)]
            }
            None => [black, [0.055, 0.055, 0.065], [0.10, 0.10, 0.11]],
        },
        VinylStyle::Marble => {
            let v: Vec<Rgb> = art.iter().map(vinylize).collect();
            match v.len() {
                0 => [black, [0.055, 0.055, 0.065], [0.10, 0.10, 0.11]],
                1 => [v[0], scale_rgb(&v[0], 0.55), scale_rgb(&v[0], 1.4)],
                2 => [v[0], v[1], lerp3(&v[0], &v[1], 0.5).map(|x| (x * 1.3).min(1.0))],
                _ => [v[0], v[1], v[2]],
            }
        }
    }
}

// ---- noise ----------------------------------------------------------------

fn hash(x: i32, y: i32, seed: u32) -> f64 {
    let mut h = (x as u32)
        .wrapping_mul(374_761_393)
        ^ (y as u32).wrapping_mul(668_265_263)
        ^ seed.wrapping_mul(2_246_822_519);
    h = (h ^ (h >> 13)).wrapping_mul(1_274_126_177);
    ((h ^ (h >> 16)) & 0xffff) as f64 / 65535.0
}

fn value_noise(x: f64, y: f64, seed: u32) -> f64 {
    let (xi, yi) = (x.floor(), y.floor());
    let (fx, fy) = (x - xi, y - yi);
    let (sx, sy) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
    let (xi, yi) = (xi as i32, yi as i32);
    let a = hash(xi, yi, seed);
    let b = hash(xi + 1, yi, seed);
    let c = hash(xi, yi + 1, seed);
    let d = hash(xi + 1, yi + 1, seed);
    let top = a + (b - a) * sx;
    let bot = c + (d - c) * sx;
    top + (bot - top) * sy
}

fn fbm(x: f64, y: f64, seed: u32) -> f64 {
    let mut sum = 0.0;
    let mut amp = 0.5;
    let (mut fx, mut fy) = (x, y);
    for i in 0..4 {
        sum += amp * value_noise(fx, fy, seed.wrapping_add(i * 131));
        fx *= 2.03;
        fy *= 1.97;
        amp *= 0.5;
    }
    sum
}

/// Domain-warped noise in ~[0,1]: the swirl of a marbled pressing.
fn marble(x: f64, y: f64, seed: u32) -> (f64, f64) {
    let q0 = fbm(x, y, seed);
    let q1 = fbm(x + 5.2, y + 1.3, seed + 7);
    let r0 = fbm(x + 4.0 * q0 + 1.7, y + 4.0 * q1 + 9.2, seed + 13);
    let r1 = fbm(x + 4.0 * q0 + 8.3, y + 4.0 * q1 + 2.8, seed + 19);
    let n = fbm(x + 4.0 * r0, y + 4.0 * r1, seed + 23);
    let vein = fbm(x * 2.0 + 3.0 * r1, y * 2.0 + 3.0 * r0, seed + 29);
    (((n - 0.28) / 0.44).clamp(0.0, 1.0), vein)
}

fn smoothstep(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

// ---- disc rendering -------------------------------------------------------

/// Draw the disc once with Cairo and upload it as a texture.
fn render_disc(size: i32, scale: i32, style: &VinylStyle, art: &[Rgb], seed: u32) -> gdk::Texture {
    let px = size * scale;
    let colors = marble_palette(style, art);
    let subtle = matches!(style, VinylStyle::Black | VinylStyle::Solid(_) | VinylStyle::Art);

    // Body pixels.
    let mut body = cairo::ImageSurface::create(cairo::Format::ARgb32, px, px).expect("cairo surface");
    {
        let stride = body.stride() as usize;
        let mut data = body.data().expect("surface data");
        let freq = 3.2 / px as f64;
        for y in 0..px as usize {
            for x in 0..px as usize {
                let (n, vein) = marble(x as f64 * freq, y as f64 * freq, seed);
                let mut c = if n < 0.5 {
                    lerp3(&colors[0], &colors[1], smoothstep(n / 0.5))
                } else {
                    lerp3(&colors[0], &colors[2], smoothstep((n - 0.5) / 0.5))
                };
                if subtle {
                    // Solid pressings only get a whisper of mottling.
                    c = lerp3(&colors[0], &c, 0.35);
                } else {
                    let v = (vein - 0.5).abs();
                    if v < 0.035 {
                        let t = (1.0 - v / 0.035) * 0.45;
                        c = lerp3(&c, &[1.0, 1.0, 1.0], t * 0.6);
                    }
                }
                let i = y * stride + x * 4;
                data[i] = (c[2] * 255.0) as u8;
                data[i + 1] = (c[1] * 255.0) as u8;
                data[i + 2] = (c[0] * 255.0) as u8;
                data[i + 3] = 255;
            }
        }
    }
    body.mark_dirty();

    let mut surface = cairo::ImageSurface::create(cairo::Format::ARgb32, px, px).expect("cairo surface");
    {
        let cr = cairo::Context::new(&surface).expect("cairo context");
        cr.scale(scale as f64, scale as f64);
        let c = size as f64 / 2.0;
        let r = c;

        cr.arc(c, c, r, 0.0, 2.0 * PI);
        cr.clip();
        cr.save().ok();
        cr.scale(1.0 / scale as f64, 1.0 / scale as f64);
        cr.set_source_surface(&body, 0.0, 0.0).ok();
        cr.paint().ok();
        cr.restore().ok();

        // Pressing shade: darker toward the rim, a soft highlight off-centre.
        let shade = cairo::RadialGradient::new(c, c, r * 0.3, c, c, r);
        shade.add_color_stop_rgba(0.0, 0.0, 0.0, 0.0, 0.0);
        shade.add_color_stop_rgba(0.75, 0.0, 0.0, 0.0, 0.12);
        shade.add_color_stop_rgba(1.0, 0.0, 0.0, 0.0, 0.42);
        cr.set_source(&shade).ok();
        cr.paint().ok();

        // Grooves. Spacing grows with the square root of the size so a big
        // record gets more grooves, not just wider ones.
        let k = (size as f64 / 150.0).max(0.5);
        let ks = k.sqrt();
        let label_r = size as f64 * LABEL_FRACTION / 2.0 + 3.0 * k;
        let (dark, light) = if subtle { (0.14, 0.035) } else { (0.16, 0.06) };
        let mut gr = label_r + 3.0 * k;
        let mut i = 0;
        while gr < r - 3.0 * k {
            let (a, w) = if i % 7 == 0 { (dark, 0.8 * ks) } else { (dark * 0.35, 0.6 * ks) };
            cr.set_source_rgba(0.0, 0.0, 0.0, a);
            cr.set_line_width(w);
            cr.arc(c, c, gr, 0.0, 2.0 * PI);
            cr.stroke().ok();
            cr.set_source_rgba(1.0, 1.0, 1.0, light * 0.5);
            cr.set_line_width(0.4 * ks);
            cr.arc(c, c, gr + 0.7 * ks, 0.0, 2.0 * PI);
            cr.stroke().ok();
            gr += 1.55 * ks;
            i += 1;
        }

        // Rim and label ring.
        cr.set_source_rgba(1.0, 1.0, 1.0, 0.10);
        cr.set_line_width(ks);
        cr.arc(c, c, r - 0.5 * ks, 0.0, 2.0 * PI);
        cr.stroke().ok();
        let ring = scale_rgb(&colors[1], 0.7);
        cr.set_source_rgb(ring[0], ring[1], ring[2]);
        cr.arc(c, c, label_r, 0.0, 2.0 * PI);
        cr.fill().ok();
    }
    surface.flush();
    let stride = surface.stride() as usize;
    let data = surface.data().expect("surface data");
    let bytes = glib::Bytes::from(&data[..]);
    gdk::MemoryTexture::new(px, px, gdk::MemoryFormat::B8g8r8a8Premultiplied, &bytes, stride).upcast()
}
