//! The widget: a record sleeve, a vinyl that slides out and spins while
//! playing, and a tone arm that drops onto the groove. Also the full-screen
//! takeover and the text/controls column.

use crate::arm::{self, ArmGeometry};
use crate::art;
use crate::backdrop::Backdrop;
use crate::mpris::{Command, PlayerState, Status};
use crate::placement::Placer;
use crate::record::{presets, RecordPaintable, Rgb, VinylStyle};
use crate::theme;
use gtk::cairo;
use gtk::gdk;
use gtk::gio;
use gtk::glib;
use gtk::pango;
use gtk::prelude::*;
use std::cell::{Cell, RefCell};
use std::f64::consts::PI;
use std::rc::Rc;
use std::time::Duration;

// Stage geometry at scale 1. Everything scales uniformly, so angles don't change.
const B_SLEEVE: f64 = 150.0;
const B_RECORD: f64 = 150.0;
const B_PAD: f64 = 6.0;
const B_OUT: f64 = 96.0;
const B_IN: f64 = 26.0;
const B_ARM_ROOM: f64 = 30.0;
const B_STAGE_W: f64 = B_SLEEVE + B_OUT + B_ARM_ROOM;
const B_STAGE_H: f64 = B_SLEEVE + 2.0 * B_PAD;
const B_ARM_REST: f64 = 82.0;
const MIN_FRAME_US: i64 = 1_000_000 / 60;

fn base_geometry() -> ArmGeometry {
    ArmGeometry {
        pivot: (B_STAGE_W - 24.0, 22.0),
        length: 96.0,
        rest_angle: B_ARM_REST,
        record_center: (B_SLEEVE - B_RECORD + B_OUT + B_RECORD / 2.0, B_PAD + B_RECORD / 2.0),
        outer_groove: 72.0,
        inner_groove: 34.0,
    }
}

