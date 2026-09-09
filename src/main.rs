mod arm;
mod art;
mod backdrop;
mod hypr;
mod mpris;
mod placement;
mod record;
mod theme;
mod ui;

use clap::{Parser, ValueEnum};
use gtk::gio;
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
    /// Monitor to live on (connector name like DP-1, or `current`). Sends a
    /// running widget there.
    #[arg(long)]
    monitor: Option<String>,
    /// Workspaces to appear on: `all`, `current`, or a comma list of ids or
    /// names like `1,3,music`. Sends a running widget there.
    #[arg(long)]
    workspace: Option<String>,
    /// Disc style: a preset (black, marble, splatter, split, tri, starburst,
    /// galaxy, smoke, picture, rainbow, gold, clear, glow, omarchy), a CSS
    /// colour, or pattern:palette such as splatter:theme or split:#1e90ff,white.
    /// Defaults to the last style picked with the button.
    #[arg(long, value_parser = VinylStyle::parse)]
    vinyl: Option<VinylStyle>,
    /// Hide the tone arm.
    #[arg(long)]
    no_arm: bool,
    /// Start in full-screen mode; if the widget is already running, toggle it.
    #[arg(long)]
    fullscreen: bool,
    /// Solid card background instead of translucent.
    #[arg(long)]
    opaque: bool,
    /// Start the widget, or quit it if it is already running.
    #[arg(long)]
    toggle: bool,
    /// Quit a running widget.
    #[arg(long)]
    quit: bool,
    /// Step a running widget to the next pressing.
    #[arg(long)]
    next_style: bool,
    /// Players to prefer when several are playing (identity or bus-name substring).
    #[arg(long)]
    prefer: Vec<String>,
    /// Players to never show, e.g. --ignore brave --ignore firefox.
    #[arg(long)]
    ignore: Vec<String>,
    /// Platter speed.
    #[arg(long, default_value_t = 33.3)]
    rpm: f64,
}

const APP_ID: &str = "io.github.knowsys42.vinyl";

fn main() -> glib::ExitCode {
    let args = Args::parse();
    let app = gtk::Application::new(Some(APP_ID), Default::default());

    // A second launch reaches the running instance over D-Bus. Remote flags
    // become action activations; otherwise the running instance is raised.
    if let Err(e) = app.register(gio::Cancellable::NONE) {
        eprintln!("vinyl: cannot register application: {e}");
    }
    if app.is_remote() {
        let mut acted = false;
        for (flag, action) in [
            (args.quit || args.toggle, "quit"),
            (args.fullscreen, "fullscreen"),
            (args.next_style, "next-style"),
        ] {
            if flag {
                app.activate_action(action, None);
                acted = true;
            }
        }
        if let Some(m) = &args.monitor {
            app.activate_action("monitor", Some(&m.to_variant()));
            acted = true;
        }
        if let Some(w) = &args.workspace {
            app.activate_action("workspace", Some(&w.to_variant()));
            acted = true;
        }
        if acted {
            // Exit without tearing the GApplication down: it was never run,
            // and its destructor would only complain about that.
            std::process::exit(0);
        }
    } else if args.quit {
        return glib::ExitCode::SUCCESS;
    }

    app.connect_activate(move |app| activate(app, &args));
    app.run_with_args::<&str>(&[])
}

fn activate(app: &gtk::Application, args: &Args) {
    // A plain second launch lands here in the running instance; one window is enough.
    if let Some(w) = app.active_window() {
        w.present();
        return;
    }
    let layer = match args.layer {
        LayerArg::Bottom => Some(Layer::Bottom),
        LayerArg::Top => Some(Layer::Top),
        LayerArg::Overlay => Some(Layer::Overlay),
        LayerArg::Window => None,
    };
    let saved = Placement::load_saved();
    let mut placement = match args.anchor {
        Some(anchor) => Placement::Anchored {
            anchor,
            margin: args.margin,
            monitor: saved.as_ref().and_then(|p| p.monitor().map(str::to_string)),
        },
        None => saved.unwrap_or(Placement::Anchored {
            anchor: Anchor::BottomRight,
            margin: args.margin,
            monitor: None,
        }),
    };
    if let Some(m) = args.monitor.as_deref().filter(|m| *m != "current") {
        placement = placement.with_monitor(Some(m.to_string()));
    }

    let style = args
        .vinyl
        .clone()
        .or_else(|| placement::load_config("style").and_then(|s| VinylStyle::parse(&s).ok()))
        .unwrap_or_else(|| VinylStyle::parse("black").unwrap());
    let cfg = ui::UiConfig {
        rpm: args.rpm,
        style,
        show_arm: !args.no_arm,
        start_fullscreen: args.fullscreen,
        opaque: args.opaque,
        workspaces: args.workspace.clone(),
    };
    let ui = ui::Ui::build(app, cfg, move |window| Placer::new(window, layer, placement));
    ui.window.present();

    let quit = gio::SimpleAction::new("quit", None);
    let a = app.clone();
    quit.connect_activate(move |_, _| a.quit());
    app.add_action(&quit);
    let fullscreen = gio::SimpleAction::new("fullscreen", None);
    let u = ui.clone();
    fullscreen.connect_activate(move |_, _| u.toggle_fullscreen());
    app.add_action(&fullscreen);
    let next_style = gio::SimpleAction::new("next-style", None);
    let u = ui.clone();
    next_style.connect_activate(move |_, _| u.cycle_style(1));
    app.add_action(&next_style);
    let monitor = gio::SimpleAction::new("monitor", Some(glib::VariantTy::STRING));
    let u = ui.clone();
    monitor.connect_activate(move |_, v| {
        if let Some(name) = v.and_then(|v| v.str()) {
            u.send_to_monitor(name.to_string());
        }
    });
    app.add_action(&monitor);
    let workspace = gio::SimpleAction::new("workspace", Some(glib::VariantTy::STRING));
    let u = ui.clone();
    workspace.connect_activate(move |_, v| {
        if let Some(spec) = v.and_then(|v| v.str()) {
            u.set_workspaces(spec.to_string());
        }
    });
    app.add_action(&workspace);
    if args.monitor.as_deref() == Some("current") {
        ui.send_to_monitor("current".to_string());
    }

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
