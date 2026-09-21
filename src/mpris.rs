//! Watches every MPRIS player on the session bus and picks the one worth showing.
//!
//! Everything runs on the GLib main loop through GDBus, so there are no threads
//! and no channels: callbacks mutate a shared `Rc<RefCell<..>>` and notify the UI.

use gtk::gio;
use gtk::glib;
use gtk::prelude::*;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Duration, Instant};

pub const MPRIS_PREFIX: &str = "org.mpris.MediaPlayer2.";
const MPRIS_PATH: &str = "/org/mpris/MediaPlayer2";
const ROOT_IFACE: &str = "org.mpris.MediaPlayer2";
const PLAYER_IFACE: &str = "org.mpris.MediaPlayer2.Player";
const PROPS_IFACE: &str = "org.freedesktop.DBus.Properties";
const DBUS_NAME: &str = "org.freedesktop.DBus";
const DBUS_PATH: &str = "/org/freedesktop/DBus";
const CALL_TIMEOUT_MS: i32 = 2000;
const POSITION_POLL: Duration = Duration::from_millis(1500);
const METADATA_SETTLE: Duration = Duration::from_millis(1200);
/// Longest track length we will believe. Players occasionally publish
/// nonsense here: Cider has been seen reporting i64::MAX microseconds, which
/// rendered as a total of 153722867280:54. Anything outside this range is
/// treated as "length unknown" instead.
const MAX_TRACK_US: i64 = 24 * 3600 * 1_000_000;

