# Changelog

All notable changes to Vinyl are recorded here. Dates are the day the work
landed on `main`.

## 0.4.0 — 2026-09-21

The record picks up a hand on the platter, the tone arm stops throwing a
shadow onto the desktop, and the label is printed on paper.

### Added

- **Scratching.** Grab the record with the middle button and drag: it turns
  under the pointer instead of under the motor, the stylus rides whatever
  groove you leave it on, and letting go seeks the player there. One turn of
  the platter is one turn's worth of music — 1.8 seconds on a 33 1/3 pressing,
  1.33 on a 45. Deliberately undocumented in the README.

### Fixed

- **A negative-width warning from the progress bar.** The filled part of the
  slider measured below zero at the left end, where the knob's negative margin
  pulled it under, and GTK complained on stderr at every start.

### Changed

- **The tone arm's shadow falls on the record and nowhere else.** It was drawn
  wherever the arm was, so the part of the arm hanging past the disc — the
  pivot and counterweight, most of the travel while the record is in its
  sleeve — laid a dark smear straight onto the desktop, with nothing under it
  to catch one. It is clipped to the disc now, and softened: three overlapping
  copies instead of one hard one, so the edge tapers the way a real penumbra
  does and spreads further as the arm lifts.
- **The label is printed paper.** A fine tooth from the fibres and a slower
  mottle, so the ink is not perfectly even, over the album art as well as over
  a plain label. It turns with the record, because print does.
- **One sheen, not two.** The static reflection drawn over the record has been
  folded into the lit disc added in 0.3.0, which does the same job from the
  groove geometry and moves.

## 0.3.0 — 2026-09-21

Three of the buttons were rendering as broken images on any desktop whose icon
theme is not installed. Fixing that meant drawing the glyphs, which is also
what the record and the progress bar wanted: light that behaves, and a bar you
can drag.

### Added

- **Drag the progress bar to seek.** The bar is a real slider now, and the tone
  arm follows it as you drag. A drag sends one `SetPosition` when you stop
  moving rather than one per pixel; players that publish no usable track path
  get the relative `Seek` instead. Players that report `CanSeek` as false keep
  a plain, handle-less bar.
- **A specular sheen on the record.** The lamp stays put while the disc turns
  under it, so the light now reads off the grooves in two lobes the way a real
  pressing does. No pressing is perfectly flat, so the highlight wobbles once
  per revolution.

### Fixed

- **The skip and full-screen buttons no longer render as broken images.** They
  were drawn from the icon theme, and a desktop pointed at a theme that is not
  installed leaves GTK with only the handful of icons compiled into it — which
  covers play and pause, but not skip or restore. Every control glyph is drawn
  by Vinyl now, so the icon theme cannot break them.

## 0.2.3 — 2026-09-14

Album art looked soft in the full-screen view, and a misbehaving player could
put nonsense on the clock.

### Fixed

- **Full-screen album art is no longer stretched into a blur.** Players publish
  a thumbnail over MPRIS, often 150 pixels square, and the full-screen view was
  enlarging that across an 827-pixel sleeve. Two changes address it. Where the
  artwork service encodes a size in the URL, Vinyl now requests a large
  variant, falling back to the original URL if the larger one cannot be
  fetched. Apple's `<width>x<height>` path segment and Spotify's image-id size
  prefix are both recognised. Independently, the full-screen stage is capped so
  the art is never enlarged more than 2.5 times: a 150-pixel cover now draws a
  375-pixel sleeve instead of an 827-pixel one, while a cover of 1000 pixels or
  more still fills the space. The blurred backdrop covers the screen either
  way, and the scale is recalculated if sharper art arrives while you are
  already in full screen.
- **A track length the player cannot possibly mean is ignored.** One player was
  seen reporting `i64::MAX` microseconds, which rendered as a total time of
  `153722867280:54`. Any length or position outside zero to twenty-four hours
  is now treated as unknown: the total is left blank and the progress bar stays
  empty rather than showing an invented number.
- **The artist line is hidden when the player publishes no artist**, matching
  what the album line already did.

## 0.2.2 — 2026-09-11

### Security

Closed the executable trust boundary in `bin/vinyl-ctl`, from the second round
of marketplace security review.

