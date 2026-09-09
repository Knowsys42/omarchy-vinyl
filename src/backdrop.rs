//! Full-screen backdrop: the album art scaled to cover, heavily blurred and
//! darkened, drawn by the GPU through a GSK blur node.

use gtk::gdk;
use gtk::glib;
use gtk::graphene;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

mod imp {
    use super::*;
    use std::cell::RefCell;

    #[derive(Default)]
    pub struct Backdrop {
        pub art: RefCell<Option<gdk::Texture>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Backdrop {
        const NAME: &'static str = "VinylBackdrop";
        type Type = super::Backdrop;
        type Interfaces = (gdk::Paintable,);
    }

    impl ObjectImpl for Backdrop {}

    impl PaintableImpl for Backdrop {
        fn snapshot(&self, snapshot: &gdk::Snapshot, width: f64, height: f64) {
            let snapshot = snapshot.downcast_ref::<gtk::Snapshot>().expect("gtk snapshot");
            let (w, h) = (width as f32, height as f32);
            let bounds = graphene::Rect::new(0.0, 0.0, w, h);
            snapshot.append_color(&gdk::RGBA::new(0.05, 0.05, 0.06, 1.0), &bounds);
            if let Some(tex) = self.art.borrow().as_ref() {
                let radius = (w.max(h) * 0.04) as f64;
                // Overscan so the blur doesn't fade at the edges.
                let pad = radius as f32 * 2.0;
                let (tw, th) = (tex.width() as f32, tex.height() as f32);
                let scale = ((w + 2.0 * pad) / tw).max((h + 2.0 * pad) / th);
                let (sw, sh) = (tw * scale, th * scale);
                let rect = graphene::Rect::new((w - sw) / 2.0, (h - sh) / 2.0, sw, sh);
                snapshot.push_clip(&bounds);
                snapshot.push_blur(radius);
                snapshot.append_texture(tex, &rect);
                snapshot.pop();
                snapshot.pop();
            }
            snapshot.append_color(&gdk::RGBA::new(0.0, 0.0, 0.0, 0.55), &bounds);
        }
    }
}

glib::wrapper! {
    pub struct Backdrop(ObjectSubclass<imp::Backdrop>) @implements gdk::Paintable;
}

impl Backdrop {
    pub fn new() -> Self {
        glib::Object::new()
    }

    pub fn set_art(&self, tex: Option<gdk::Texture>) {
        self.imp().art.replace(tex);
        self.invalidate_contents();
    }
}
