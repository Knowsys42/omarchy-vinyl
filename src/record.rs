//! A `gdk::Paintable` that draws the vinyl: a pressed disc in one of many
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
use std::fmt;

/// Label diameter as a fraction of the record diameter.
pub const LABEL_FRACTION: f64 = 56.0 / 150.0;
const HOLE_FRACTION: f64 = 7.0 / 150.0;

pub type Rgb = [f64; 3];

// ---- styles ---------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pattern {
    Solid,
    Marble,
    Splatter,
    Split,
    Tri,
    Starburst,
    Galaxy,
    Smoke,
    Picture,
    Rainbow,
    Gold,
    Clear,
}

impl Pattern {
    const ALL: [(Pattern, &'static str); 12] = [
        (Pattern::Solid, "solid"),
        (Pattern::Marble, "marble"),
        (Pattern::Splatter, "splatter"),
        (Pattern::Split, "split"),
        (Pattern::Tri, "tri"),
        (Pattern::Starburst, "starburst"),
        (Pattern::Galaxy, "galaxy"),
        (Pattern::Smoke, "smoke"),
        (Pattern::Picture, "picture"),
        (Pattern::Rainbow, "rainbow"),
        (Pattern::Gold, "gold"),
        (Pattern::Clear, "clear"),
    ];

    fn key(self) -> &'static str {
        Self::ALL.iter().find(|(p, _)| *p == self).map(|(_, k)| *k).unwrap()
    }

    fn from_key(s: &str) -> Option<Self> {
        Self::ALL.iter().find(|(_, k)| *k == s).map(|(p, _)| *p)
    }