- Only the artifact built inside the plugin's own checkout is executed. An
  ambient-`PATH` `vinyl` is ignored rather than preferred, and the artifact is
  refused unless it is a regular file, not a symlink, owned by the invoking
  user, with no group or other write permission.
- Every helper the script calls (`stat`, `mkdir`, `rmdir`, `rm`, `pacman`,
  `cargo`, `notify-send`) is resolved from a closed trusted path and must be a
  root-owned regular file that only root can write. `PATH` is replaced with
  that list, so cargo's own resolution of `rustc` and the linker is bound the
  same way. Missing helpers fail closed; only `notify-send` is optional, and
  its absence silences notifications rather than falling back.
- The private state directory is pinned by descriptor once validated, and the
  build lock and log are addressed only through `/proc/self/fd`, so a pathname
  swapped after validation cannot redirect either one.

## 0.2.1 — 2026-09-10

### Security

First round of marketplace security review, also in `bin/vinyl-ctl`.

- The build runs `cargo build --release --locked`, and `Cargo.lock` is
  committed, so the dependency graph is exactly what the lockfile records.
- The build lock and log moved out of world-writable `/tmp` into a private
  directory (`$XDG_RUNTIME_DIR/vinyl`, otherwise `$HOME/.cache/vinyl`), checked
  to be a real directory, not a symlink, owned by the invoking user, with no
  foreign write permission.
- The lock is a directory created with `mkdir`, which is atomic and cannot be
  satisfied by a planted symlink. The log is created with `O_EXCL` after any
  existing path is removed.

## 0.2.0 — 2026-09-09

### Added

- **Tone arm.** The record slides out of its sleeve, spins up to 33⅓, and the
  arm drops onto the outer groove; the stylus tracks inward as the song plays.
  Pausing lifts the arm, coasts the platter down and slides the record home.
  Clicking the arm toggles playback. `--no-arm` hides it.
- **Fourteen pressings**, selected with a button on the card or `--vinyl`, and
  remembered between runs. `black`, `marble`, `splatter`, `split`, `tri`,
  `starburst`, `galaxy`, `smoke`, `picture`, `rainbow`, `gold`, `clear`,
  `glow`, and `omarchy`. The art-based pressings take the album's dominant
  colour first and then the colours most distinct from it, so two-tone patterns
  keep their contrast. Any pattern combines with any palette on the command
  line as `pattern:palette`.
- **Omarchy theme awareness.** The `omarchy` pressing marbles the current
  theme's accent, its most saturated colours and its foreground, and
  re-presses itself when you switch themes.
- **Full-screen view**, from the card's corner button, `--fullscreen`, or a
  right-click on the bar widget. A large record over a blurred, darkened copy
  of the album art. Escape or a background click returns.
- **Drag to move.** The position is remembered per monitor.
- **Workspace and monitor control.** `--workspace all|current|<ids or names>`
  hides the widget when its monitor shows a workspace that is not on the list,
  driven by Hyprland's event socket. `--monitor <name>|current` moves it
  between screens. Both apply to an already running widget.
- **Omarchy bar widget.** A small record in the bar that turns while anything
  is playing. Left click opens or closes the desktop widget, right click opens
  the full-screen view, middle click steps to the next pressing, shift-click
  brings it to the current monitor and workspace, and control-click returns it
  to every workspace.
- **Remote control.** A second `vinyl` talks to the running one: `--toggle`,
  `--fullscreen`, `--next-style`, `--monitor`, `--workspace`, `--quit`.
- `--opaque` for a solid card, and `tools/demo-player.py`, a fake MPRIS player
  for trying the widget without a music app.

## 0.1.0 — 2026-09-09

### Added

- First release. A spinning-record now-playing widget for Omarchy and other
  Hyprland desktops, reading whatever is playing over MPRIS, so any player that
  speaks it works with no accounts and no API keys.
- Album sleeve, record, track title, artist, album, progress and transport
  controls. Click the record to play or pause, the sleeve to raise the player.
- Follows whichever player is playing and hands off between players.
- The record is a custom `gdk::Paintable`: the disc is rendered once with Cairo
  and each frame rotates that texture inside the paintable's own snapshot, so a
  new angle invalidates one picture rather than relaying out the card. About
  3% CPU while spinning on a 144 Hz display.
