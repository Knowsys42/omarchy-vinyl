mod arm;
mod art;
mod backdrop;
mod mpris;
mod placement;
mod record;
mod ui;

use clap::{Parser, ValueEnum};
use gtk::glib;
use gtk::prelude::*;
use gtk4_layer_shell::Layer;
use mpris::{Mpris, Preferences};
use placement::{Anchor, Placement, Placer};
use record::VinylStyle;
use std::rc::Rc;

#[derive(Clone, Copy, Debug, ValueEnum)]
enum LayerArg {
    /// Above the wallpaper, below windows (a desktop widget).
    Bottom,
    /// Above windows.
    Top,
    /// Above everything, including fullscreen apps.
    Overlay,
    /// A normal floating window; no layer-shell.
    Window,
}

/// A spinning-record now-playing widget for Omarchy, fed by MPRIS.
#[derive(Parser, Debug, Clone)]
#[command(version, about)]
struct Args {
    /// Where to put the widget in the layer stack.
    #[arg(long, value_enum, default_value_t = LayerArg::Bottom)]
    layer: LayerArg,
    /// Screen corner or edge to pin to. Overrides (and forgets) a dragged position.
    #[arg(long, value_enum)]
    anchor: Option<Anchor>,
    /// Gap in pixels from the anchored edges.
    #[arg(long, default_value_t = 32)]
    margin: i32,
    /// Disc colour: black, marble (swirled from the album art), art (the art's
    /// dominant colour), or any CSS colour like crimson or #1e90ff.
    #[arg(long, default_value = "black", value_parser = VinylStyle::parse)]
    vinyl: VinylStyle,
    /// Hide the tone arm.
    #[arg(long)]
    no_arm: bool,
    /// Start in full-screen mode.
    #[arg(long)]
    fullscreen: bool,
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
    let layer = match args.layer {
        LayerArg::Bottom => Some(Layer::Bottom),
        LayerArg::Top => Some(Layer::Top),
        LayerArg::Overlay => Some(Layer::Overlay),
        LayerArg::Window => None,
    };
    let placement = match args.anchor {
        Some(anchor) => Placement::Anchored { anchor, margin: args.margin },
        None => Placement::load_saved().unwrap_or(Placement::Anchored {
            anchor: Anchor::BottomRight,
            margin: args.margin,
        }),
    };
    if args.anchor.is_some() {
        // An explicit anchor replaces any remembered drag position.
        let _ = std::fs::remove_file(glib::user_config_dir().join("vinyl").join("position"));
    }

    let cfg = ui::UiConfig {
        rpm: args.rpm,
        style: args.vinyl.clone(),
        show_arm: !args.no_arm,
        start_fullscreen: args.fullscreen,
    };
    let ui = ui::Ui::build(app, cfg, move |window| Placer::new(window, layer, placement));
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
