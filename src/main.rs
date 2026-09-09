mod art;
mod mpris;
mod record;
mod ui;

use clap::{Parser, ValueEnum};
use gtk::glib;
use gtk::prelude::*;
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use mpris::{Mpris, Preferences};
use std::rc::Rc;

#[derive(Clone, Copy, Debug, ValueEnum)]
enum LayerArg {
    /// Above the wallpaper, below windows (a desktop widget).
    Bottom,
    /// Above windows.
    Top,
    /// Above everything, including fullscreen.
    Overlay,
    /// A normal floating window; no layer-shell.
    Window,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum Anchor {
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

/// A spinning-record now-playing widget for Omarchy, fed by MPRIS.
#[derive(Parser, Debug, Clone)]
#[command(version, about)]
struct Args {
    /// Where to put the widget in the layer stack.
    #[arg(long, value_enum, default_value_t = LayerArg::Bottom)]
    layer: LayerArg,
    /// Screen corner or edge to pin to.
    #[arg(long, value_enum, default_value_t = Anchor::BottomRight)]
    anchor: Anchor,
    /// Gap in pixels from the anchored edges.
    #[arg(long, default_value_t = 32)]
    margin: i32,
    /// Players to prefer when several are playing (identity or bus-name substring).
    #[arg(long, default_values_t = vec!["spotify".to_string(), "cider".to_string()])]
    prefer: Vec<String>,
    /// Players to never show, e.g. --ignore brave --ignore firefox.
    #[arg(long)]
    ignore: Vec<String>,
    /// Platter speed.
    #[arg(long, default_value_t = 33.3)]
    rpm: f64,
}

fn main() -> glib::ExitCode {
    let args = Args::parse();
    let app = gtk::Application::new(Some("dev.derek.vinyl"), Default::default());
    app.connect_activate(move |app| activate(app, &args));
    app.run_with_args::<&str>(&[])
}

fn activate(app: &gtk::Application, args: &Args) {
    let ui = ui::Ui::build(app, args.rpm);
    place_window(&ui.window, args);
    ui.window.present();

    let prefs = Preferences {
        prefer: args.prefer.clone(),
        ignore: args.ignore.clone(),
    };
    let ui2 = ui.clone();
    glib::spawn_future_local(async move {
        let mpris = match Mpris::connect(prefs).await {
            Ok(m) => m,
            Err(e) => {
                eprintln!("vinyl: cannot reach the session bus: {e}");
                return;
            }
        };
        let ui3 = ui2.clone();
        mpris.connect_changed(move |state| ui3.set_state(state));
        let m: Rc<Mpris> = mpris.clone();
        ui2.connect_command(move |cmd| m.command(cmd));
        mpris.start();
    });
}

fn place_window(window: &gtk::ApplicationWindow, args: &Args) {
    if matches!(args.layer, LayerArg::Window) {
        return;
    }
    if !gtk4_layer_shell::is_supported() {
        eprintln!("vinyl: compositor lacks wlr-layer-shell; falling back to a normal window");
        return;
    }
    window.init_layer_shell();
    window.set_namespace(Some("vinyl"));
    window.set_layer(match args.layer {
        LayerArg::Bottom => Layer::Bottom,
        LayerArg::Top => Layer::Top,
        LayerArg::Overlay => Layer::Overlay,
        LayerArg::Window => unreachable!(),
    });
    window.set_keyboard_mode(KeyboardMode::None);
    window.set_exclusive_zone(0);

    let (top, bottom, left, right) = match args.anchor {
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
    for (edge, on) in [(Edge::Top, top), (Edge::Bottom, bottom), (Edge::Left, left), (Edge::Right, right)] {
        window.set_anchor(edge, on);
        window.set_margin(edge, if on { args.margin } else { 0 });
    }
}
