// SPDX-License-Identifier: GPL-3.0-or-later
//! The hand-edited settings file.
//!
//! ```toml
//! # ~/.config/wlrix/background.toml
//! image = "/home/you/Pictures/scatter.png"   # absent: the color alone
//! mode  = "tile"                             # tile|center|fit|fill|stretch|solid
//! color = "#555555"                          # letterbox bars, and the whole screen in solid
//!
//! # Per-monitor overrides, by connector name. Each field falls back to the top-level one.
//! [[output]]
//! name  = "DP-2"
//! image = "/home/you/Pictures/yaeka.png"
//! mode  = "fit"
//! ```
//!
//! Read from the user's config directory first, then `/etc/wlrix`; the first file found wins
//! outright rather than merging, so what a user sees in their own file is the whole of what they
//! get. This mirrors the compositor, the desktop, and the session deliberately: one shape of file
//! across the stack.
//!
//! Unknown keys are an error, for the same reason every other wlRIX config rejects them -- a
//! silently ignored typo is a setting that never applies and never says why.
//!
//! ## `[[output]]` and the settings daemon
//!
//! The three top-level keys are scalar leaves, which is what `wlrix-settings-daemon` describes
//! and writes. `[[output]]` is an array-of-table section, which that daemon refuses by design --
//! a keyed collection wants add/remove/reorder rather than get/set, and its collection surface is
//! a documented follow-up. So per-monitor overrides are hand-edited for now, and `Set` on one
//! answers `Unsupported` rather than pretending. The same is true of the session's `[[app]]` and
//! `wlrix-idle`'s `[[timeout]]`.

use std::path::{Path, PathBuf};

use serde::Deserialize;

/// How the picture is fitted to the screen.
///
/// The names are the ones a user types, and `Kind::Enum` in the settings daemon's table lists
/// exactly these -- offering a value serde would reject there would cost the user their file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// Native size, repeated from the top-left. IRIX's patterns are tiles, and it is what
    /// `swaybg -m tile` did.
    Tile,
    /// Native size, centered, with the color around it. Cropped if it is larger than the screen.
    Center,
    /// Scaled to fit entirely, aspect kept, the color in the letterbox bars.
    Fit,
    /// Scaled to cover the screen, aspect kept, the overflow cropped evenly.
    Fill,
    /// Scaled to the screen exactly, aspect discarded.
    Stretch,
    /// The color alone. Any `image` is ignored, and not even decoded.
    Solid,
}

impl Mode {
    /// The name this is written as, for a log line.
    pub fn name(self) -> &'static str {
        match self {
            Self::Tile => "tile",
            Self::Center => "center",
            Self::Fit => "fit",
            Self::Fill => "fill",
            Self::Stretch => "stretch",
            Self::Solid => "solid",
        }
    }
}

/// What an unset `mode` means.
///
/// Cover rather than fit or stretch: it fills the screen without bars and without distorting
/// anybody's photograph, which is the least surprising thing to do with a picture somebody just
/// pointed at.
pub const DEFAULT_MODE: Mode = Mode::Fill;

/// An opaque color, as `0x00rrggbb`.
///
/// No alpha: this is the *background*, and there is nothing behind it to show through.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Color(pub u32);

/// What an unset `color` means: `#555555`.
///
/// The `DESKTOP` role of the wlRIX palette, generated into `wlrix-desktop/src/theme/palette.rs`
/// from `wlrix-assets/palette/`. Written out here rather than generated, because one constant
/// does not earn a `theme/` module of its own; if a second color ever appears, add a `wlrix-bg`
/// block to `wlrix-assets/tools/palettegen/Program.cs` instead of a second hand copy.
pub const DEFAULT_COLOR: Color = Color(0x0055_5555);

impl Color {
    /// The `(red, green, blue)` channels.
    pub fn channels(self) -> (u8, u8, u8) {
        (
            (self.0 >> 16) as u8,
            (self.0 >> 8) as u8,
            (self.0 & 0xff) as u8,
        )
    }

    /// The pixel this is, in the `Xrgb8888` a `wl_shm` buffer wants.
    pub fn pixel(self) -> u32 {
        self.0 & 0x00ff_ffff
    }
}

impl std::str::FromStr for Color {
    type Err = String;