    /// Patterns whose look doesn't depend on the palette.
    fn ignores_palette(self) -> bool {
        matches!(self, Self::Smoke | Self::Picture | Self::Rainbow | Self::Gold | Self::Clear)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Palette {
    Black,
    /// Dominant colours of the album art.
    Art,
    /// The current Omarchy theme.
    Theme,
    Colors(Vec<Rgb>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct VinylStyle {
    pub pattern: Pattern,
    pub palette: Palette,
}

pub struct Preset {
    pub key: &'static str,
    pub label: &'static str,
    pub style: VinylStyle,
}

fn style(pattern: Pattern, palette: Palette) -> VinylStyle {
    VinylStyle { pattern, palette }
}

/// The combos the style button cycles through.
pub fn presets() -> Vec<Preset> {
    use Palette::*;
    use Pattern::*;
    let p = |key, label, pattern, palette| Preset { key, label, style: style(pattern, palette) };
    vec![
        p("black", "Black", Solid, Black),
        p("marble", "Marble", Marble, Art),
        p("splatter", "Splatter", Splatter, Art),
        p("split", "Split", Split, Art),
        p("tri", "Tri-colour", Tri, Art),
        p("starburst", "Starburst", Starburst, Art),
        p("galaxy", "Galaxy", Galaxy, Art),
        p("smoke", "Smoke", Smoke, Black),
        p("picture", "Picture disc", Picture, Art),
        p("rainbow", "Rainbow", Rainbow, Black),
        p("gold", "Gold", Gold, Black),
        p("clear", "Clear", Clear, Black),
        p("glow", "Glow in the dark", Solid, Colors(vec![[0.80, 0.93, 0.52]])),
        p("omarchy", "Omarchy theme", Marble, Theme),
    ]
}

fn parse_color(s: &str) -> Option<Rgb> {
    gdk::RGBA::parse(s).ok().map(|c| [c.red() as f64, c.green() as f64, c.blue() as f64])
}

fn hex(c: &Rgb) -> String {
    format!("#{:02x}{:02x}{:02x}", (c[0] * 255.0).round() as u8, (c[1] * 255.0).round() as u8, (c[2] * 255.0).round() as u8)
}

impl VinylStyle {
    /// Accepts a preset name (`black`, `marble`, `omarchy`, ...), a CSS colour
    /// (a solid pressing), or `pattern:palette` where the palette is `art`,
    /// `theme`, `black`, or CSS colours separated by commas, e.g.
    /// `splatter:theme` or `split:#1e90ff,white`.
    pub fn parse(s: &str) -> Result<Self, String> {
        let s = s.trim().to_ascii_lowercase();
        if let Some(p) = presets().into_iter().find(|p| p.key == s) {
            return Ok(p.style);
        }
        if s == "art" {
            return Ok(style(Pattern::Solid, Palette::Art));
        }
        if let Some((pat, pal)) = s.split_once(':') {
            let pattern = Pattern::from_key(pat).ok_or_else(|| {
                let names: Vec<&str> = Pattern::ALL.iter().map(|(_, k)| *k).collect();
                format!("unknown pattern '{pat}'; try one of {}", names.join(", "))
            })?;
            let palette = match pal {
                "black" => Palette::Black,
                "art" => Palette::Art,
                "theme" | "omarchy" => Palette::Theme,
                list => {
                    let colors: Option<Vec<Rgb>> = list.split(',').map(|c| parse_color(c.trim())).collect();
                    Palette::Colors(colors.ok_or_else(|| format!("'{list}' is not a list of CSS colours"))?)
                }
            };
            return Ok(style(pattern, palette));
        }
        parse_color(&s)
            .map(|c| style(Pattern::Solid, Palette::Colors(vec![c])))
            .ok_or_else(|| {
                let names: Vec<&str> = presets().iter().map(|p| p.key).collect();
                format!("'{s}' is not a preset ({}), a CSS colour, or pattern:palette", names.join(", "))
            })
    }

    /// Human name: the preset's label, or the pattern:palette form.
    pub fn label(&self) -> String {
        presets()
            .into_iter()
            .find(|p| p.style == *self)
            .map(|p| p.label.to_string())
            .unwrap_or_else(|| self.to_string())
    }

    fn needs_art(&self) -> bool {
        self.palette == Palette::Art || self.pattern == Pattern::Picture
    }

    fn needs_theme(&self) -> bool {
        self.palette == Palette::Theme
    }
}

impl fmt::Display for VinylStyle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(p) = presets().into_iter().find(|p| p.style == *self) {
            return write!(f, "{}", p.key);
        }
        let palette = match &self.palette {
            Palette::Black => "black".to_string(),
            Palette::Art => "art".to_string(),
            Palette::Theme => "theme".to_string(),
            Palette::Colors(cs) => cs.iter().map(hex).collect::<Vec<_>>().join(","),
        };
        write!(f, "{}:{}", self.pattern.key(), palette)
    }
}

/// Album art pixels, downsampled, straight RGB.
#[derive(Clone)]
pub struct ArtPixels {
    w: usize,
    h: usize,
    rgb: Vec<u8>,
}

/// Where the lamp sits, in degrees around the disc, and how far the highlight
/// swings either side of that as a warped pressing turns.
const SHEEN_ANGLE: f32 = 42.0;
const SHEEN_WOBBLE: f32 = 7.0;

/// One turn around the disc: a bright lobe at the light, a dimmer one opposite
/// it, and darkness across the grooves that face away.
fn sheen_stops() -> [gsk::ColorStop; 7] {
    let white = |a: f32| gdk::RGBA::new(1.0, 1.0, 1.0, a);
    let stop = |offset: f32, a: f32| gsk::ColorStop::new(offset, white(a));
    [
        stop(0.00, 0.15),
        stop(0.11, 0.0),
        stop(0.39, 0.0),
        stop(0.50, 0.09),
        stop(0.61, 0.0),
        stop(0.89, 0.0),
        stop(1.00, 0.15),
    ]
}

mod imp {
    use super::*;
    use std::cell::{Cell, RefCell};

    pub struct RecordPaintable {
        pub disc: RefCell<Option<gdk::Texture>>,
        pub label: RefCell<Option<gdk::Texture>>,
        pub style: RefCell<VinylStyle>,
        pub art_colors: RefCell<Vec<Rgb>>,
        pub art_pixels: RefCell<Option<ArtPixels>>,
        pub theme_colors: RefCell<Vec<Rgb>>,
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
                style: RefCell::new(style(Pattern::Solid, Palette::Black)),
                art_colors: RefCell::new(Vec::new()),
                art_pixels: RefCell::new(None),
                theme_colors: RefCell::new(Vec::new()),
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

            // Picture discs carry the art edge to edge; no paper label.
            if self.style.borrow().pattern != Pattern::Picture {
                let label = (width * LABEL_FRACTION) as f32;
                let r = label / 2.0;
                let label_rect = graphene::Rect::new(cx - r, cy - r, label, label);
                snapshot.push_rounded_clip(&gsk::RoundedRect::from_rect(label_rect, r));
                match self.label.borrow().as_ref() {
                    Some(tex) => snapshot.append_texture(tex, &cover_rect(tex, &label_rect)),
                    None => snapshot.append_color(&gdk::RGBA::new(0.42, 0.16, 0.16, 1.0), &label_rect),
                }
                snapshot.pop();
            }

            let hole_d = (width * HOLE_FRACTION) as f32;
            let hr = hole_d / 2.0;
            let hole = graphene::Rect::new(cx - hr, cy - hr, hole_d, hole_d);
            snapshot.push_rounded_clip(&gsk::RoundedRect::from_rect(hole, hr));
            snapshot.append_color(&gdk::RGBA::new(0.04, 0.04, 0.05, 1.0), &hole);
            snapshot.pop();

            snapshot.restore();

            // Specular sheen, drawn outside the rotation: the lamp stays put
            // while the record turns under it. Concentric grooves throw the
            // reflection into two lobes opposite each other along the light
            // axis, which is a conic gradient. No pressing is perfectly flat,
            // so the axis wobbles once per revolution.
            let disc = graphene::Rect::new(0.0, 0.0, width as f32, height as f32);
            let wobble = SHEEN_WOBBLE * (self.angle.get() * PI / 180.0).sin() as f32;
            snapshot.push_rounded_clip(&gsk::RoundedRect::from_rect(disc, cx.min(cy)));
            snapshot.append_conic_gradient(
                &disc,
                &graphene::Point::new(cx, cy),
                SHEEN_ANGLE + wobble,
                &sheen_stops(),
            );
            snapshot.pop();
        }
    }
}

glib::wrapper! {
    pub struct RecordPaintable(ObjectSubclass<imp::RecordPaintable>) @implements gdk::Paintable;
}

impl RecordPaintable {
    pub fn new(size: i32, scale: i32, style: VinylStyle, theme: Vec<Rgb>) -> Self {
        let obj: Self = glib::Object::new();
        obj.imp().size.set(size);
        obj.imp().scale.set(scale.max(1));
        obj.imp().style.replace(style);
        obj.imp().theme_colors.replace(theme);
        obj.render();
        obj
    }

