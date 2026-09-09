//! Where the widget sits on screen, for both layer-shell and plain-window
//! modes: anchored corners, free positions from dragging (persisted), and the
//! full-screen takeover.

use gtk::gdk;
use gtk::glib;
use gtk::prelude::*;
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use std::cell::{Cell, RefCell};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum Anchor {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
    Top,
    Bottom,
    Left,
    Right,
    Center,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Placement {
    Anchored { anchor: Anchor, margin: i32 },
    /// Top-left corner offset within a monitor, in logical pixels.
    At { x: i32, y: i32, monitor: Option<String> },
}

pub struct Placer {
    window: gtk::ApplicationWindow,
    layer: Option<Layer>,
    current: RefCell<Placement>,
    fullscreen: Cell<bool>,
    dragging: Cell<bool>,
}

fn state_file() -> PathBuf {
    glib::user_config_dir().join("vinyl").join("position")
}

impl Placement {
    pub fn load_saved() -> Option<Self> {
        let text = std::fs::read_to_string(state_file()).ok()?;
        let mut parts = text.split_whitespace();
        let x = parts.next()?.parse().ok()?;
        let y = parts.next()?.parse().ok()?;
        let monitor = parts.next().map(str::to_string);
        Some(Self::At { x, y, monitor })
    }

    fn save(&self) {
        let path = state_file();
        match self {
            Self::At { x, y, monitor } => {
                if let Some(dir) = path.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                let line = format!("{x} {y} {}\n", monitor.as_deref().unwrap_or(""));
                if let Err(e) = std::fs::write(&path, line) {
                    eprintln!("vinyl: cannot save position: {e}");
                }
            }
            Self::Anchored { .. } => {
                let _ = std::fs::remove_file(&path);
            }
        }
    }
}

impl Placer {
    /// `layer` is `None` for a plain window.
    pub fn new(window: &gtk::ApplicationWindow, layer: Option<Layer>, placement: Placement) -> Self {
        let layer = layer.filter(|_| {
            let ok = gtk4_layer_shell::is_supported();
            if !ok {
                eprintln!("vinyl: compositor lacks wlr-layer-shell; falling back to a normal window");
            }
            ok
        });
        if let Some(l) = layer {
            window.init_layer_shell();
            window.set_namespace(Some("vinyl"));
            window.set_layer(l);
            window.set_keyboard_mode(KeyboardMode::None);
            window.set_exclusive_zone(0);
            if let Placement::At { monitor: Some(name), .. } = &placement {
                if let Some(m) = find_monitor(name) {
                    window.set_monitor(Some(&m));
                }
            }
        }
        let p = Self {
            window: window.clone(),
            layer,
            current: RefCell::new(placement),
            fullscreen: Cell::new(false),
            dragging: Cell::new(false),
        };
        p.apply();
        p
    }

    pub fn is_fullscreen(&self) -> bool {
        self.fullscreen.get()
    }

    fn apply(&self) {
        if self.layer.is_none() || self.fullscreen.get() {
            return;
        }
        let w = &self.window;
        let (top, bottom, left, right, mt, mb, ml, mr) = match &*self.current.borrow() {
            Placement::Anchored { anchor, margin } => {
                let m = *margin;
                let (t, b, l, r) = match anchor {
                    Anchor::TopLeft => (true, false, true, false),
                    Anchor::TopRight => (true, false, false, true),
                    Anchor::BottomLeft => (false, true, true, false),
                    Anchor::BottomRight => (false, true, false, true),
                    Anchor::Top => (true, false, false, false),
                    Anchor::Bottom => (false, true, false, false),
                    Anchor::Left => (false, false, true, false),
                    Anchor::Right => (false, false, false, true),
                    Anchor::Center => (false, false, false, false),
                };
                (t, b, l, r, m, m, m, m)
            }
            Placement::At { x, y, .. } => (true, false, true, false, *y, 0, *x, 0),
        };
        for (edge, on, m) in [
            (Edge::Top, top, mt),
            (Edge::Bottom, bottom, mb),
            (Edge::Left, left, ml),
            (Edge::Right, right, mr),
        ] {
            w.set_anchor(edge, on);
            w.set_margin(edge, if on { m } else { 0 });
        }
    }

    /// Monitor geometry (logical) and window size, if mapped.
    fn frame(&self) -> Option<(gdk::Rectangle, i32, i32, Option<String>)> {
        let surface = self.window.surface()?;
        let monitor = surface.display().monitor_at_surface(&surface)?;
        let geo = monitor.geometry();
        let name = monitor.connector().map(|c| c.to_string());
        Some((geo, self.window.width(), self.window.height(), name))
    }

    /// Start a drag: turn an anchored placement into an explicit position so
    /// deltas can be applied to it.
    pub fn begin_drag(&self, device: Option<&gdk::Device>, button: i32, x: f64, y: f64, time: u32) {
        if self.fullscreen.get() {
            return;
        }
        if self.layer.is_none() {
            if let (Some(surface), Some(dev)) = (self.window.surface(), device) {
                if let Ok(top) = surface.downcast::<gdk::Toplevel>() {
                    top.begin_move(dev, button, x, y, time);
                }
            }
            return;
        }
        let Some((geo, ww, wh, name)) = self.frame() else {
            return;
        };
        let current = self.current.borrow().clone();
        let (x, y) = match current {
            Placement::At { x, y, .. } => (x, y),
            Placement::Anchored { anchor, margin } => {
                let (l, r, t, b) = match anchor {
                    Anchor::TopLeft => (true, false, true, false),
                    Anchor::TopRight => (false, true, true, false),
                    Anchor::BottomLeft => (true, false, false, true),
                    Anchor::BottomRight => (false, true, false, true),
                    Anchor::Top => (false, false, true, false),
                    Anchor::Bottom => (false, false, false, true),
                    Anchor::Left => (true, false, false, false),
                    Anchor::Right => (false, true, false, false),
                    Anchor::Center => (false, false, false, false),
                };
                let x = if l { margin } else if r { geo.width() - margin - ww } else { (geo.width() - ww) / 2 };
                let y = if t { margin } else if b { geo.height() - margin - wh } else { (geo.height() - wh) / 2 };
                (x, y)
            }
        };
        *self.current.borrow_mut() = Placement::At { x, y, monitor: name };
        self.dragging.set(true);
        self.apply();
    }

    /// Apply a drag offset. GTK reports the offset in surface coordinates,
    /// which shift as the surface itself moves, so adding the reported offset
    /// to the *current* position converges on the pointer without needing
    /// absolute coordinates (which Wayland doesn't give us).
    pub fn drag_by(&self, dx: f64, dy: f64) {
        if !self.dragging.get() {
            return;
        }
        let Some((geo, ww, wh, _)) = self.frame() else {
            return;
        };
        let mut cur = self.current.borrow_mut();
        if let Placement::At { x, y, .. } = &mut *cur {
            *x = (*x + dx.round() as i32).clamp(0, (geo.width() - ww).max(0));
            *y = (*y + dy.round() as i32).clamp(0, (geo.height() - wh).max(0));
        }
        drop(cur);
        self.apply();
    }

    pub fn end_drag(&self) {
        if self.dragging.replace(false) {
            self.current.borrow().save();
        }
    }

    pub fn set_fullscreen(&self, on: bool) {
        if self.fullscreen.get() == on {
            return;
        }
        self.fullscreen.set(on);
        let w = &self.window;
        match self.layer {
            Some(layer) => {
                if on {
                    for edge in [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right] {
                        w.set_anchor(edge, true);
                        w.set_margin(edge, 0);
                    }
                    w.set_layer(Layer::Overlay);
                    w.set_exclusive_zone(-1);
                    w.set_keyboard_mode(KeyboardMode::Exclusive);
                } else {
                    w.set_keyboard_mode(KeyboardMode::None);
                    w.set_exclusive_zone(0);
                    w.set_layer(layer);
                    self.apply();
                }
            }
            None => {
                if on {
                    w.fullscreen();
                } else {
                    w.unfullscreen();
                }
            }
        }
    }

    /// Logical size of the monitor the window is on.
    pub fn monitor_size(&self) -> Option<(i32, i32)> {
        self.frame().map(|(geo, _, _, _)| (geo.width(), geo.height()))
    }
}

fn find_monitor(connector: &str) -> Option<gdk::Monitor> {
    let display = gdk::Display::default()?;
    let monitors = display.monitors();
    (0..monitors.n_items())
        .filter_map(|i| monitors.item(i).and_downcast::<gdk::Monitor>())
        .find(|m| m.connector().map(|c| c == connector).unwrap_or(false))
}