    /// `#rrggbb` or the shorthand `#rgb`, with the `#` optional.
    ///
    /// Deliberately not a color-name table: `"red"` in a wlRIX config would be the only place in
    /// the stack where a color is not written as hex, and the palette people actually want is
    /// IRIX's, which has no CSS names anyway.
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let digits = text.trim().strip_prefix('#').unwrap_or(text.trim());
        let value = u32::from_str_radix(digits, 16)
            .map_err(|_| format!("{text:?} is not a hex color like \"#555555\""))?;
        match digits.len() {
            6 => Ok(Color(value)),
            // #rgb, each digit doubled -- so #abc is #aabbcc, as everywhere else that accepts it.
            3 => {
                let expand = |nibble: u32| nibble * 0x11;
                Ok(Color(
                    (expand((value >> 8) & 0xf) << 16)
                        | (expand((value >> 4) & 0xf) << 8)
                        | expand(value & 0xf),
                ))
            }
            _ => Err(format!(
                "{text:?} is not a hex color like \"#555555\" or \"#555\""
            )),
        }
    }
}

impl<'de> Deserialize<'de> for Color {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

/// `background.toml`, as written.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// The picture, for every monitor without an override of its own. Absent means the color
    /// alone -- which is why this is an `Option` rather than defaulting to an empty path: absent
    /// and empty have to be tellable apart for the override below to be able to say "none".
    #[serde(default)]
    pub image: Option<PathBuf>,
    #[serde(default)]
    pub mode: Option<Mode>,
    #[serde(default)]
    pub color: Option<Color>,
    /// Per-monitor overrides. `rename` because the section is written `[[output]]` and read as a
    /// list, exactly as `wlrix-compositor`'s `[[output]]` is.
    #[serde(default, rename = "output")]
    pub outputs: Vec<OutputConfig>,
}

/// One monitor's overrides. Every field but the name is optional and falls back to the
/// top-level value.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutputConfig {
    /// The connector name, e.g. `DP-1` or `Virtual-1` -- the same key `wlrix-compositor`'s
    /// `[[output]]` and `wlrix-desktop`'s `output` use.
    pub name: String,
    /// This monitor's picture. `image = ""` means *no* picture on this monitor, which is the
    /// only way to say "color only here" while a top-level image is set.
    #[serde(default)]
    pub image: Option<PathBuf>,
    #[serde(default)]
    pub mode: Option<Mode>,
    #[serde(default)]
    pub color: Option<Color>,
}

/// What one monitor is actually going to show, with every fallback already applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wallpaper {
    /// `None` is a bare color, either because nothing was configured or because the mode is
    /// `solid`.
    pub image: Option<PathBuf>,
    pub mode: Mode,
    pub color: Color,
}

impl Config {
    /// What `output` should show.
    ///
    /// `None` for the name means "a monitor we could not get a name for", which takes the
    /// top-level settings -- an override is keyed by name and there is nothing to match.
    ///
    /// **This is the seam for per-desk wallpapers.** IRIX gave each desk its own background, and
    /// `wlrix-compositor` already models desks and exposes them over `wlrix-desks`. When that
    /// arrives this grows a second argument and every caller keeps its shape, because resolving
    /// is already one function over an identity rather than a scatter of reads of the config.
    pub fn resolve(&self, output: Option<&str>) -> Wallpaper {
        let over = output.and_then(|name| self.outputs.iter().find(|entry| entry.name == name));

        let mode = over
            .and_then(|entry| entry.mode)
            .or(self.mode)
            .unwrap_or(DEFAULT_MODE);
        let color = over
            .and_then(|entry| entry.color)
            .or(self.color)
            .unwrap_or(DEFAULT_COLOR);
        let image = match over.map(|entry| &entry.image) {
            // The override says something, including `""` for "none here".
            Some(Some(path)) => Some(path.clone()),
            // The override exists but is silent about the image; take the top-level one.
            Some(None) | None => self.image.clone(),
        };
        // `solid` means the color and nothing else, so the file is never opened. An empty path
        // is how an override says "no picture on this monitor".
        let image = image.filter(|path| mode != Mode::Solid && !path.as_os_str().is_empty());

        Wallpaper { image, mode, color }
    }