const CSS: &str = r#"
window { background: transparent; }
.card {
  background: rgba(18, 18, 22, 0.86);
  border: 1px solid rgba(255, 255, 255, 0.08);
  border-radius: 24px;
  padding: 18px 22px 18px 18px;
  box-shadow: 0 12px 40px rgba(0, 0, 0, 0.45);
}
.sleeve {
  border-radius: 10px;
  background: #24242a;
  box-shadow: 0 6px 18px rgba(0, 0, 0, 0.55);
}
.player { font-size: 10px; font-weight: 700; letter-spacing: 2px; color: rgba(255,255,255,0.45); }
.title { font-size: 16px; font-weight: 700; color: rgba(255,255,255,0.95); }
.artist { font-size: 13px; color: rgba(255,255,255,0.72); }
.album { font-size: 12px; color: rgba(255,255,255,0.45); }
.time { font-size: 10px; color: rgba(255,255,255,0.45); font-variant-numeric: tabular-nums; }
progressbar.thin { min-height: 4px; }
progressbar.thin trough { min-height: 4px; border-radius: 2px; background: rgba(255,255,255,0.12); }
progressbar.thin progress { min-height: 4px; border-radius: 2px; background: rgba(255,255,255,0.85); }
button.ctl {
  background: transparent; border: none; box-shadow: none; padding: 6px;
  color: rgba(255,255,255,0.85); min-width: 0; min-height: 0;
}
button.ctl:hover { background: rgba(255,255,255,0.10); }
button.ctl:active { background: rgba(255,255,255,0.18); }
button.ctl.play { background: rgba(255,255,255,0.92); color: #111; padding: 8px; }
button.ctl.play:hover { background: #fff; }
button.ctl:disabled { color: rgba(255,255,255,0.25); }
button.fs { padding: 2px; color: rgba(255,255,255,0.35); opacity: 0; }
.card:hover button.fs { opacity: 1; }
button.fs:hover { color: rgba(255,255,255,0.9); }
.flash { color: rgba(255,255,255,0.85); }

window.opaque .card { background: #121216; }
window.takeover .card { background: transparent; border: none; box-shadow: none; padding: 0; }
window.takeover .column { margin-left: 48px; }
window.takeover .player { font-size: 15px; letter-spacing: 4px; }
window.takeover .title { font-size: 44px; }
window.takeover .artist { font-size: 26px; }
window.takeover .album { font-size: 20px; }
window.takeover .time { font-size: 14px; }
window.takeover progressbar.thin trough,
window.takeover progressbar.thin progress { min-height: 6px; border-radius: 3px; }
window.takeover button.ctl { padding: 14px; }
window.takeover button.ctl.play { padding: 18px; }
window.takeover button.ctl image { -gtk-icon-size: 30px; }
window.takeover button.fs { opacity: 1; }
window.takeover .sleeve { border-radius: 22px; }
"#;

pub struct UiConfig {
    pub rpm: f64,
    pub style: VinylStyle,
    pub show_arm: bool,
    pub start_fullscreen: bool,
    pub opaque: bool,
}

/// Animation state, scale-independent.
struct Anim {
    playing: bool,
    progress: f64,
    angle: f64,
    ang_vel: f64,
    /// 0 = record in the sleeve, 1 = fully out.
    slide: f64,
    arm_angle: f64,
    last_us: i64,
    tick: Option<gtk::TickCallbackId>,
    geom: ArmGeometry,
}

impl Anim {
    fn needle_angle(&self) -> f64 {
        self.geom.angle_for_progress(self.progress)
    }
    fn lifted(&self) -> f64 {
        ((self.arm_angle - self.needle_angle()).abs() / 8.0).clamp(0.0, 1.0)
    }
}

/// Sleeve, record, arm and sheen at one scale factor.
struct Stage {
    k: f64,
    root: gtk::Overlay,
    fixed: gtk::Fixed,
    record: gtk::Picture,
    paintable: RecordPaintable,
    sleeve_pic: gtk::Image,
    sheen: gtk::DrawingArea,
    arm: gtk::DrawingArea,
    show_arm: bool,
    last_slide: Cell<f64>,
    last_arm: Cell<f64>,
}

enum Hit {
    Arm,
    Record,
    Sleeve,
    Nothing,
}

impl Stage {
    fn new(k: f64, style: VinylStyle, theme: Vec<Rgb>, show_arm: bool, anim: Rc<RefCell<Anim>>) -> Self {
        let sleeve_px = (B_SLEEVE * k).round() as i32;
        let record_px = (B_RECORD * k).round() as i32;
        let fixed = gtk::Fixed::new();
        fixed.set_size_request((B_STAGE_W * k).round() as i32, (B_STAGE_H * k).round() as i32);

        // Big records don't need a 2x texture.
        let tex_scale = if record_px > 400 { 1 } else { 2 };
        let paintable = RecordPaintable::new(record_px, tex_scale, style, theme);
        let record = gtk::Picture::for_paintable(&paintable);
        record.set_size_request(record_px, record_px);
        record.set_can_shrink(false);

        let sleeve_pic = gtk::Image::new();
        sleeve_pic.set_pixel_size(sleeve_px);
        let sleeve = gtk::Box::new(gtk::Orientation::Vertical, 0);
        sleeve.add_css_class("sleeve");
        sleeve.set_overflow(gtk::Overflow::Hidden);
        sleeve.set_size_request(sleeve_px, sleeve_px);
        sleeve.append(&sleeve_pic);

        fixed.put(&record, Self::record_x_for(k, 0.0), B_PAD * k);
        fixed.put(&sleeve, 0.0, B_PAD * k);

        let sheen = gtk::DrawingArea::new();
        sheen.set_can_target(false);
        let a = anim.clone();
        sheen.set_draw_func(move |_, cr, _, h| {
            let slide = a.borrow().slide;
            draw_sheen(cr, k, h as f64, Self::record_x_for(k, slide));
        });

        let arm = gtk::DrawingArea::new();
        arm.set_can_target(false);
        arm.set_visible(show_arm);
        let a = anim.clone();
        arm.set_draw_func(move |_, cr, _, _| {
            let a = a.borrow();
            cr.scale(k, k);
            arm::draw_arm(cr, &a.geom, a.arm_angle, a.lifted());
        });

        let root = gtk::Overlay::new();
        root.set_child(Some(&fixed));
        root.add_overlay(&sheen);
        root.add_overlay(&arm);

        Self {
            k,
            root,
            fixed,
            record,
            paintable,
            sleeve_pic,
            sheen,
            arm,
            show_arm,
            last_slide: Cell::new(-1.0),
            last_arm: Cell::new(f64::NAN),
        }
    }

    fn record_x_for(k: f64, slide: f64) -> f64 {
        (B_SLEEVE - B_RECORD + B_IN + (B_OUT - B_IN) * slide) * k
    }

    /// Push animation state into the widgets, redrawing only what changed.
    fn apply(&self, a: &Anim) {
        self.paintable.set_angle(a.angle);
        if (a.slide - self.last_slide.get()).abs() > 1e-4 {
            self.last_slide.set(a.slide);
            self.fixed.move_(&self.record, Self::record_x_for(self.k, a.slide), B_PAD * self.k);
            self.sheen.queue_draw();
        }
        if self.show_arm && (a.arm_angle - self.last_arm.get()).abs() > 0.02 {
            self.last_arm.set(a.arm_angle);
            self.arm.queue_draw();
        }
    }

    fn set_art(&self, tex: Option<&gdk::Texture>, seed: u32) {
        self.sleeve_pic.set_paintable(tex);
        self.paintable.set_label(tex.cloned(), seed);
    }

    fn hit(&self, a: &Anim, x: f64, y: f64) -> Hit {
        let (x, y) = (x / self.k, y / self.k);
        if self.show_arm && a.geom.distance_to_arm(a.arm_angle, x, y) < 9.0 {
            return Hit::Arm;
        }
        let rx = Self::record_x_for(1.0, a.slide);
        let (cx, cy) = (rx + B_RECORD / 2.0, B_PAD + B_RECORD / 2.0);
        let in_record = ((x - cx).powi(2) + (y - cy).powi(2)).sqrt() < B_RECORD / 2.0;
        let in_sleeve = x >= 0.0 && x < B_SLEEVE && y >= B_PAD && y < B_PAD + B_SLEEVE;
        if in_record && !in_sleeve {
            Hit::Record
        } else if in_sleeve {
            Hit::Sleeve
        } else {
            Hit::Nothing
        }
    }
}

pub struct Ui {
    pub window: gtk::ApplicationWindow,
    placer: Rc<Placer>,
    backdrop_pic: gtk::Picture,
    backdrop: Backdrop,
    card: gtk::Box,
    stage_slot: gtk::Box,
    stage: RefCell<Stage>,
    player_label: gtk::Label,
    title: gtk::Label,
    artist: gtk::Label,
    album: gtk::Label,
    progress: gtk::ProgressBar,
    elapsed: gtk::Label,
    total: gtk::Label,
    prev_btn: gtk::Button,
    play_btn: gtk::Button,
    next_btn: gtk::Button,
    fs_btn: gtk::Button,
    style_btn: gtk::Button,
    cfg: UiConfig,
    style: RefCell<VinylStyle>,
    theme_colors: RefCell<Vec<Rgb>>,
    theme_monitor: RefCell<Option<gio::FileMonitor>>,
    flash_gen: Cell<u64>,
    state: RefCell<Option<PlayerState>>,
    art_url: RefCell<Option<String>>,
    art_tex: RefCell<Option<gdk::Texture>>,
    art_generation: Cell<u64>,
    anim: Rc<RefCell<Anim>>,
    on_command: RefCell<Option<Box<dyn Fn(Command)>>>,
}

impl Ui {
    pub fn build(app: &gtk::Application, cfg: UiConfig, make_placer: impl FnOnce(&gtk::ApplicationWindow) -> Placer) -> Rc<Self> {
        let provider = gtk::CssProvider::new();
        provider.load_from_string(CSS);
        gtk::style_context_add_provider_for_display(
            &gdk::Display::default().expect("no display"),
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );

        let anim = Rc::new(RefCell::new(Anim {
            playing: false,
            progress: 0.0,
            angle: 0.0,
            ang_vel: 0.0,
            slide: 0.0,
            arm_angle: B_ARM_REST,
            last_us: 0,
            tick: None,
            geom: base_geometry(),
        }));

        let theme_colors = theme::load();
        let stage = Stage::new(1.0, cfg.style.clone(), theme_colors.clone(), cfg.show_arm, anim.clone());
        let stage_slot = gtk::Box::new(gtk::Orientation::Vertical, 0);
        stage_slot.set_valign(gtk::Align::Center);
        stage_slot.append(&stage.root);

        // --- text + controls ------------------------------------------------
        let player_label = gtk::Label::new(None);
        player_label.add_css_class("player");
        player_label.set_xalign(0.0);
        player_label.set_hexpand(true);
        let fs_btn = gtk::Button::from_icon_name("view-fullscreen-symbolic");
        fs_btn.add_css_class("ctl");
        fs_btn.add_css_class("fs");
        fs_btn.add_css_class("circular");
        fs_btn.set_valign(gtk::Align::Center);
        let style_btn = gtk::Button::from_icon_name("color-select-symbolic");
        style_btn.add_css_class("ctl");
        style_btn.add_css_class("fs");
        style_btn.add_css_class("circular");
        style_btn.set_valign(gtk::Align::Center);
        style_btn.set_tooltip_text(Some(&format!("{} (click: next, right-click: previous)", cfg.style.label())));
        let header = gtk::Box::new(gtk::Orientation::Horizontal, 2);
        header.append(&player_label);
        header.append(&style_btn);
        header.append(&fs_btn);

        let title = gtk::Label::new(None);
        title.add_css_class("title");
        let artist = gtk::Label::new(None);
        artist.add_css_class("artist");
        let album = gtk::Label::new(None);
        album.add_css_class("album");
        for l in [&title, &artist, &album] {
            l.set_xalign(0.0);
            l.set_ellipsize(pango::EllipsizeMode::End);
            l.set_max_width_chars(24);
            l.set_width_chars(24);
        }

        let progress = gtk::ProgressBar::new();
        progress.add_css_class("thin");
        progress.set_margin_top(6);

        let elapsed = gtk::Label::new(Some("0:00"));
        let total = gtk::Label::new(Some("0:00"));
        elapsed.add_css_class("time");
        total.add_css_class("time");
        let times = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        times.append(&elapsed);
        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        times.append(&spacer);
        times.append(&total);

        let prev_btn = gtk::Button::from_icon_name("media-skip-backward-symbolic");
        let play_btn = gtk::Button::from_icon_name("media-playback-start-symbolic");
        let next_btn = gtk::Button::from_icon_name("media-skip-forward-symbolic");
        for b in [&prev_btn, &play_btn, &next_btn] {
            b.add_css_class("ctl");
            b.add_css_class("circular");
        }
        play_btn.add_css_class("play");
        let controls = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        controls.set_halign(gtk::Align::Center);
        controls.set_margin_top(6);
        controls.append(&prev_btn);
        controls.append(&play_btn);
        controls.append(&next_btn);

        let column = gtk::Box::new(gtk::Orientation::Vertical, 2);
        column.add_css_class("column");
        column.set_valign(gtk::Align::Center);
        column.set_hexpand(true);
        column.set_margin_start(6);
        column.append(&header);
        column.append(&title);
        column.append(&artist);
        column.append(&album);
        column.append(&progress);
        column.append(&times);
        column.append(&controls);

        let card = gtk::Box::new(gtk::Orientation::Horizontal, 14);
        card.add_css_class("card");
        card.set_halign(gtk::Align::Center);
        card.set_valign(gtk::Align::Center);
        card.append(&stage_slot);
        card.append(&column);

        let backdrop = Backdrop::new();
        let backdrop_pic = gtk::Picture::for_paintable(&backdrop);
        backdrop_pic.set_hexpand(true);
        backdrop_pic.set_vexpand(true);
        backdrop_pic.set_visible(false);

        let root = gtk::Overlay::new();
        root.set_child(Some(&backdrop_pic));
        root.add_overlay(&card);
        root.set_measure_overlay(&card, true);

        let window = gtk::ApplicationWindow::new(app);
        window.set_title(Some("Vinyl"));
        if cfg.opaque {
            window.add_css_class("opaque");
        }
        window.set_decorated(false);
        window.set_resizable(true);
        window.set_child(Some(&root));
        let placer = Rc::new(make_placer(&window));

        let start_fullscreen = cfg.start_fullscreen;
        let ui = Rc::new(Self {
            window,
            placer,
            backdrop_pic,
            backdrop,
            card,
            stage_slot,
            stage: RefCell::new(stage),
            player_label,
            title,
            artist,
            album,
            progress,
            elapsed,
            total,
            prev_btn,
            play_btn,
            next_btn,
            fs_btn,
            style_btn,
            style: RefCell::new(cfg.style.clone()),
            theme_colors: RefCell::new(theme_colors),
            theme_monitor: RefCell::new(None),
            flash_gen: Cell::new(0),
            cfg,
            state: RefCell::new(None),
            art_url: RefCell::new(None),
            art_tex: RefCell::new(None),
            art_generation: Cell::new(0),
            anim,
            on_command: RefCell::new(None),
        });

        ui.wire_events();
        ui.show_idle();
        ui.watch_theme();

        let this = ui.clone();
        glib::timeout_add_local(Duration::from_millis(250), move || {
            this.refresh_position();
            glib::ControlFlow::Continue
        });

        if start_fullscreen {
            let this = ui.clone();
            // Needs the window mapped to know the monitor size.
            ui.window.connect_map(move |_| {
                let this = this.clone();
                glib::idle_add_local_once(move || this.set_fullscreen(true));
            });
        }
        ui
    }

    pub fn connect_command<F: Fn(Command) + 'static>(&self, f: F) {
        *self.on_command.borrow_mut() = Some(Box::new(f));
    }

    fn emit(&self, cmd: Command) {
        if let Some(cb) = self.on_command.borrow().as_ref() {
            cb(cmd);
        }
    }

    fn wire_events(self: &Rc<Self>) {
        let this = self.clone();
        self.play_btn.connect_clicked(move |_| this.emit(Command::PlayPause));
        let this = self.clone();
        self.prev_btn.connect_clicked(move |_| this.emit(Command::Previous));
        let this = self.clone();
        self.next_btn.connect_clicked(move |_| this.emit(Command::Next));
        let this = self.clone();
        self.fs_btn.connect_clicked(move |_| this.set_fullscreen(!this.placer.is_fullscreen()));
        let this = self.clone();
        self.style_btn.connect_clicked(move |_| this.cycle_style(1));
        let right = gtk::GestureClick::new();
        right.set_button(3);
        let this = self.clone();
        right.connect_released(move |_, _, _, _| this.cycle_style(-1));
        self.style_btn.add_controller(right);

        self.wire_stage_click();

        // Drag the card to move the widget.
        let drag = gtk::GestureDrag::new();
        drag.set_button(1);
        let this = self.clone();
        drag.connect_drag_begin(move |g, x, y| {
            let device = g.device();
            this.placer.begin_drag(device.as_ref(), 1, x, y, g.current_event_time());
        });
        let this = self.clone();
        drag.connect_drag_update(move |_, dx, dy| this.placer.drag_by(dx, dy));
        let this = self.clone();
        drag.connect_drag_end(move |_, _, _| this.placer.end_drag());
        self.card.add_controller(drag);

        // Click the backdrop, or press Escape, to leave full screen.
        let click = gtk::GestureClick::new();
        let this = self.clone();
        click.connect_released(move |_, _, _, _| this.set_fullscreen(false));
        self.backdrop_pic.add_controller(click);
        let key = gtk::EventControllerKey::new();
        let this = self.clone();
        key.connect_key_pressed(move |_, k, _, _| {
            if k == gdk::Key::Escape && this.placer.is_fullscreen() {
                this.set_fullscreen(false);
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
        self.window.add_controller(key);
    }

    fn wire_stage_click(self: &Rc<Self>) {
        let click = gtk::GestureClick::new();
        click.set_button(1);
        let this = self.clone();
        click.connect_released(move |_, _, x, y| {
            let hit = {
                let stage = this.stage.borrow();
                let a = this.anim.borrow();
                stage.hit(&a, x, y)
            };
            match hit {
                Hit::Arm | Hit::Record => this.emit(Command::PlayPause),
                Hit::Sleeve => this.emit(Command::Raise),
                Hit::Nothing => {}
            }
        });
        self.stage.borrow().root.add_controller(click);
    }

    // --- full screen --------------------------------------------------------

    pub fn toggle_fullscreen(self: &Rc<Self>) {
        self.set_fullscreen(!self.placer.is_fullscreen());
    }

    pub fn set_fullscreen(self: &Rc<Self>, on: bool) {
        if self.placer.is_fullscreen() == on {
            return;
        }
        self.placer.set_fullscreen(on);
        let k = if on {
            let (mw, mh) = self.placer.monitor_size().unwrap_or((1920, 1080));
            ((mh as f64 * 0.62 / B_STAGE_H).min(mw as f64 * 0.42 / B_STAGE_W)).clamp(1.5, 8.0)
        } else {
            1.0
        };
        if on {
            self.window.add_css_class("takeover");
        } else {
            self.window.remove_css_class("takeover");
        }
        self.backdrop_pic.set_visible(on);
        self.fs_btn.set_icon_name(if on { "view-restore-symbolic" } else { "view-fullscreen-symbolic" });
        for l in [&self.title, &self.artist, &self.album] {
            l.set_max_width_chars(if on { 28 } else { 24 });
            l.set_width_chars(if on { 28 } else { 24 });
        }
        self.rebuild_stage(k);
    }

    fn rebuild_stage(self: &Rc<Self>, k: f64) {
        let old = self.stage.replace(Stage::new(
            k,
            self.style.borrow().clone(),
            self.theme_colors.borrow().clone(),
            self.cfg.show_arm,
            self.anim.clone(),
        ));
        self.stage_slot.remove(&old.root);
        {
            let stage = self.stage.borrow();
            self.stage_slot.append(&stage.root);
            let seed = seed_for(self.art_url.borrow().as_deref());
            stage.set_art(self.art_tex.borrow().as_ref(), seed);
            stage.apply(&self.anim.borrow());
        }
        self.wire_stage_click();
    }

    // --- vinyl style --------------------------------------------------------

    /// Step through the presets. A custom style (from `--vinyl`) starts at the first preset.
    pub fn cycle_style(self: &Rc<Self>, delta: i32) {
        let list = presets();
        let current = self.style.borrow().clone();
        let idx = list.iter().position(|p| p.style == current);
        let next = match idx {
            Some(i) => (i as i32 + delta).rem_euclid(list.len() as i32) as usize,
            None => 0,
        };
        self.set_style(list[next].style.clone());
    }

    pub fn set_style(self: &Rc<Self>, style: VinylStyle) {
        let label = style.label();
        *self.style.borrow_mut() = style.clone();
        self.stage.borrow().paintable.set_style(style.clone());
        self.style_btn.set_tooltip_text(Some(&format!("{label} (click: next, right-click: previous)")));
        crate::placement::save_config("style", &style.to_string());
        self.flash(&label);
    }

    /// Show a short message in the header, then restore the player name.
    fn flash(self: &Rc<Self>, text: &str) {
        let gen = self.flash_gen.get() + 1;
        self.flash_gen.set(gen);
        self.player_label.set_text(&text.to_uppercase());
        self.player_label.add_css_class("flash");
        let this = self.clone();
        glib::timeout_add_local_once(Duration::from_millis(1600), move || {
            if this.flash_gen.get() == gen {
                this.player_label.remove_css_class("flash");
                this.refresh_header();
            }
        });
    }

    fn refresh_header(&self) {
        let state = self.state.borrow();
        match state.as_ref() {
            Some(st) => self.player_label.set_text(&st.identity.to_uppercase()),
            None => self.player_label.set_text("NOTHING PLAYING"),
        }
    }

    fn watch_theme(self: &Rc<Self>) {
        let this = self.clone();
        let monitor = theme::watch(move || {
            let colors = theme::load();
            *this.theme_colors.borrow_mut() = colors.clone();
            this.stage.borrow().paintable.set_theme(colors);
        });
        *self.theme_monitor.borrow_mut() = monitor;
    }

    // --- state --------------------------------------------------------------

    pub fn set_state(self: &Rc<Self>, state: Option<&PlayerState>) {
        match state {
            None => self.show_idle(),
            Some(st) => self.show_player(st),
        }
        *self.state.borrow_mut() = state.cloned();
        self.refresh_position();
        self.ensure_animating();
    }

    fn show_idle(self: &Rc<Self>) {
        if !self.player_label.has_css_class("flash") {
            self.player_label.set_text("NOTHING PLAYING");
        }
        self.title.set_text("Drop the needle");
        self.artist.set_text("Play something");
        self.album.set_text("");
        self.progress.set_fraction(0.0);
        self.elapsed.set_text("0:00");
        self.total.set_text("0:00");
        self.play_btn.set_icon_name("media-playback-start-symbolic");
        for b in [&self.prev_btn, &self.play_btn, &self.next_btn] {
            b.set_sensitive(false);
        }
        self.set_art(None);
        let mut a = self.anim.borrow_mut();
        a.playing = false;
        a.progress = 0.0;
    }

    fn show_player(self: &Rc<Self>, st: &PlayerState) {
        if !self.player_label.has_css_class("flash") {
            self.player_label.set_text(&st.identity.to_uppercase());
        }
        self.title.set_text(if st.track.title.is_empty() { "Untitled" } else { &st.track.title });
        self.artist.set_text(&st.track.artist);
        self.album.set_text(&st.track.album);
        self.album.set_visible(!st.track.album.is_empty());
        let playing = st.status == Status::Playing;
        self.play_btn.set_icon_name(if playing {
            "media-playback-pause-symbolic"
        } else {
            "media-playback-start-symbolic"
        });
        self.play_btn.set_sensitive(true);
        self.prev_btn.set_sensitive(st.can_go_previous);
        self.next_btn.set_sensitive(st.can_go_next);
        self.set_art(st.track.art_url.clone());
        self.anim.borrow_mut().playing = playing;
    }

    fn refresh_position(self: &Rc<Self>) {
        let state = self.state.borrow();
        let Some(st) = state.as_ref() else {
            return;
        };
        let pos = st.position_us();
        let len = st.track.length_us;
        let frac = if len > 0 { (pos as f64 / len as f64).clamp(0.0, 1.0) } else { 0.0 };
        self.progress.set_fraction(frac);
        self.elapsed.set_text(&fmt_time(pos));
        self.total.set_text(&fmt_time(len));
        drop(state);
        let changed = {
            let mut a = self.anim.borrow_mut();
            let changed = (a.progress - frac).abs() > 0.002;
            a.progress = frac;
            changed
        };
        if changed {
            self.ensure_animating();
        }
    }

    fn set_art(self: &Rc<Self>, url: Option<String>) {
        if *self.art_url.borrow() == url {
            return;
        }
        *self.art_url.borrow_mut() = url.clone();
        let gen = self.art_generation.get() + 1;
        self.art_generation.set(gen);
        let Some(url) = url else {
            self.art_tex.replace(None);
            self.stage.borrow().set_art(None, 0);
            self.backdrop.set_art(None);
            return;
        };
        let this = self.clone();
        glib::spawn_future_local(async move {
            match art::load_texture(url.clone()).await {
                Ok(tex) if this.art_generation.get() == gen => {
                    this.art_tex.replace(Some(tex.clone()));
                    this.stage.borrow().set_art(Some(&tex), seed_for(Some(&url)));
                    this.backdrop.set_art(Some(tex));
                }
                Ok(_) => {}
                Err(e) => eprintln!("vinyl: art {url}: {e}"),
            }
        });
    }

    // --- animation ---------------------------------------------------------

    fn ensure_animating(self: &Rc<Self>) {
        let mut a = self.anim.borrow_mut();
        if a.tick.is_some() {
            return;
        }
        a.last_us = 0;
        let this = self.clone();
        let id = self.window.add_tick_callback(move |_, clock| this.tick(clock.frame_time()));
        a.tick = Some(id);
    }

    fn tick(self: &Rc<Self>, now_us: i64) -> glib::ControlFlow {
        let settled = {
            let mut a = self.anim.borrow_mut();
            if a.last_us != 0 && now_us - a.last_us < MIN_FRAME_US {
                return glib::ControlFlow::Continue;
            }
            let dt = if a.last_us == 0 { 0.0 } else { ((now_us - a.last_us) as f64 / 1e6).min(0.1) };
            a.last_us = now_us;

            let show_arm = self.cfg.show_arm;
            let rest = a.geom.rest_angle;
            let needle = a.needle_angle();
            let arm_parked = !show_arm || (a.arm_angle - rest).abs() < 1.5;
            let record_out = a.slide > 0.97;

            // Sequence: record slides out, then the arm drops. Arm lifts, then
            // the record slides back in.
            let slide_target = if a.playing {
                1.0
            } else if arm_parked {
                0.0
            } else {
                a.slide.round()
            };
            let arm_target = if a.playing && record_out { needle } else { rest };
            let vel_target = if a.playing { self.cfg.rpm * 6.0 } else { 0.0 };

            // Heavy platter: quick spin-up, slow coast-down.
            let k = if vel_target > a.ang_vel { 3.0 } else { 1.4 };
            a.ang_vel += (vel_target - a.ang_vel) * (1.0 - (-k * dt).exp());
            a.angle = (a.angle + a.ang_vel * dt).rem_euclid(360.0);
            a.slide += (slide_target - a.slide) * (1.0 - (-7.0 * dt).exp());
            a.arm_angle += (arm_target - a.arm_angle) * (1.0 - (-4.5 * dt).exp());

            let vel_done = vel_target == 0.0 && a.ang_vel.abs() < 0.5;
            let slide_done = (slide_target - a.slide).abs() < 0.001;
            let arm_done = (arm_target - a.arm_angle).abs() < 0.02;
            if vel_done {
                a.ang_vel = 0.0;
            }
            if slide_done {
                a.slide = slide_target;
            }
            if arm_done {
                a.arm_angle = arm_target;
            }
            vel_done && slide_done && arm_done
        };
        self.stage.borrow().apply(&self.anim.borrow());
        if settled {
            self.anim.borrow_mut().tick = None;
            glib::ControlFlow::Break
        } else {
            glib::ControlFlow::Continue
        }
    }
}

fn fmt_time(us: i64) -> String {
    let s = (us / 1_000_000).max(0);
    format!("{}:{:02}", s / 60, s % 60)
}

fn seed_for(url: Option<&str>) -> u32 {
    let mut h: u32 = 2166136261;
    for b in url.unwrap_or("").bytes() {
        h ^= b as u32;
        h = h.wrapping_mul(16777619);
    }
    h
}

/// Static reflection over the visible part of the record, so the disc reads as
/// spinning under a fixed light.
fn draw_sheen(cr: &cairo::Context, k: f64, h: f64, record_x: f64) {
    let r = B_RECORD * k / 2.0;
    let cx = record_x + r;
    let cy = B_PAD * k + r;
    let sleeve = B_SLEEVE * k;

    cr.rectangle(sleeve, 0.0, record_x + 2.0 * r - sleeve + 1.0, h);
    cr.clip();
    cr.arc(cx, cy, r, 0.0, 2.0 * PI);
    cr.clip();

    let g = cairo::LinearGradient::new(cx - r, cy - r, cx + r, cy + r);
    g.add_color_stop_rgba(0.00, 1.0, 1.0, 1.0, 0.00);
    g.add_color_stop_rgba(0.22, 1.0, 1.0, 1.0, 0.00);
    g.add_color_stop_rgba(0.32, 1.0, 1.0, 1.0, 0.13);
    g.add_color_stop_rgba(0.40, 1.0, 1.0, 1.0, 0.00);
    g.add_color_stop_rgba(0.62, 1.0, 1.0, 1.0, 0.00);
    g.add_color_stop_rgba(0.72, 1.0, 1.0, 1.0, 0.10);
    g.add_color_stop_rgba(0.82, 1.0, 1.0, 1.0, 0.00);
    let _ = cr.set_source(&g);
    let _ = cr.paint();
}
