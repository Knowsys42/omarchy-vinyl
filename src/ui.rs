//! The widget: a record sleeve with a vinyl that slides out and spins while playing.

use crate::art;
use crate::mpris::{Command, PlayerState, Status};
use crate::record::RecordPaintable;
use gtk::cairo;
use gtk::gdk;
use gtk::glib;
use gtk::pango;
use gtk::prelude::*;
use std::cell::{Cell, RefCell};
use std::f64::consts::PI;
use std::rc::Rc;
use std::time::Duration;

pub const SLEEVE: f64 = 150.0;
pub const RECORD: f64 = 150.0;
const MIN_FRAME_US: i64 = 1_000_000 / 60;
const STAGE_PAD: f64 = 6.0;
const STAGE_W: f64 = SLEEVE + 96.0 + STAGE_PAD;
const STAGE_H: f64 = SLEEVE + STAGE_PAD * 2.0;
const RECORD_OUT_X: f64 = SLEEVE - RECORD + 96.0;
const RECORD_IN_X: f64 = SLEEVE - RECORD + 26.0;
const RECORD_Y: f64 = STAGE_PAD;

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
.label-art { border-radius: 9999px; background: #6b2a2a; }
.spindle { border-radius: 9999px; background: #0a0a0c; min-width: 7px; min-height: 7px; }
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
"#;

struct Anim {
    angle: f64,
    ang_vel: f64,
    target_vel: f64,
    offset: f64,
    target_offset: f64,
    last_us: i64,
    tick: Option<gtk::TickCallbackId>,
}

pub struct Ui {
    pub window: gtk::ApplicationWindow,
    stage: gtk::Fixed,
    record: gtk::Picture,
    paintable: RecordPaintable,
    sheen: gtk::DrawingArea,
    sleeve_pic: gtk::Image,
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
    rpm: f64,
    state: RefCell<Option<PlayerState>>,
    art_url: RefCell<Option<String>>,
    art_generation: Cell<u64>,
    anim: RefCell<Anim>,
    on_command: RefCell<Option<Box<dyn Fn(Command)>>>,
}

impl Ui {
    pub fn build(app: &gtk::Application, rpm: f64) -> Rc<Self> {
        let provider = gtk::CssProvider::new();
        provider.load_from_string(CSS);
        gtk::style_context_add_provider_for_display(
            &gdk::Display::default().expect("no display"),
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );

        // --- stage: sleeve + record ---------------------------------------
        let stage = gtk::Fixed::new();
        stage.set_size_request(STAGE_W as i32, STAGE_H as i32);
        stage.set_overflow(gtk::Overflow::Visible);

        let scale = gdk::Display::default()
            .and_then(|d| d.monitors().item(0))
            .and_downcast::<gdk::Monitor>()
            .map(|m| m.scale_factor())
            .unwrap_or(1)
            .max(2);
        let paintable = RecordPaintable::new(RECORD as i32, scale);
        let record = gtk::Picture::for_paintable(&paintable);
        record.set_size_request(RECORD as i32, RECORD as i32);
        record.set_can_shrink(false);

        let sleeve_pic = gtk::Image::new();
        sleeve_pic.set_pixel_size(SLEEVE as i32);
        let sleeve = gtk::Box::new(gtk::Orientation::Vertical, 0);
        sleeve.add_css_class("sleeve");
        sleeve.set_overflow(gtk::Overflow::Hidden);
        sleeve.set_size_request(SLEEVE as i32, SLEEVE as i32);
        sleeve.append(&sleeve_pic);

        stage.put(&record, RECORD_IN_X, RECORD_Y);
        stage.put(&sleeve, 0.0, STAGE_PAD);

        let sheen = gtk::DrawingArea::new();
        sheen.set_can_target(false);
        let stage_overlay = gtk::Overlay::new();
        stage_overlay.set_child(Some(&stage));
        stage_overlay.add_overlay(&sheen);

        // --- text + controls ------------------------------------------------
        let player_label = gtk::Label::new(None);
        player_label.add_css_class("player");
        player_label.set_xalign(0.0);

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
        column.set_valign(gtk::Align::Center);
        column.set_hexpand(true);
        column.set_margin_start(6);
        column.append(&player_label);
        column.append(&title);
        column.append(&artist);
        column.append(&album);
        column.append(&progress);
        column.append(&times);
        column.append(&controls);

        let card = gtk::Box::new(gtk::Orientation::Horizontal, 14);
        card.add_css_class("card");
        card.append(&stage_overlay);
        card.append(&column);

        let window = gtk::ApplicationWindow::new(app);
        window.set_title(Some("Vinyl"));
        window.set_decorated(false);
        window.set_resizable(false);
        window.set_child(Some(&card));

        let ui = Rc::new(Self {
            window,
            stage,
            record,
            paintable,
            sheen,
            sleeve_pic,
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
            rpm,
            state: RefCell::new(None),
            art_url: RefCell::new(None),
            art_generation: Cell::new(0),
            anim: RefCell::new(Anim {
                angle: 0.0,
                ang_vel: 0.0,
                target_vel: 0.0,
                offset: RECORD_IN_X,
                target_offset: RECORD_IN_X,
                last_us: 0,
                tick: None,
            }),
            on_command: RefCell::new(None),
        });

        ui.wire_events(&sleeve);
        ui.apply_transform();
        ui.show_idle();

        let this = ui.clone();
        glib::timeout_add_local(Duration::from_millis(250), move || {
            this.refresh_position();
            glib::ControlFlow::Continue
        });

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

    fn wire_events(self: &Rc<Self>, sleeve: &gtk::Box) {
        let this = self.clone();
        self.play_btn.connect_clicked(move |_| this.emit(Command::PlayPause));
        let this = self.clone();
        self.prev_btn.connect_clicked(move |_| this.emit(Command::Previous));
        let this = self.clone();
        self.next_btn.connect_clicked(move |_| this.emit(Command::Next));

        let click = gtk::GestureClick::new();
        let this = self.clone();
        click.connect_released(move |_, _, _, _| this.emit(Command::PlayPause));
        self.record.add_controller(click);

        let click = gtk::GestureClick::new();
        let this = self.clone();
        click.connect_released(move |_, _, _, _| this.emit(Command::Raise));
        sleeve.add_controller(click);

        let this = self.clone();
        self.sheen.set_draw_func(move |_, cr, w, h| {
            let offset = this.anim.borrow().offset;
            draw_sheen(cr, w as f64, h as f64, offset);
        });
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
        self.player_label.set_text("NOTHING PLAYING");
        self.title.set_text("Drop the needle");
        self.artist.set_text("Start Spotify or Cider");
        self.album.set_text("");
        self.progress.set_fraction(0.0);
        self.elapsed.set_text("0:00");
        self.total.set_text("0:00");
        self.play_btn.set_icon_name("media-playback-start-symbolic");
        for b in [&self.prev_btn, &self.play_btn, &self.next_btn] {
            b.set_sensitive(false);
        }
        self.set_art(None);
        let mut anim = self.anim.borrow_mut();
        anim.target_vel = 0.0;
        anim.target_offset = RECORD_IN_X;
    }

    fn show_player(self: &Rc<Self>, st: &PlayerState) {
        self.player_label.set_text(&st.identity.to_uppercase());
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
        let mut anim = self.anim.borrow_mut();
        anim.target_vel = if playing { self.rpm * 6.0 } else { 0.0 };
        anim.target_offset = if playing { RECORD_OUT_X } else { RECORD_IN_X };
    }

    fn refresh_position(&self) {
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
    }

    fn set_art(self: &Rc<Self>, url: Option<String>) {
        if *self.art_url.borrow() == url {
            return;
        }
        *self.art_url.borrow_mut() = url.clone();
        let gen = self.art_generation.get() + 1;
        self.art_generation.set(gen);
        let Some(url) = url else {
            self.sleeve_pic.set_paintable(None::<&gdk::Paintable>);
            self.paintable.set_label(None);
            return;
        };
        let this = self.clone();
        glib::spawn_future_local(async move {
            match art::load_texture(url.clone()).await {
                Ok(tex) if this.art_generation.get() == gen => {
                    this.sleeve_pic.set_paintable(Some(&tex));
                    this.paintable.set_label(Some(tex));
                }
                Ok(_) => {}
                Err(e) => eprintln!("vinyl: art {url}: {e}"),
            }
        });
    }

    // --- animation ---------------------------------------------------------

    fn apply_transform(&self) {
        let anim = self.anim.borrow();
        self.paintable.set_angle(anim.angle);
        self.stage.move_(&self.record, anim.offset, RECORD_Y);
    }

    fn ensure_animating(self: &Rc<Self>) {
        let mut anim = self.anim.borrow_mut();
        if anim.tick.is_some() {
            return;
        }
        anim.last_us = 0;
        let this = self.clone();
        let id = self.stage.add_tick_callback(move |_, clock| this.tick(clock.frame_time()));
        anim.tick = Some(id);
    }

    fn tick(self: &Rc<Self>, now_us: i64) -> glib::ControlFlow {
        if self.anim.borrow().last_us != 0 && now_us - self.anim.borrow().last_us < MIN_FRAME_US {
            return glib::ControlFlow::Continue;
        }
        let (settled, moved) = {
            let mut a = self.anim.borrow_mut();
            let dt = if a.last_us == 0 { 0.0 } else { ((now_us - a.last_us) as f64 / 1e6).min(0.1) };
            a.last_us = now_us;
            let before = a.offset;

            // Heavy platter: quick spin-up, slow coast-down.
            let k = if a.target_vel > a.ang_vel { 3.0 } else { 1.4 };
            a.ang_vel += (a.target_vel - a.ang_vel) * (1.0 - (-k * dt).exp());
            a.angle = (a.angle + a.ang_vel * dt).rem_euclid(360.0);
            a.offset += (a.target_offset - a.offset) * (1.0 - (-7.0 * dt).exp());

            let vel_done = a.target_vel == 0.0 && a.ang_vel.abs() < 0.5;
            let off_done = (a.target_offset - a.offset).abs() < 0.05;
            if vel_done {
                a.ang_vel = 0.0;
            }
            if off_done {
                a.offset = a.target_offset;
            }
            (vel_done && off_done, (a.offset - before).abs() > 1e-6)
        };
        self.paintable.set_angle(self.anim.borrow().angle);
        if moved {
            self.stage.move_(&self.record, self.anim.borrow().offset, RECORD_Y);
            self.sheen.queue_draw();
        }
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

// --- drawing ---------------------------------------------------------------

/// Static reflection over the visible part of the record, so the disc reads as
/// spinning under a fixed light.
fn draw_sheen(cr: &cairo::Context, _w: f64, h: f64, offset: f64) {
    let cx = offset + RECORD / 2.0;
    let cy = RECORD_Y + RECORD / 2.0;
    let r = RECORD / 2.0;

    // Only the part of the disc that sticks out past the sleeve.
    cr.rectangle(SLEEVE, 0.0, offset + RECORD - SLEEVE + 1.0, h);
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