    pub fn set_angle(&self, angle: f64) {
        if (self.imp().angle.get() - angle).abs() > 1e-3 {
            self.imp().angle.set(angle);
            self.invalidate_contents();
        }
    }

    /// New album art (or none). `seed` varies the pattern per album.
    pub fn set_label(&self, tex: Option<gdk::Texture>, seed: u32) {
        let imp = self.imp();
        let pixels = tex.as_ref().map(download_art);
        let colors = pixels.as_ref().map(palette_from_pixels).unwrap_or_default();
        imp.label.replace(tex);
        imp.art_pixels.replace(pixels);
        imp.art_colors.replace(colors);
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

    pub fn set_theme(&self, colors: Vec<Rgb>) {
        let imp = self.imp();
        if *imp.theme_colors.borrow() == colors {
            return;
        }
        imp.theme_colors.replace(colors);
        if imp.style.borrow().needs_theme() {
            self.render();
            self.invalidate_contents();
        }
    }

    fn render(&self) {
        let imp = self.imp();
        let style = imp.style.borrow().clone();
        let colors = resolve_palette(&style, &imp.art_colors.borrow(), &imp.theme_colors.borrow());
        let art = imp.art_pixels.borrow();
        let tex = render_disc(imp.size.get(), imp.scale.get(), style.pattern, &colors, art.as_ref(), imp.seed.get());
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

// ---- art sampling ---------------------------------------------------------

/// Pull the art down to at most 512px on its long side, straight RGB.
fn download_art(tex: &gdk::Texture) -> ArtPixels {
    let (w, h) = (tex.width() as usize, tex.height() as usize);
    let stride = w * 4;
    let mut buf = vec![0u8; stride * h.max(1)];
    tex.download(&mut buf, stride);
    let step = (w.max(h) / 512).max(1);
    let (ow, oh) = (w.div_ceil(step), h.div_ceil(step));
    let mut rgb = Vec::with_capacity(ow * oh * 3);
    for y in (0..h).step_by(step) {
        for x in (0..w).step_by(step) {
            let i = y * stride + x * 4;
            let a = buf[i + 3] as f64 / 255.0;
            let un = |v: u8| if a > 0.0 { ((v as f64 / 255.0 / a).min(1.0) * 255.0) as u8 } else { 0 };
            // GDK downloads B8G8R8A8 premultiplied.
            rgb.push(un(buf[i + 2]));
            rgb.push(un(buf[i + 1]));
            rgb.push(un(buf[i]));
        }
    }
    ArtPixels { w: ow, h: oh, rgb }
}

/// Up to three distinct, dominant colours of the art, most common first.
fn palette_from_pixels(art: &ArtPixels) -> Vec<Rgb> {
    let step = (art.w.max(art.h) / 48).max(1);
    let mut samples: Vec<Rgb> = Vec::new();
    for y in (0..art.h).step_by(step) {
        for x in (0..art.w).step_by(step) {
            let i = (y * art.w + x) * 3;
            samples.push([art.rgb[i] as f64 / 255.0, art.rgb[i + 1] as f64 / 255.0, art.rgb[i + 2] as f64 / 255.0]);
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

    // Dominant colour first, then whichever remaining clusters (with some
    // weight behind them) are farthest from what's already chosen, so
    // two-tone patterns get contrast rather than three shades of the same.
    let total: usize = counts.iter().sum();
    let mut ranked: Vec<(usize, Rgb)> = counts.into_iter().zip(centers).collect();
    ranked.sort_by(|a, b| b.0.cmp(&a.0));
    let mut out: Vec<Rgb> = vec![ranked[0].1];
    let mut pool: Vec<(usize, Rgb)> = ranked.into_iter().skip(1).filter(|(n, _)| *n * 40 >= total).collect();
    while out.len() < 3 && !pool.is_empty() {
        let (i, _) = pool
            .iter()
            .enumerate()
            .max_by(|a, b| min_dist(&a.1 .1, &out).partial_cmp(&min_dist(&b.1 .1, &out)).unwrap())
            .unwrap();
        let (_, c) = pool.remove(i);
        if min_dist(&c, &out) > 0.18 {
            out.push(c);
        } else {
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

// ---- colour helpers -------------------------------------------------------

fn luma(c: &Rgb) -> f64 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

fn scale_rgb(c: &Rgb, f: f64) -> Rgb {
    [(c[0] * f).clamp(0.0, 1.0), (c[1] * f).clamp(0.0, 1.0), (c[2] * f).clamp(0.0, 1.0)]
}

fn lerp3(a: &Rgb, b: &Rgb, t: f64) -> Rgb {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
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

fn hsv(h: f64, s: f64, v: f64) -> Rgb {
    let h = h.rem_euclid(1.0) * 6.0;
    let i = h.floor() as i32;
    let f = h - h.floor();
    let p = v * (1.0 - s);
    let q = v * (1.0 - s * f);
    let t = v * (1.0 - s * (1.0 - f));
    match i {
        0 => [v, t, p],
        1 => [q, v, p],
        2 => [p, v, t],
        3 => [p, q, v],
        4 => [t, p, v],
        _ => [v, p, q],
    }
}

const BLACK_TRIO: [Rgb; 3] = [[0.075, 0.075, 0.085], [0.055, 0.055, 0.065], [0.10, 0.10, 0.11]];

/// Three working colours: main, secondary, accent.
fn expand(v: &[Rgb]) -> [Rgb; 3] {
    let v: Vec<Rgb> = v.iter().map(vinylize).collect();
    match v.len() {
        0 => BLACK_TRIO,
        1 => [v[0], scale_rgb(&v[0], 0.55), scale_rgb(&v[0], 1.45)],
        2 => [v[0], v[1], scale_rgb(&lerp3(&v[0], &v[1], 0.5), 1.35)],
        _ => [v[0], v[1], v[2]],
    }
}

fn resolve_palette(style: &VinylStyle, art: &[Rgb], theme: &[Rgb]) -> [Rgb; 3] {
    match &style.palette {
        Palette::Black => BLACK_TRIO,
        Palette::Art => expand(art),
        Palette::Theme => expand(theme),
        Palette::Colors(cs) => expand(cs),
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

/// Two-octave noise: organic blobs without the cost of full fbm.
fn blobs(x: f64, y: f64, seed: u32) -> f64 {
    0.65 * value_noise(x, y, seed) + 0.35 * value_noise(x * 2.1 + 3.7, y * 2.1 + 1.9, seed + 77)
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

fn marble_color(n: f64, vein: f64, c: &[Rgb; 3]) -> Rgb {
    let mut col = if n < 0.5 {
        lerp3(&c[0], &c[1], smoothstep(n / 0.5))
    } else {
        lerp3(&c[0], &c[2], smoothstep((n - 0.5) / 0.5))
    };
    let v = (vein - 0.5).abs();
    if v < 0.035 {
        let t = (1.0 - v / 0.035) * 0.45;
        col = lerp3(&col, &[1.0, 1.0, 1.0], t * 0.6);
    }
    col
}

// ---- disc rendering -------------------------------------------------------

/// Colour and alpha of the disc body at one pixel.
#[allow(clippy::too_many_arguments)]
fn body_pixel(
    pattern: Pattern,
    c: &[Rgb; 3],
    art: Option<&ArtPixels>,
    seed: u32,
    px: usize,
    x: usize,
    y: usize,
    fx: f64,
    fy: f64,
) -> (Rgb, f64) {
    let half = px as f64 / 2.0;
    let u = (x as f64 + 0.5 - half) / half;
    let v = (y as f64 + 0.5 - half) / half;
    let rad = (u * u + v * v).sqrt();
    let theta = v.atan2(u);
    let turn = theta / (2.0 * PI) + 0.5; // 0..1 around the disc

    match pattern {
        Pattern::Solid => {
            let (n, _) = marble(fx, fy, seed);
            let mottle = lerp3(&c[1], &c[2], n);
            (lerp3(&c[0], &mottle, 0.3), 1.0)
        }
        Pattern::Marble => {
            let (n, vein) = marble(fx, fy, seed);
            (marble_color(n, vein, c), 1.0)
        }
        Pattern::Splatter => {
            let mut col = c[0];
            let b1 = blobs(fx * 6.0, fy * 6.0, seed + 41);
            col = lerp3(&col, &c[1], smoothstep((b1 - 0.62) / 0.05));
            let b2 = blobs(fx * 8.0 + 3.3, fy * 8.0 + 7.1, seed + 43);
            col = lerp3(&col, &c[2], smoothstep((b2 - 0.66) / 0.05));
            let specks = value_noise(fx * 28.0, fy * 28.0, seed + 47);
            col = lerp3(&col, &c[2], smoothstep((specks - 0.80) / 0.06));
            (col, 1.0)
        }
        Pattern::Split => {
            let wobble = (fbm(fx * 1.5, fy * 1.5, seed + 51) - 0.5) * 0.5;
            let t = smoothstep((u + wobble) / 0.04 + 0.5);
            (lerp3(&c[0], &c[1], t), 1.0)
        }
        Pattern::Tri => {
            let s = turn * 3.0 + (fbm(fx * 2.0, fy * 2.0, seed + 61) - 0.5) * 0.7;
            let i = s.floor().rem_euclid(3.0) as usize;
            let f = s - s.floor();
            let prev = c[(i + 2) % 3];
            let col = if f < 0.05 { lerp3(&prev, &c[i], smoothstep(f / 0.05)) } else { c[i] };
            (col, 1.0)
        }
        Pattern::Starburst => {
            let n = 14.0;
            let s = turn * n + (fbm(fx * 2.0, fy * 2.0, seed + 71) - 0.5) * 0.5;
            let i = s.floor() as i64;
            let f = s - s.floor();
            let (a, b) = if i.rem_euclid(2) == 0 { (c[0], c[1]) } else { (c[1], c[0]) };
            let edge = f.min(1.0 - f);
            let col = lerp3(&lerp3(&a, &b, 0.5), &a, smoothstep(edge / 0.06));
            let streak = fbm(fx * 0.8, fy * 5.0, seed + 73);
            (lerp3(&col, &c[2], smoothstep((streak - 0.68) / 0.08) * 0.6), 1.0)
        }
        Pattern::Galaxy => {
            let deep = [0.02, 0.02, 0.05];
            let mut col = lerp3(&deep, &scale_rgb(&c[0], 0.35), 0.5);
            let (n, _) = marble(fx * 1.3, fy * 1.3, seed + 81);
            col = lerp3(&col, &scale_rgb(&c[1], 0.95), smoothstep((n - 0.45) / 0.35) * 0.65);
            let n2 = fbm(fx * 2.2 + 9.0, fy * 2.2 + 4.0, seed + 83);
            col = lerp3(&col, &c[2], smoothstep((n2 - 0.55) / 0.25) * 0.5);
            let star = hash(x as i32, y as i32, seed + 91);
            if star > 0.9965 {
                let bright = 0.5 + 0.5 * hash(y as i32, x as i32, seed + 92);
                col = lerp3(&col, &[1.0, 1.0, 1.0], bright);
            }
            let big = hash((x / 3) as i32, (y / 3) as i32, seed + 93);
            if big > 0.9992 {
                col = lerp3(&col, &[1.0, 1.0, 0.95], 0.85);
            }
            (col, 1.0)
        }
        Pattern::Smoke => {
            let (n, vein) = marble(fx, fy, seed);
            let base = [0.06, 0.06, 0.07];
            let mut col = lerp3(&base, &[0.82, 0.82, 0.86], smoothstep((n - 0.5) / 0.42) * 0.9);
            let vv = (vein - 0.5).abs();
            if vv < 0.03 {
                col = lerp3(&col, &[0.95, 0.95, 0.97], (1.0 - vv / 0.03) * 0.35);
            }
            (col, 1.0)
        }
        Pattern::Picture => match art {
            Some(a) => {
                let s = (px as f64 / a.w as f64).max(px as f64 / a.h as f64);
                let sx = ((x as f64 - (px as f64 - a.w as f64 * s) / 2.0) / s).clamp(0.0, a.w as f64 - 1.0) as usize;
                let sy = ((y as f64 - (px as f64 - a.h as f64 * s) / 2.0) / s).clamp(0.0, a.h as f64 - 1.0) as usize;
                let i = (sy * a.w + sx) * 3;
                ([a.rgb[i] as f64 / 255.0, a.rgb[i + 1] as f64 / 255.0, a.rgb[i + 2] as f64 / 255.0], 1.0)
            }
            None => ([0.45, 0.45, 0.5], 1.0),
        },
        Pattern::Rainbow => {
            let h = turn + rad * 0.15 + (fbm(fx, fy, seed + 101) - 0.5) * 0.12;
            (hsv(h, 0.82, 0.88), 1.0)
        }
        Pattern::Gold => {
            let base = [0.80, 0.63, 0.20];
            let brushed = (hash((rad * 600.0) as i32, 0, seed + 111) - 0.5) * 0.10;
            let sheen = 0.5 + 0.5 * (theta * 3.0 + rad * 2.0).sin();
            let mut col = scale_rgb(&base, 0.72 + 0.38 * sheen + brushed);
            col = lerp3(&col, &[1.0, 0.97, 0.85], sheen * sheen * 0.25);
            (col, 1.0)
        }
        Pattern::Clear => {
            let (n, _) = marble(fx, fy, seed);
            (scale_rgb(&[0.86, 0.88, 0.92], 0.9 + (n - 0.5) * 0.15), 0.42)
        }
    }
}

/// Draw the disc once with Cairo and upload it as a texture.
fn render_disc(size: i32, scale: i32, pattern: Pattern, colors: &[Rgb; 3], art: Option<&ArtPixels>, seed: u32) -> gdk::Texture {
    let px = size * scale;

    // Body pixels.
    let mut body = cairo::ImageSurface::create(cairo::Format::ARgb32, px, px).expect("cairo surface");
    {
        let stride = body.stride() as usize;
        let mut data = body.data().expect("surface data");
        let freq = 3.2 / px as f64;
        let pxu = px as usize;
        for y in 0..pxu {
            for x in 0..pxu {
                let (col, a) = body_pixel(pattern, colors, art, seed, pxu, x, y, x as f64 * freq, y as f64 * freq);
                let i = y * stride + x * 4;
                data[i] = (col[2] * a * 255.0) as u8;
                data[i + 1] = (col[1] * a * 255.0) as u8;
                data[i + 2] = (col[0] * a * 255.0) as u8;
                data[i + 3] = (a * 255.0) as u8;
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

        // Pressing shade: darker toward the rim.
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
        let (dark, light) = match pattern {
            Pattern::Solid => (0.14, 0.035),
            Pattern::Picture | Pattern::Rainbow => (0.10, 0.05),
            _ => (0.16, 0.06),
        };
        let start = if pattern == Pattern::Picture { size as f64 * 0.08 } else { label_r + 3.0 * k };
        let mut gr = start;
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
        if pattern != Pattern::Picture {
            let ring = if pattern.ignores_palette() { [0.11, 0.11, 0.12] } else { scale_rgb(&colors[1], 0.7) };
            cr.set_source_rgb(ring[0], ring[1], ring[2]);
            cr.arc(c, c, label_r, 0.0, 2.0 * PI);
            cr.fill().ok();
        }
    }
    surface.flush();
    let stride = surface.stride() as usize;
    let data = surface.data().expect("surface data");
    let bytes = glib::Bytes::from(&data[..]);
    gdk::MemoryTexture::new(px, px, gdk::MemoryFormat::B8g8r8a8Premultiplied, &bytes, stride).upcast()
}
