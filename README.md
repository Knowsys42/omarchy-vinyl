# vinyl

A spinning-record "now playing" widget for [Omarchy](https://omarchy.org) (Hyprland),
in the spirit of MD Vinyl on macOS. It reads whatever is playing over
[MPRIS](https://specifications.freedesktop.org/mpris-spec/latest/), so it works
with Spotify, Cider, and any other player that speaks it. No accounts, no API keys.

Press play and the record slides out of its sleeve, spins up to 33⅓, and the
tone arm drops onto the groove; the stylus tracks inward as the song plays.
Pause and the arm lifts, the platter coasts down, and the record slides home.
Click the record or the arm to play/pause, click the sleeve to raise the
player window, and drag anywhere on the card to move the widget. The corner
button (or `--fullscreen`) opens a full-screen view with a large record over
the blurred album art; Escape or a click on the background brings it back.

## Build

Needs GTK 4, gtk4-layer-shell, and a Rust toolchain (all in the Arch repos):

```sh
sudo pacman -S --needed gtk4 gtk4-layer-shell rust
cargo build --release
install -Dm755 target/release/vinyl ~/.local/bin/vinyl
```

## Run

```sh
vinyl                       # bottom-right corner, on the desktop layer
vinyl --anchor top-left     # other corners/edges: top-right, bottom-left, top, bottom, left, right, center
vinyl --layer top           # sit above windows instead of under them
vinyl --layer window        # plain floating window (for testing)
vinyl --ignore brave        # never show browser tabs; repeatable
vinyl --prefer cider        # who wins when several players are playing
vinyl --rpm 45
vinyl --vinyl marble        # disc marbled from the album art's colours
vinyl --vinyl art           # solid disc in the art's dominant colour
vinyl --vinyl crimson       # any CSS colour: "#1e90ff", teal, ...
vinyl --no-arm              # hide the tone arm
vinyl --fullscreen          # start in the full-screen view
```

Drag the card and the new position is remembered in
`~/.config/vinyl/position` (with the monitor it was on). Passing `--anchor`
again forgets it.

By default the widget lives on the `bottom` layer: above the wallpaper, below
every window, on every workspace. Show the desktop and it's there.

Player choice: anything currently *playing* wins (ties broken by `--prefer`,
default `spotify` then `cider`), otherwise the last active player stays, otherwise
any paused player. Browsers show up too, because they publish MPRIS when a tab
plays media, hence `--ignore brave`.

## Autostart on Omarchy

Add to `~/.config/hypr/autostart.conf`:

```
exec-once = vinyl --ignore brave
```

Optional, to let Omarchy blur the card like its own panels, add to
`~/.config/hypr/looknfeel.conf` (or wherever your layer rules live):

```
layerrule = blur, vinyl
layerrule = ignorezero, vinyl
```

## Player notes

- **Spotify** publishes clean MPRIS with CDN album art. Everything works.
- **Cider** publishes MPRIS through Electron's Chromium media-session bridge
  (`org.mpris.MediaPlayer2.chromium.instanceNNN`, identity `Cider`). The name
  only appears once playback has started. Around track skips it sends a burst of
  partial updates, some with a blank title, so the widget keeps the last good
  text and re-reads the metadata a second later. Cider also has a hidden,
  deprecated native MPRIS module (`linux.useMpris: true` in
  `~/.config/sh.cider.genten/client-options.yml`); enabling it turns the
  Chromium bridge off and, in testing, registered nothing, so leave it alone.
- **Browsers** (Brave, Firefox, Chromium) publish MPRIS per tab. They're not
  ignored by default; pass `--ignore`.

## How it works

- `src/mpris.rs`: GDBus on the GLib main loop. Watches `NameOwnerChanged` for
  players appearing and vanishing, subscribes to `PropertiesChanged` and
  `Seeked` per player, polls `Position` while playing, interpolates in between.
- `src/art.rs`: loads `file://`, `http(s)://`, or `data:` art off the main
  thread and decodes it to a `gdk::Texture`.
- `src/record.rs`: a custom `gdk::Paintable` for the record. The disc is
  rendered once with Cairo (black, a solid colour, or a domain-warped noise
  marble in up to three k-means colours pulled from the album art) and uploaded
  as a texture; each frame only rotates that texture plus the art label inside
  the paintable's own snapshot, so a new angle invalidates one picture rather
  than relaying out the card. About 3% CPU while spinning at 60 fps.
- `src/arm.rs`: tone arm geometry (law of cosines from pivot to groove radius)
  and Cairo drawing, with a shadow that grows when the arm is lifted.
- `src/backdrop.rs`: the full-screen background, a GSK blur node over the art.
- `src/placement.rs`: anchored corners, drag-to-move via layer-shell margins
  (or a compositor move in window mode), position persistence, and the
  full-screen takeover (all four anchors, overlay layer, keyboard grab).
- `src/ui.rs`: the stage (sleeve, record, arm, sheen) built at a scale factor
  so full screen reuses the same layout, the text/controls column, and a
  frame-clock tick callback that sequences slide, spin and arm with easing.
