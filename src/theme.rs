//! The current Omarchy theme's palette, read from the colors.toml that
//! `omarchy-theme-set` links into place, and a watcher that fires when the
//! theme changes.

use crate::record::Rgb;
use gtk::gdk;
use gtk::gio;
use gtk::glib;
use gtk::prelude::*;
use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

fn current_dir() -> PathBuf {
    glib::home_dir().join(".local/state/omarchy/current")
}

fn colors_file() -> PathBuf {
    current_dir().join("theme/colors.toml")
}

fn parse_hex(s: &str) -> Option<Rgb> {
    let c = gdk::RGBA::parse(s).ok()?;
    Some([c.red() as f64, c.green() as f64, c.blue() as f64])
}

fn saturation(c: &Rgb) -> f64 {
    let max = c[0].max(c[1]).max(c[2]);
    let min = c[0].min(c[1]).min(c[2]);
    if max <= 1e-6 {
        0.0
    } else {
        (max - min) / max
    }
}

fn dist(a: &Rgb, b: &Rgb) -> f64 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

/// Theme colours worth pressing a record from, most important first: the
/// accent, then the most saturated named colours that differ from what's
/// already chosen, then the foreground. Empty if there is no Omarchy theme.
pub fn load() -> Vec<Rgb> {
    let Ok(text) = std::fs::read_to_string(colors_file()) else {
        return Vec::new();
    };
    let mut map = std::collections::HashMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('#') || !line.contains('=') {
            continue;
        }
        let (k, v) = line.split_once('=').unwrap();
        let v = v.trim().trim_matches('"').trim_matches('\'');
        if let Some(c) = parse_hex(v) {
            map.insert(k.trim().to_string(), c);
        }
    }

    let mut out: Vec<Rgb> = Vec::new();
    let push = |c: Rgb, out: &mut Vec<Rgb>| {
        if out.iter().all(|o| dist(o, &c) > 0.15) {
            out.push(c);
        }
    };
    if let Some(a) = map.get("accent") {
        push(*a, &mut out);
    }
    let mut named: Vec<Rgb> = ["red", "orange", "yellow", "green", "cyan", "blue", "magenta", "purple", "brown"]
        .iter()
        .filter_map(|k| map.get(*k).copied())
        .collect();
    named.sort_by(|a, b| saturation(b).partial_cmp(&saturation(a)).unwrap());
    for c in named {
        push(c, &mut out);
    }
    if let Some(f) = map.get("foreground") {
        push(*f, &mut out);
    }
    if let Some(b) = map.get("background") {
        push(*b, &mut out);
    }
    out
}

/// Watch the Omarchy `current` directory; call `on_change` (debounced)
/// whenever the theme link changes. Keep the returned monitor alive.
pub fn watch(on_change: impl Fn() + 'static) -> Option<gio::FileMonitor> {
    let dir = gio::File::for_path(current_dir());
    let monitor = dir
        .monitor_directory(gio::FileMonitorFlags::WATCH_MOVES, gio::Cancellable::NONE)
        .ok()?;
    let on_change = Rc::new(on_change);
    let generation = Rc::new(Cell::new(0u64));
    monitor.connect_changed(move |_, _, _, _| {
        let gen = generation.get() + 1;
        generation.set(gen);
        let generation = generation.clone();
        let on_change = on_change.clone();
        glib::timeout_add_local_once(Duration::from_millis(400), move || {
            if generation.get() == gen {
                on_change();
            }
        });
    });
    Some(monitor)
}
