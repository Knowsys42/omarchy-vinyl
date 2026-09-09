//! A `gdk::Paintable` that draws the vinyl: pre-rendered grooves, the album art
//! as the centre label, and a spindle hole, all rotated by `angle`.
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

pub const LABEL: f64 = 56.0;
const HOLE: f64 = 7.0;

mod imp {
    use super::*;
    use std::cell::{Cell, RefCell};

    #[derive(Default)]
    pub struct RecordPaintable {
        pub disc: RefCell<Option<gdk::Texture>>,
        pub label: RefCell<Option<gdk::Texture>>,
        pub angle: Cell<f64>,
        pub size: Cell<i32>,
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

            let r = (LABEL / 2.0) as f32;
            let label_rect = graphene::Rect::new(cx - r, cy - r, LABEL as f32, LABEL as f32);
            snapshot.push_rounded_clip(&gsk::RoundedRect::from_rect(label_rect, r));
            match self.label.borrow().as_ref() {
                Some(tex) => snapshot.append_texture(tex, &cover_rect(tex, &label_rect)),
                None => snapshot.append_color(&gdk::RGBA::new(0.42, 0.16, 0.16, 1.0), &label_rect),
            }
            snapshot.pop();

            let hr = (HOLE / 2.0) as f32;
            let hole = graphene::Rect::new(cx - hr, cy - hr, HOLE as f32, HOLE as f32);
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
    pub fn new(size: i32, scale: i32) -> Self {
        let obj: Self = glib::Object::new();
        obj.imp().size.set(size);
        obj.imp().disc.replace(Some(render_disc(size, scale.max(1))));
        obj
    }

    pub fn set_angle(&self, angle: f64) {
        if (self.imp().angle.get() - angle).abs() > 1e-3 {
            self.imp().angle.set(angle);
            self.invalidate_contents();
        }
    }

    pub fn set_label(&self, tex: Option<gdk::Texture>) {
        self.imp().label.replace(tex);
        self.invalidate_contents();
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

/// Draw the grooved disc once with Cairo and upload it as a texture.
fn render_disc(size: i32, scale: i32) -> gdk::Texture {
    let px = size * scale;
    let mut surface = cairo::ImageSurface::create(cairo::Format::ARgb32, px, px).expect("cairo surface");
    {
        let cr = cairo::Context::new(&surface).expect("cairo context");
        cr.scale(scale as f64, scale as f64);
        draw_vinyl(&cr, size as f64);
    }
    surface.flush();
    let stride = surface.stride() as usize;
    let data = surface.data().expect("surface data");
    let bytes = glib::Bytes::from(&data[..]);
    gdk::MemoryTexture::new(px, px, gdk::MemoryFormat::B8g8r8a8Premultiplied, &bytes, stride).upcast()
}

fn draw_vinyl(cr: &cairo::Context, size: f64) {
    let c = size / 2.0;
    let r = c;

    let body = cairo::RadialGradient::new(c, c, r * 0.2, c, c, r);
    body.add_color_stop_rgb(0.0, 0.16, 0.16, 0.18);
    body.add_color_stop_rgb(0.6, 0.09, 0.09, 0.10);
    body.add_color_stop_rgb(1.0, 0.05, 0.05, 0.06);
    cr.arc(c, c, r, 0.0, 2.0 * PI);
    let _ = cr.set_source(&body);
    let _ = cr.fill();

    let label_r = LABEL / 2.0 + 3.0;
    let mut gr = label_r + 3.0;
    let mut i = 0;
    while gr < r - 3.0 {
        let a = if i % 7 == 0 { 0.09 } else { 0.035 };
        cr.set_source_rgba(1.0, 1.0, 1.0, a);
        cr.set_line_width(0.6);
        cr.arc(c, c, gr, 0.0, 2.0 * PI);
        let _ = cr.stroke();
        gr += 1.55;
        i += 1;
    }

    cr.set_source_rgba(1.0, 1.0, 1.0, 0.10);
    cr.set_line_width(1.0);
    cr.arc(c, c, r - 0.5, 0.0, 2.0 * PI);
    let _ = cr.stroke();
    cr.set_source_rgb(0.11, 0.11, 0.12);
    cr.arc(c, c, label_r, 0.0, 2.0 * PI);
    let _ = cr.fill();
}
