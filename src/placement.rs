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

impl Anchor {
    const KEYS: [(Anchor, &'static str); 9] = [
        (Anchor::TopLeft, "top-left"),
        (Anchor::TopRight, "top-right"),
        (Anchor::BottomLeft, "bottom-left"),
        (Anchor::BottomRight, "bottom-right"),
        (Anchor::Top, "top"),
        (Anchor::Bottom, "bottom"),
        (Anchor::Left, "left"),
        (Anchor::Right, "right"),
        (Anchor::Center, "center"),
    ];
    fn key(self) -> &'static str {
        Self::KEYS.iter().find(|(a, _)| *a == self).map(|(_, k)| *k).unwrap()
    }
    fn from_key(s: &str) -> Option<Self> {
        Self::KEYS.iter().find(|(_, k)| *k == s).map(|(a, _)| *a)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Placement {
    Anchored { anchor: Anchor, margin: i32, monitor: Option<String> },
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

fn config_path(name: &str) -> PathBuf {
    glib::user_config_dir().join("vinyl").join(name)
}

fn state_file() -> PathBuf {
    config_path("position")
}

pub fn save_config(name: &str, value: &str) {
    let path = config_path(name);
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Err(e) = std::fs::write(&path, format!("{value}\n")) {
        eprintln!("vinyl: cannot save {name}: {e}");
    }
}

pub fn load_config(name: &str) -> Option<String> {
    std::fs::read_to_string(config_path(name)).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

impl Placement {
    /// `at X Y [MONITOR]`, `anchored ANCHOR MARGIN [MONITOR]`, or the older
    /// bare `X Y [MONITOR]`.
    pub fn load_saved() -> Option<Self> {
        let text = std::fs::read_to_string(state_file()).ok()?;
        let parts: Vec<&str> = text.split_whitespace().collect();
        let monitor = |i: usize| parts.get(i).map(|s| s.to_string());
        match parts.first().copied() {
            Some("at") => Some(Self::At { x: parts.get(1)?.parse().ok()?, y: parts.get(2)?.parse().ok()?, monitor: monitor(3) }),
            Some("anchored") => Some(Self::Anchored {
                anchor: Anchor::from_key(parts.get(1)?)?,
                margin: parts.get(2)?.parse().ok()?,
                monitor: monitor(3),
            }),
            Some(_) => Some(Self::At { x: parts.first()?.parse().ok()?, y: parts.get(1)?.parse().ok()?, monitor: monitor(2) }),
            None => None,
        }
    }

    pub fn monitor(&self) -> Option<&str> {
        match self {
            Self::At { monitor, .. } | Self::Anchored { monitor, .. } => monitor.as_deref(),
        }
    }

    pub fn with_monitor(mut self, name: Option<String>) -> Self {
        match &mut self {
            Self::At { monitor, .. } | Self::Anchored { monitor, .. } => *monitor = name,
        }
        self
    }

    fn save(&self) {
        let line = match self {
            Self::At { x, y, monitor } => format!("at {x} {y} {}", monitor.as_deref().unwrap_or("")),
            Self::Anchored { anchor, margin, monitor } => {
                format!("anchored {} {margin} {}", anchor.key(), monitor.as_deref().unwrap_or(""))
            }
        };
        save_config("position", line.trim_end());
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
            if let Some(m) = placement.monitor().and_then(find_monitor) {
                window.set_monitor(Some(&m));
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
            Placement::Anchored { anchor, margin, .. } => {
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
            Placement::Anchored { anchor, margin, .. } => {
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

    /// The monitor this widget lives on: what the placement says, or where
    /// the surface currently is.
    pub fn monitor_name(&self) -> Option<String> {
        self.current
            .borrow()
            .monitor()
            .map(str::to_string)
            .or_else(|| self.frame().and_then(|(_, _, _, name)| name))
    }

    /// Record the monitor the surface is on (call once mapped), so the widget
    /// can find its way back after being hidden.
    pub fn remember_monitor(&self) {
        if self.current.borrow().monitor().is_some() {
            return;
        }
        if let Some((_, _, _, Some(name))) = self.frame() {
            let cur = self.current.borrow().clone();
            *self.current.borrow_mut() = cur.with_monitor(Some(name));
        }
    }

    /// Move to another monitor, keeping the anchor or position. Layer-shell only.
    pub fn set_monitor(&self, name: &str) -> bool {
        let Some(monitor) = find_monitor(name) else {
            eprintln!("vinyl: no monitor named {name}");
            return false;
        };
        if self.layer.is_none() {
            return false;
        }
        let cur = self.current.borrow().clone();
        let cur = match cur {
            // A dragged position is relative to the old monitor; re-anchor.
            Placement::At { .. } => Placement::Anchored { anchor: Anchor::BottomRight, margin: 32, monitor: Some(name.to_string()) },
            other => other.with_monitor(Some(name.to_string())),
        };
        *self.current.borrow_mut() = cur;
        // gtk4-layer-shell applies the monitor on map, so remap.
        let was_visible = self.window.is_visible();
        if std::env::var_os("VINYL_DEBUG").is_some() {
            eprintln!("vinyl: set_monitor {name}: connector {:?}, visible {was_visible}", monitor.connector());
        }
        self.window.set_visible(false);
        self.window.set_monitor(Some(&monitor));
        self.apply();
        if was_visible {
            self.window.set_visible(true);
        }
        self.current.borrow().save();
        true
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