    /// Load the first config file that exists. No file at all is not an error -- the default is
    /// a plain `#555555` desktop, which is a perfectly good background.
    pub fn load() -> Self {
        for path in crate::xdg::config_paths() {
            let text = match std::fs::read_to_string(&path) {
                Ok(text) => text,
                // Not-found is the ordinary case; only real errors are worth a line.
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => continue,
                Err(err) => {
                    eprintln!("wlrix-bg: could not read {}: {err}", path.display());
                    continue;
                }
            };
            match toml::from_str::<Self>(&text) {
                Ok(config) => return config,
                Err(err) => {
                    // Loud, then carry on with defaults: a broken config should cost the picture,
                    // not leave the screen unpainted.
                    eprintln!("wlrix-bg: {} is not valid: {err}", path.display());
                    return Self::default();
                }
            }
        }
        Self::default()
    }
}

/// Parse a candidate config file, for `--check-config`.
///
/// This program's own serde types are the authority on what `background.toml` may contain.
/// `wlrix-settings-daemon` writes a temporary file and runs this against it before renaming it
/// into place, so a settings app cannot produce a file this program would refuse -- which matters
/// because `deny_unknown_fields` means one wrong key costs the *whole* file and the user silently
/// gets built-in defaults for all of it.
///
/// Deliberately not [`Config::load`]: that reports to stderr and carries on with defaults, which
/// is right at startup -- a typo should cost the wallpaper, not the session -- and exactly wrong
/// here, where the question *is* whether the file is acceptable.
pub fn check(path: &Path) -> Result<(), String> {
    let text = std::fs::read_to_string(path)
        .map_err(|err| format!("could not read {}: {err}", path.display()))?;
    toml::from_str::<Config>(&text)
        .map(|_| ())
        .map_err(|err| err.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_file_is_a_plain_gray_desktop() {
        let config: Config = toml::from_str("").expect("empty config should parse");
        let wallpaper = config.resolve(None);
        assert_eq!(wallpaper.image, None);
        assert_eq!(wallpaper.mode, DEFAULT_MODE);
        assert_eq!(wallpaper.color, DEFAULT_COLOR);
        // The IRIX desktop gray, and the same value `wlrix-desktop` paints under its icons.
        assert_eq!(DEFAULT_COLOR.channels(), (0x55, 0x55, 0x55));
    }

    #[test]
    fn a_typo_is_refused_rather_than_ignored() {
        // `color` is the one that will actually happen, given the prose in this repo spells it
        // that way. It must fail loudly rather than leaving the user's color silently unset.
        assert!(toml::from_str::<Config>("colour = \"#123456\"\n").is_err());
        assert!(toml::from_str::<Config>("mode = \"tiled\"\n").is_err());
        assert!(toml::from_str::<Config>("[[output]]\nnmae = \"DP-1\"\n").is_err());
    }

    #[test]
    fn the_installed_default_config_is_one_this_program_accepts() {
        // `data/background.toml.in` is what `just install` writes to /etc/wlrix/background.toml,
        // and it is the whole of what a fresh machine's desktop looks like. It goes through the
        // same parser as anything else -- `deny_unknown_fields` and all -- so a typo in it would
        // cost every user on that machine their default background silently, at which point they
        // get built-in defaults and a plain gray screen with no clue why.
        //
        // Read at test time rather than `include_str!`d, so the template is checked as a file on
        // disk rather than as something the compiler happened to inline.
        let template = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("data/background.toml.in"),
        )
        .expect("data/background.toml.in should be readable");

        // The one substitution `just install` makes. Anything else left in it is a token the
        // justfile does not know to replace, which would land in /etc verbatim.
        let installed = template.replace("@WALLPAPERDIR@", "/usr/share/wlrix/wallpapers");
        assert!(
            !installed.contains('@'),
            "an unsubstituted @TOKEN@ is left in background.toml.in"
        );

        let config: Config = toml::from_str(&installed).expect("the installed default must parse");
        let wallpaper = config.resolve(None);
        assert_eq!(
            wallpaper.image.as_deref(),
            Some(Path::new("/usr/share/wlrix/wallpapers/scatter.png")),
            "the default should be the IRIX scatter, installed by wlrix-assets"
        );
        // Tile, not the built-in `fill`: the scatter is a 100x100 seamless dither, and scaling it
        // to a screen would turn the pattern into mush.
        assert_eq!(wallpaper.mode, Mode::Tile);
        assert_eq!(wallpaper.color, DEFAULT_COLOR);
    }

    #[test]
    fn every_mode_a_user_can_type_parses() {
        for (text, mode) in [
            ("tile", Mode::Tile),
            ("center", Mode::Center),
            ("fit", Mode::Fit),
            ("fill", Mode::Fill),
            ("stretch", Mode::Stretch),
            ("solid", Mode::Solid),
        ] {
            let config: Config = toml::from_str(&format!("mode = {text:?}\n")).expect(text);
            assert_eq!(config.mode, Some(mode), "{text}");
            // ...and the name round-trips, which is what the settings daemon's enum choices and
            // this program's log lines both rely on.
            assert_eq!(mode.name(), text);
        }
        // Not `Tiled`, `Center`, or a capitalized form: serde is `rename_all = "lowercase"`, and
        // offering any of these in a settings panel would cost the user their file.
        for text in ["Tile", "tiled", "centre", "cover", "zoom", ""] {
            assert!(
                toml::from_str::<Config>(&format!("mode = {text:?}\n")).is_err(),
                "{text:?} should not be a mode"
            );
        }
    }

    #[test]
    fn colors_are_hex_with_or_without_the_hash() {
        for (text, expected) in [
            ("#555555", 0x0055_5555),
            ("555555", 0x0055_5555),
            ("#ffffff", 0x00ff_ffff),
            ("#000000", 0x0000_0000),
            // The shorthand, each digit doubled.
            ("#abc", 0x00aa_bbcc),
            ("#f00", 0x00ff_0000),
        ] {
            assert_eq!(text.parse::<Color>(), Ok(Color(expected)), "{text}");
        }
    }

    #[test]
    fn nonsense_colors_are_refused() {
        // Including an eight-digit one: there is nothing behind the background, so an alpha
        // channel here would be a value that cannot mean anything.
        for text in ["", "#", "#12345", "#12345678", "red", "#gggggg", "0x555555"] {
            assert!(text.parse::<Color>().is_err(), "{text:?} should be refused");
        }
    }

    #[test]
    fn a_color_becomes_an_opaque_pixel() {
        assert_eq!(Color(0x0012_3456).pixel(), 0x0012_3456);
        assert_eq!(Color(0x0012_3456).channels(), (0x12, 0x34, 0x56));
    }

    #[test]
    fn an_override_wins_field_by_field() {
        let config: Config = toml::from_str(
            "image = \"/pictures/scatter.png\"\n\
             mode = \"tile\"\n\
             color = \"#555555\"\n\
             [[output]]\n\
             name = \"DP-2\"\n\
             image = \"/pictures/yaeka.png\"\n\
             mode = \"fit\"\n",
        )
        .unwrap();

        // The overridden monitor takes its own image and mode, and inherits the color it did
        // not mention -- which is the whole point of resolving per field rather than per section.
        let there = config.resolve(Some("DP-2"));
        assert_eq!(
            there.image.as_deref(),
            Some(Path::new("/pictures/yaeka.png"))
        );
        assert_eq!(there.mode, Mode::Fit);
        assert_eq!(there.color, Color(0x0055_5555));

        // Every other monitor, and a monitor whose name we never learned, take the top level.
        for other in [Some("DP-1"), Some("HDMI-A-1"), None] {
            let wallpaper = config.resolve(other);
            assert_eq!(
                wallpaper.image.as_deref(),
                Some(Path::new("/pictures/scatter.png")),
                "{other:?}"
            );
            assert_eq!(wallpaper.mode, Mode::Tile, "{other:?}");
        }
    }

    #[test]
    fn an_empty_image_is_how_one_monitor_says_no_picture() {
        // The case that needs `Option<PathBuf>` rather than a plain default: with a top-level
        // image set, there has to be a way for one monitor to opt out and show only a color.
        let config: Config = toml::from_str(
            "image = \"/pictures/scatter.png\"\n\
             [[output]]\n\
             name = \"DP-2\"\n\
             image = \"\"\n\
             color = \"#000000\"\n",
        )
        .unwrap();
        let there = config.resolve(Some("DP-2"));
        assert_eq!(there.image, None);
        assert_eq!(there.color, Color(0));
        // ...and it has not disturbed anyone else.
        assert!(config.resolve(Some("DP-1")).image.is_some());
    }

    #[test]
    fn solid_never_opens_the_file() {
        // Not just "does not draw it" -- the resolved wallpaper carries no path at all, so the
        // decoder is never asked and a picture that has since been deleted costs nothing.
        let config: Config =
            toml::from_str("image = \"/pictures/scatter.png\"\nmode = \"solid\"\n").unwrap();
        let wallpaper = config.resolve(None);
        assert_eq!(wallpaper.image, None);
        assert_eq!(wallpaper.mode, Mode::Solid);
    }

    #[test]
    fn an_override_can_switch_one_monitor_to_solid() {
        let config: Config = toml::from_str(
            "image = \"/pictures/scatter.png\"\n\
             [[output]]\n\
             name = \"DP-2\"\n\
             mode = \"solid\"\n",
        )
        .unwrap();
        assert_eq!(config.resolve(Some("DP-2")).image, None);
        assert!(config.resolve(Some("DP-1")).image.is_some());
    }

    #[test]
    fn a_full_file_round_trips() {
        // Every key this program accepts, in one file -- so a key added without a test here
        // still has to survive being written down.
        let config: Config = toml::from_str(
            "image = \"/pictures/scatter.png\"\n\
             mode = \"tile\"\n\
             color = \"#555555\"\n\
             \n\
             [[output]]\n\
             name = \"DP-1\"\n\
             image = \"/pictures/one.sgi\"\n\
             mode = \"center\"\n\
             color = \"#000\"\n\
             \n\
             [[output]]\n\
             name = \"HDMI-A-1\"\n\
             mode = \"stretch\"\n",
        )
        .unwrap();
        assert_eq!(config.outputs.len(), 2);
        assert_eq!(
            config.resolve(Some("DP-1")),
            Wallpaper {
                image: Some(PathBuf::from("/pictures/one.sgi")),
                mode: Mode::Center,
                color: Color(0),
            }
        );
        assert_eq!(
            config.resolve(Some("HDMI-A-1")),
            Wallpaper {
                image: Some(PathBuf::from("/pictures/scatter.png")),
                mode: Mode::Stretch,
                color: Color(0x0055_5555),
            }
        );
    }

    /// The `background.toml` block of `wlrix-settings-daemon --dump-schema`, verbatim.
    ///
    /// Pasted here on purpose. That daemon's schema table is a hand-kept copy of the serde types
    /// in this file, and a hand-kept copy drifts -- and because of `deny_unknown_fields` a drifted
    /// key does not produce a wrong setting, it makes this program reject the *whole file* and
    /// fall back to built-in defaults. Which is the user's entire wallpaper configuration gone.
    ///
    /// The daemon's README asks each component to hold this fixture for exactly that reason: it
    /// turns the drift into a failing test in the repo that defines the truth.
    const SCHEMA_DUMP: &str = "\
# Wallpaper
# image =    # no default; unset by default
# How the wallpaper is fitted
mode = \"fill\"
# Desktop color
color = \"#555555\"
";

    #[test]
    fn the_settings_daemon_describes_a_file_this_program_accepts() {
        let config: Config = toml::from_str(SCHEMA_DUMP)
            .expect("the daemon's declared defaults must be a file this program accepts");
        // ...and its declared defaults must be *this program's* defaults, or the daemon would
        // report a value the background is not actually showing.
        assert_eq!(config.mode, Some(DEFAULT_MODE));
        assert_eq!(config.color, Some(DEFAULT_COLOR));
        // `image` has no default and is emitted commented out, which is the daemon saying that
        // absent and empty are different things here. They are: see `resolve`.
        assert_eq!(config.image, None);
    }

    #[test]
    fn every_mode_the_daemon_offers_is_one_this_program_accepts() {
        // The `Kind::Enum` choices in `wlrix-settings-daemon/src/schema/table.rs`, in its order.
        // A choice this program rejects would be a dropdown entry that costs the user their file
        // the moment they pick it.
        for value in ["fill", "fit", "stretch", "center", "tile", "solid"] {
            assert!(
                toml::from_str::<Config>(&format!("mode = {value:?}\n")).is_ok(),
                "the daemon offers {value:?}"
            );
        }
    }

    #[test]
    fn checking_a_file_answers_rather_than_carrying_on() {
        let dir = std::env::temp_dir().join("wlrix-bg-config-check");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let good = dir.join("good.toml");
        std::fs::write(&good, "mode = \"fit\"\ncolor = \"#123456\"\n").unwrap();
        assert!(check(&good).is_ok());

        let bad = dir.join("bad.toml");
        std::fs::write(&bad, "mode = \"tiled\"\n").unwrap();
        assert!(check(&bad).is_err());

        // A file that is not there is a refusal, not a silent pass: the daemon hands us a path
        // it has just written, and being unable to read it means something is wrong.
        assert!(check(&dir.join("absent.toml")).is_err());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
