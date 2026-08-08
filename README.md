# wlrix-bg

The wlRIX desktop background. Paints the wallpaper, one wlr-layer-shell surface per monitor.

- **Language:** Rust
- **License:** GPL-3.0-or-later
- **Reference:** IRIX 6.5's Desktop Background panel

It replaces `swaybg`, which was the last third-party client in the core wlRIX session. Three things
`swaybg` could not do, and this exists for:

- **The original IRIX artwork.** SGI images (`.rgb`, `.rgba`, `.bw`, `.sgi`) and XPM decode directly, so
  `background.toml` can point at a file off an Indy rather than at a PNG somebody converted.
- **A config file.** `swaybg` takes its wallpaper on argv, once, at process start, which put the setting in
  `autostart.toml` and out of reach of `wlrix-settings-daemon` — changing it meant editing a file and logging out. Here
  it is three keys the daemon writes and signals, and the change is on screen before the panel's button comes back up.
- **Per-monitor wallpapers**, as configuration rather than as a repeated flag.

## Layers

`wlrix-bg` is on the **background** layer. `wlrix-desktop` — the icons — is on **bottom**, one above, with a transparent
surface, so the wallpaper shows through wherever an icon is not drawn. That split is deliberate on both sides: two
clients sharing a layer have no defined order between them, so restarting one could draw it over the other.

The exclusive zone is `-1`, not `0`. A wallpaper covers the whole output *including* underneath a panel's exclusive
zone; zero would shrink the surface when a panel appeared and leave an unpainted strip behind it.

## Settings

`~/.config/wlrix/background.toml`, then `/etc/wlrix/background.toml`. The first file found wins outright and nothing is
merged, as everywhere else in wlRIX.

`just install` writes the second of those from [`data/background.toml.in`](data/background.toml.in) — the **IRIX
scatter**, tiled. That is a two-color dither, 100×100, seamless; `mode = "tile"` rather than the built-in `fill`
because scaling a 100-pixel dither to a screen turns the pattern into mush. The picture itself comes from
`wlrix-assets`, which installs it to `$prefix/share/wlrix/wallpapers/`; the config names it by absolute path, so the
template carries `@WALLPAPERDIR@` and the justfile substitutes the real prefix at install time.

An existing `/etc/wlrix/background.toml` is **never overwritten** by a reinstall — /etc belongs to whoever installed the
machine. And if `wlrix-assets` is not installed, the path simply does not resolve: `wlrix-bg` says so on stderr and
falls back to the color, a plain `#555555` desktop — the palette's `DESKTOP` role, the gray IRIX put under everything.
With no config file anywhere at all, that gray is also what you get.

```toml
image = "/home/you/Pictures/scatter.png"   # absent: the color alone
mode = "tile"                             # tile|center|fit|fill|stretch|solid
color = "#555555"                          # letterbox bars, and the whole screen under solid

# Per-monitor overrides, by connector name. Each field falls back to the top-level one.
[[output]]
name = "DP-2"
image = "/home/you/Pictures/yaeka.png"
mode = "fit"
```

| `mode`    |                                                                           |
|-----------|---------------------------------------------------------------------------|
| `tile`    | Native size, repeated from the top-left. IRIX's patterns are tiles.       |
| `center`  | Native size, centred, color around it; cropped if larger than the screen. |
| `fit`     | Scaled to fit entirely, aspect kept, color in the letterbox bars.         |
| `fill`    | Scaled to cover, aspect kept, overflow cropped evenly. **The default.**   |
| `stretch` | Scaled to the screen exactly, aspect discarded.                           |
| `solid`   | The color alone. The picture is not even opened.                          |

`color` is `#rrggbb` or the shorthand `#rgb`. It is what shows in the bars, through anything transparent in the picture,
and over the whole screen when there is no picture. There is no alpha:
this *is* the background, and there is nothing behind it.

`image = ""` in an `[[output]]` section is how one monitor says "no picture here" while a top-level
`image` is set — which is why an absent `image` and an empty one are different things.

### Coming from swaybg

```toml
# swaybg -i ~/Pictures/scatter.png -m tile
image = "/home/you/Pictures/scatter.png"
mode = "tile"
```

`swaybg`'s `-m fill`, `fit`, `stretch`, `center` and `tile` all carry over under the same names. Its `solid_color` is
`mode = "solid"` with a `color`. Remove the `[[app]]` block that started
`swaybg` from `autostart.toml`: `wlrix-bg` is one of `wlrix-session`'s default apps, so leaving both in place gets two
wallpaper clients fighting over the background layer.

### Through the settings daemon

`background.image`, `background.mode` and `background.color` are described and written by
`wlrix-settings-daemon`, which validates a candidate file through this program's own parser (`wlrix-bg --check-config`)
before committing it and then sends `SIGHUP`.

```bash
busctl --user call com.wlrix.Settings /com/wlrix/Settings com.wlrix.Settings1 \
    Set sv background.mode s tile
```

`[[output]]` is **not** offered over D-Bus and answers `Unsupported`. It is an array-of-table section, and that daemon
does not serve keyed collections yet — the same position `[[app]]` and
`[[timeout]]` are in. Per-monitor overrides are hand-edited for now; the file is watched, so an edit still applies
without a restart.

Reloading by hand is the same signal:

```bash
kill -HUP $(cat "$XDG_RUNTIME_DIR/wlrix-bg.pid")
```

Everything reloads live, including the per-monitor overrides — changing a wallpaper is a buffer swap, not a surface
teardown.

## Formats

Through [`image`](https://crates.io/crates/image): PNG, JPEG, GIF, BMP, TIFF, WebP. Through
[`image-extras`](https://crates.io/crates/image-extras), registered as a decoding hook so they come back through the
same reader: **SGI**, **XPM**, **XBM**, **PCX**.

The format is decided by **content first, extension second**. That matters for the IRIX files in particular: `.rgb` is
used for raw pixel dumps as well as for SGI images, and plenty of the old artwork has no extension worth trusting. XBM
is the exception — it is C source with no signature to sniff — so it is found by its extension.

A file that will not decode costs the picture and nothing else: the screen falls back to `color`, with the reason on
stderr. `image-extras` says outright that its decoders "may panic or worse on malformed input", so decoding runs inside
`catch_unwind` and a panic is treated as an unreadable file. `image::Limits` caps dimensions and allocation from the
header onwards, so a file claiming to be 60000×60000 is refused before anything is allocated.

## What it does not do yet

- **Slideshows and gradients.** One picture per monitor.
- **Fractional scale.** The buffer is allocated at the output's *integer* scale (`wl_surface.set_buffer_scale`). The
  compositor implements `wp_fractional_scale_v1` and
  `wp_viewporter`, so a fractional pass is possible; it is not written.
- **Per-desk wallpapers**, which IRIX had. The seam is deliberate rather than accidental:
  `Config::resolve` takes an output identity and returns one fully-resolved `Wallpaper`, so a desk axis is a second
  argument there and a second key on the surface map, not a rewrite. Waiting on the Desks overview and the `wlrix-desks`
  protocol.

## Building

```bash
cargo build --release
just release && sudo just install
```

Started by `wlrix-session`, first in its `DEFAULT_APPS` so the wallpaper is up before the icons that show through it.
Needs a compositor with `wlr-layer-shell`.