/// A published duration, or 0 when it is missing or not credible.
fn sane_duration(value: i64) -> i64 {
    if (1..=MAX_TRACK_US).contains(&value) { value } else { 0 }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Status {
    Playing,
    Paused,
    #[default]
    Stopped,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Track {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub art_url: Option<String>,
    pub length_us: i64,
    pub track_id: String,
}

#[derive(Clone, Debug)]
pub struct PlayerState {
    pub bus_name: String,
    pub identity: String,
    pub status: Status,
    pub track: Track,
    pub rate: f64,
    pub can_go_next: bool,
    pub can_go_previous: bool,
    pub can_seek: bool,
    /// Position reported by the player, and when we sampled it.
    position_us: i64,
    sampled_at: Instant,
    /// Wall clock of the last change, used to break ties between players.
    last_change: Instant,
}

impl PlayerState {
    fn new(bus_name: &str) -> Self {
        Self {
            bus_name: bus_name.to_string(),
            identity: String::new(),
            status: Status::Stopped,
            track: Track::default(),
            rate: 1.0,
            can_go_next: false,
            can_go_previous: false,
            can_seek: false,
            position_us: 0,
            sampled_at: Instant::now(),
            last_change: Instant::now(),
        }
    }

    /// Best guess of the current position, interpolated while playing.
    pub fn position_us(&self) -> i64 {
        let mut pos = self.position_us;
        if self.status == Status::Playing {
            let elapsed = self.sampled_at.elapsed().as_micros() as f64 * self.rate.max(0.0);
            pos += elapsed as i64;
        }
        if self.track.length_us > 0 {
            pos = pos.clamp(0, self.track.length_us);
        }
        pos.max(0)
    }

    pub fn has_track(&self) -> bool {
        !self.track.title.is_empty() || self.track.art_url.is_some()
    }
}

#[derive(Clone, Copy, Debug)]
pub enum Command {
    PlayPause,
    Next,
    Previous,
    Raise,
    /// Jump to an absolute position, in microseconds from the start.
    Seek(i64),
}

/// Which players to prefer or hide, matched case-insensitively against the
/// player's `Identity` ("Spotify", "Cider", "Brave", ...) or its bus name.
#[derive(Clone, Debug, Default)]
pub struct Preferences {
    pub prefer: Vec<String>,
    pub ignore: Vec<String>,
}

impl Preferences {
    fn matches(list: &[String], state: &PlayerState) -> Option<usize> {
        let ident = state.identity.to_lowercase();
        let name = state.bus_name.to_lowercase();
        list.iter().position(|p| {
            let p = p.to_lowercase();
            ident == p || ident.contains(&p) || name.contains(&p)
        })
    }
    fn ignored(&self, state: &PlayerState) -> bool {
        Self::matches(&self.ignore, state).is_some()
    }
    fn rank(&self, state: &PlayerState) -> usize {
        Self::matches(&self.prefer, state).unwrap_or(self.prefer.len())
    }
}

type ChangedFn = Box<dyn Fn(Option<&PlayerState>)>;

struct Inner {
    players: HashMap<String, PlayerState>,
    subscriptions: HashMap<String, Vec<gio::SignalSubscription>>,
    active: Option<String>,
    on_changed: Option<ChangedFn>,
}

pub struct Mpris {
    conn: gio::DBusConnection,
    prefs: Preferences,
    inner: RefCell<Inner>,
}

impl Mpris {
    pub async fn connect(prefs: Preferences) -> Result<Rc<Self>, glib::Error> {
        let conn = gio::bus_get_future(gio::BusType::Session).await?;
        Ok(Rc::new(Self {
            conn,
            prefs,
            inner: RefCell::new(Inner {
                players: HashMap::new(),
                subscriptions: HashMap::new(),
                active: None,
                on_changed: None,
            }),
        }))
    }

    pub fn connect_changed<F: Fn(Option<&PlayerState>) + 'static>(&self, f: F) {
        self.inner.borrow_mut().on_changed = Some(Box::new(f));
    }

    /// Start watching the bus. Safe to call once.
    pub fn start(self: &Rc<Self>) {
        self.watch_name_owner_changes();
        let this = self.clone();
        glib::spawn_future_local(async move {
            match this.list_names().await {
                Ok(names) => {
                    for name in names {
                        this.add_player(name);
                    }
                }
                Err(e) => eprintln!("vinyl: ListNames failed: {e}"),
            }
        });
        let this = self.clone();
        glib::timeout_add_local(POSITION_POLL, move || {
            this.poll_position();
            glib::ControlFlow::Continue
        });
    }

    pub fn command(self: &Rc<Self>, cmd: Command) {
        let Some(name) = self.inner.borrow().active.clone() else {
            return;
        };
        if let Command::Seek(target) = cmd {
            self.seek(name, target);
            return;
        }
        let (iface, method) = match cmd {
            Command::PlayPause => (PLAYER_IFACE, "PlayPause"),
            Command::Next => (PLAYER_IFACE, "Next"),
            Command::Previous => (PLAYER_IFACE, "Previous"),
            Command::Raise => (ROOT_IFACE, "Raise"),
            Command::Seek(_) => unreachable!("handled above"),
        };
        let this = self.clone();
        glib::spawn_future_local(async move {
            if let Err(e) = this.call(&name, iface, method, None, None).await {
                eprintln!("vinyl: {method} on {name} failed: {e}");
            }
        });
    }

    // ---- bus plumbing -----------------------------------------------------

    async fn call(
        &self,
        bus_name: &str,
        iface: &str,
        method: &str,
        args: Option<&glib::Variant>,
        reply: Option<&glib::VariantTy>,
    ) -> Result<glib::Variant, glib::Error> {
        self.conn
            .call_future(
                Some(bus_name),
                MPRIS_PATH,
                iface,
                method,
                args,
                reply,
                gio::DBusCallFlags::NONE,
                CALL_TIMEOUT_MS,
            )
            .await
    }

    async fn list_names(&self) -> Result<Vec<String>, glib::Error> {
        let reply = self
            .conn
            .call_future(
                Some(DBUS_NAME),
                DBUS_PATH,
                DBUS_NAME,
                "ListNames",
                None,
                Some(glib::VariantTy::new("(as)").unwrap()),
                gio::DBusCallFlags::NONE,
                CALL_TIMEOUT_MS,
            )
            .await?;
        let names: Vec<String> = reply.child_value(0).get().unwrap_or_default();
        Ok(names
            .into_iter()
            .filter(|n| n.starts_with(MPRIS_PREFIX))
            .collect())
    }

    fn watch_name_owner_changes(self: &Rc<Self>) {
        let this = self.clone();
        let sub = self.conn.subscribe_to_signal(
            Some(DBUS_NAME),
            Some(DBUS_NAME),
            Some("NameOwnerChanged"),
            Some(DBUS_PATH),
            Some(ROOT_IFACE),
            gio::DBusSignalFlags::MATCH_ARG0_NAMESPACE,
            move |sig| {
                let name: String = sig.parameters.child_value(0).get().unwrap_or_default();
                let new_owner: String = sig.parameters.child_value(2).get().unwrap_or_default();
                if !name.starts_with(MPRIS_PREFIX) {
                    return;
                }
                if new_owner.is_empty() {
                    this.remove_player(&name);
                } else {
                    this.add_player(name);
                }
            },
        );
        self.inner.borrow_mut().subscriptions.insert(DBUS_NAME.to_string(), vec![sub]);
    }

    fn add_player(self: &Rc<Self>, name: String) {
        {
            let mut inner = self.inner.borrow_mut();
            if inner.players.contains_key(&name) {
                return;
            }
            inner.players.insert(name.clone(), PlayerState::new(&name));

            let mut subs = Vec::new();
            let this = self.clone();
            let n = name.clone();
            subs.push(self.conn.subscribe_to_signal(
                Some(&name),
                Some(PROPS_IFACE),
                Some("PropertiesChanged"),
                Some(MPRIS_PATH),
                None,
                gio::DBusSignalFlags::NONE,
                move |sig| this.on_properties_changed(&n, sig.parameters),
            ));
            let this = self.clone();
            let n = name.clone();
            subs.push(self.conn.subscribe_to_signal(
                Some(&name),
                Some(PLAYER_IFACE),
                Some("Seeked"),
                Some(MPRIS_PATH),
                None,
                gio::DBusSignalFlags::NONE,
                move |sig| {
                    let pos: i64 = sig.parameters.child_value(0).get().unwrap_or(0);
                    this.update(&n, |st| {
                        st.position_us = pos;
                        st.sampled_at = Instant::now();
                    });
                },
            ));
            inner.subscriptions.insert(name.clone(), subs);
        }

        let this = self.clone();
        glib::spawn_future_local(async move {
            let props_ty = glib::VariantTy::new("(a{sv})").unwrap();
            let root = this
                .call(&name, PROPS_IFACE, "GetAll", Some(&(ROOT_IFACE,).to_variant()), Some(props_ty))
                .await;
            let player = this
                .call(&name, PROPS_IFACE, "GetAll", Some(&(PLAYER_IFACE,).to_variant()), Some(props_ty))
                .await;
            let root = root.map(|v| dict(&v.child_value(0))).unwrap_or_default();
            let player = match player {
                Ok(v) => dict(&v.child_value(0)),
                Err(e) => {
                    eprintln!("vinyl: {name}: GetAll failed: {e}");
                    return;
                }
            };
            this.update(&name, |st| {
                if let Some(id) = root.get("Identity").and_then(|v| v.str()) {
                    st.identity = id.to_string();
                }
                if st.identity.is_empty() {
                    st.identity = name.trim_start_matches(MPRIS_PREFIX).to_string();
                }
                apply_player_props(st, &player);
            });
        });
    }

    fn remove_player(&self, name: &str) {
        let mut inner = self.inner.borrow_mut();
        // Dropping the subscriptions unsubscribes them.
        inner.subscriptions.remove(name);
        inner.players.remove(name);
        if inner.active.as_deref() == Some(name) {
            inner.active = None;
        }
        drop(inner);
        self.reselect();
    }

    fn on_properties_changed(self: &Rc<Self>, name: &str, params: &glib::Variant) {
        let iface: String = params.child_value(0).get().unwrap_or_default();
        if iface != PLAYER_IFACE {
            return;
        }
        let changed = dict(&params.child_value(1));
        let touched_playback = changed.contains_key("PlaybackStatus")
            || changed.contains_key("Metadata")
            || changed.contains_key("Rate");
        self.update(name, |st| apply_player_props(st, &changed));
        if touched_playback {
            // Most players omit Position from the signal; fetch it fresh.
            self.fetch_position(name.to_string());
        }
        if changed.contains_key("Metadata") {
            // Chromium-based players (Cider) fire a burst of partial or blank
            // Metadata updates around a track change. Re-read the settled value.
            let this = self.clone();
            let name = name.to_string();
            glib::timeout_add_local_once(METADATA_SETTLE, move || this.fetch_metadata(name));
        }
    }

    fn fetch_metadata(self: &Rc<Self>, name: String) {
        let this = self.clone();
        glib::spawn_future_local(async move {
            let args = (PLAYER_IFACE, "Metadata").to_variant();
            let reply_ty = glib::VariantTy::new("(v)").unwrap();
            if let Ok(reply) = this.call(&name, PROPS_IFACE, "Get", Some(&args), Some(reply_ty)).await {
                if let Some(meta) = reply.child_value(0).as_variant() {
                    let mut props = HashMap::new();
                    props.insert("Metadata".to_string(), meta);
                    this.update(&name, |st| apply_player_props(st, &props));
                }
            }
        });
    }

    fn poll_position(self: &Rc<Self>) {
        let active = self.inner.borrow().active.clone();
        if let Some(name) = active {
            let playing = self
                .inner
                .borrow()
                .players
                .get(&name)
                .map(|p| p.status == Status::Playing)
                .unwrap_or(false);
            if playing {
                self.fetch_position(name);
            }
        }
    }

    /// Jump to an absolute position. `SetPosition` is the accurate way and
    /// wants the track's object path; players that publish none (or the spec's
    /// `NoTrack`) get the relative `Seek` worked out from where we think the
    /// track is, which is what our own progress bar was showing anyway.
    fn seek(self: &Rc<Self>, name: String, target: i64) {
        let Some((track_id, current, length, can_seek)) = self.inner.borrow().players.get(&name).map(|st| {
            (st.track.track_id.clone(), st.position_us(), st.track.length_us, st.can_seek)
        }) else {
            return;
        };
        if !can_seek || length <= 0 {
            return;
        }
        let target = target.clamp(0, length);
        let (method, args) = match seek_call(&track_id, target, current) {
            SeekCall::SetPosition(path) => ("SetPosition", (path, target).to_variant()),
            SeekCall::Seek(offset) => ("Seek", (offset,).to_variant()),
        };
        // Move the needle now; the player confirms the position right after.
        self.update_quiet(&name, |st| {
            st.position_us = target;
            st.sampled_at = Instant::now();
        });
        let this = self.clone();
        glib::spawn_future_local(async move {
            if let Err(e) = this.call(&name, PLAYER_IFACE, method, Some(&args), None).await {
                eprintln!("vinyl: {method} on {name} failed: {e}");
            }
            this.fetch_position(name);
        });
    }

    fn fetch_position(self: &Rc<Self>, name: String) {
        let this = self.clone();
        glib::spawn_future_local(async move {
            let args = (PLAYER_IFACE, "Position").to_variant();
            let reply_ty = glib::VariantTy::new("(v)").unwrap();
            if let Ok(reply) = this.call(&name, PROPS_IFACE, "Get", Some(&args), Some(reply_ty)).await {
                if let Some(pos) = reply.child_value(0).as_variant().and_then(|v| variant_i64(&v)) {
                    if (0..=MAX_TRACK_US).contains(&pos) {
                        this.update_quiet(&name, |st| {
                            st.position_us = pos;
                            st.sampled_at = Instant::now();
                        });
                    }
                }
            }
        });
    }

    // ---- state ------------------------------------------------------------

    /// Mutate a player and re-run selection + notification.
    fn update(&self, name: &str, f: impl FnOnce(&mut PlayerState)) {
        {
            let mut inner = self.inner.borrow_mut();
            let Some(st) = inner.players.get_mut(name) else {
                return;
            };
            f(st);
            st.last_change = Instant::now();
        }
        self.reselect();
    }

    /// Mutate without changing selection; still notifies if it's the active player.
    fn update_quiet(&self, name: &str, f: impl FnOnce(&mut PlayerState)) {
        let mut inner = self.inner.borrow_mut();
        let Some(st) = inner.players.get_mut(name) else {
            return;
        };
        f(st);
        let is_active = inner.active.as_deref() == Some(name);
        drop(inner);
        if is_active {
            self.notify();
        }
    }

    fn reselect(&self) {
        let mut inner = self.inner.borrow_mut();
        let candidates: Vec<&PlayerState> = inner
            .players
            .values()
            .filter(|p| !self.prefs.ignored(p) && p.has_track())
            .collect();

        let pick = |status: Status| -> Option<String> {
            candidates
                .iter()
                .filter(|p| p.status == status)
                .min_by_key(|p| (self.prefs.rank(p), std::cmp::Reverse(p.last_change)))
                .map(|p| p.bus_name.clone())
        };

        let current = inner.active.clone();
        let current_ok = current
            .as_ref()
            .and_then(|n| inner.players.get(n))
            .map(|p| !self.prefs.ignored(p) && p.has_track())
            .unwrap_or(false);

        let next = pick(Status::Playing)
            .or(if current_ok { current.clone() } else { None })
            .or_else(|| pick(Status::Paused))
            .or_else(|| pick(Status::Stopped));

        inner.active = next;
        drop(inner);
        self.notify();
    }

    fn notify(&self) {
        let inner = self.inner.borrow();
        let state = inner.active.as_ref().and_then(|n| inner.players.get(n)).cloned();
        if let Some(cb) = inner.on_changed.as_ref() {
            cb(state.as_ref());
        }
    }
}

// ---- variant helpers ------------------------------------------------------

/// Flatten an `a{sv}` into a map, unboxing each `v`.
fn dict(v: &glib::Variant) -> HashMap<String, glib::Variant> {
    let mut out = HashMap::new();
    if !v.is_container() {
        return out;
    }
    for i in 0..v.n_children() {
        let entry = v.child_value(i);
        if entry.n_children() != 2 {
            continue;
        }
        let Some(key) = entry.child_value(0).str().map(str::to_string) else {
            continue;
        };
        let val = entry.child_value(1);
        let val = val.as_variant().unwrap_or(val);
        out.insert(key, val);
    }
    out
}

fn variant_i64(v: &glib::Variant) -> Option<i64> {
    v.get::<i64>()
        .or_else(|| v.get::<u64>().map(|x| x as i64))
        .or_else(|| v.get::<i32>().map(i64::from))
        .or_else(|| v.get::<u32>().map(i64::from))
        .or_else(|| v.get::<f64>().map(|x| x as i64))
}

fn variant_string_list(v: &glib::Variant) -> String {
    if let Some(list) = v.get::<Vec<String>>() {
        list.into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(", ")
    } else {
        v.str().unwrap_or_default().to_string()
    }
}

#[derive(Debug, PartialEq)]
enum SeekCall {
    SetPosition(glib::variant::ObjectPath),
    Seek(i64),
}

/// `SetPosition` is absolute and immune to a stale position, but it needs the
/// track's object path. Players that publish none, publish junk, or publish the
/// spec's `NoTrack` placeholder get the relative `Seek` instead.
fn seek_call(track_id: &str, target: i64, current: i64) -> SeekCall {
    if !track_id.ends_with("/NoTrack") {
        if let Ok(path) = glib::variant::ObjectPath::try_from(track_id) {
            return SeekCall::SetPosition(path);
        }
    }
    SeekCall::Seek(target - current)
}

fn apply_player_props(st: &mut PlayerState, props: &HashMap<String, glib::Variant>) {
    if let Some(s) = props.get("PlaybackStatus").and_then(|v| v.str()) {
        st.status = match s {
            "Playing" => Status::Playing,
            "Paused" => Status::Paused,
            _ => Status::Stopped,
        };
    }
    if let Some(r) = props.get("Rate").and_then(|v| v.get::<f64>()) {
        st.rate = r;
    }
    if let Some(b) = props.get("CanGoNext").and_then(|v| v.get::<bool>()) {
        st.can_go_next = b;
    }
    if let Some(b) = props.get("CanGoPrevious").and_then(|v| v.get::<bool>()) {
        st.can_go_previous = b;
    }
    if let Some(b) = props.get("CanSeek").and_then(|v| v.get::<bool>()) {
        st.can_seek = b;
    }
    if let Some(m) = props.get("Metadata") {
        let m = dict(m);
        let mut track = Track {
            title: m.get("xesam:title").and_then(|v| v.str()).unwrap_or_default().to_string(),
            artist: m.get("xesam:artist").map(variant_string_list).unwrap_or_default(),
            album: m.get("xesam:album").and_then(|v| v.str()).unwrap_or_default().to_string(),
            art_url: m
                .get("mpris:artUrl")
                .and_then(|v| v.str())
                .filter(|s| !s.is_empty())
                .map(str::to_string),
            length_us: sane_duration(m.get("mpris:length").and_then(variant_i64).unwrap_or(0)),
            track_id: m
                .get("mpris:trackid")
                .and_then(|v| v.str())
                .unwrap_or_default()
                .to_string(),
        };
        // Chromium-based players (Cider) send blank text in some updates.
        // Treat those as partial: keep the text we have, take what's new.
        if track.title.is_empty() && track.artist.is_empty() && !st.track.title.is_empty() {
            track.title = st.track.title.clone();
            track.artist = st.track.artist.clone();
            if track.album.is_empty() {
                track.album = st.track.album.clone();
            }
            if track.art_url.is_none() {
                track.art_url = st.track.art_url.clone();
            }
            if track.length_us == 0 {
                track.length_us = st.track.length_us;
            }
        }
        if track != st.track {
            let new_song = track.title != st.track.title || track.artist != st.track.artist;
            st.track = track;
            if new_song {
                // Position restarts unless the player says otherwise.
                st.position_us = 0;
                st.sampled_at = Instant::now();
            }
        }
    }
    if let Some(p) = props.get("Position").and_then(variant_i64) {
        st.position_us = if (0..=MAX_TRACK_US).contains(&p) { p } else { 0 };
        st.sampled_at = Instant::now();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(s: &str) -> SeekCall {
        SeekCall::SetPosition(glib::variant::ObjectPath::try_from(s).unwrap())
    }

    #[test]
    fn seeking_prefers_the_absolute_call_when_the_track_has_a_path() {
        assert_eq!(seek_call("/org/mpris/MediaPlayer2/vinyldemo/1", 90, 30), path("/org/mpris/MediaPlayer2/vinyldemo/1"));
        assert_eq!(seek_call("/com/brave/MediaPlayer2/TrackList/TrackCC8C", 90, 30), path("/com/brave/MediaPlayer2/TrackList/TrackCC8C"));
    }

    #[test]
    fn seeking_falls_back_to_the_relative_call() {
        // No track, no path published, and a path D-Bus would reject.
        for id in ["/org/mpris/MediaPlayer2/TrackList/NoTrack", "", "spotify:track:4u7e", "/trailing/"] {
            assert_eq!(seek_call(id, 90, 30), SeekCall::Seek(60), "{id}");
        }
        assert_eq!(seek_call("", 10, 30), SeekCall::Seek(-20), "seeking backwards");
    }
}
