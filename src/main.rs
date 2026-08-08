// SPDX-License-Identifier: GPL-3.0-or-later
//! The wlRIX desktop background.
//!
//! Paints the wallpaper on a wlr-layer-shell **background** surface, one per monitor -- below the
//! desktop icons, which sit on the layer above so this one shows through them. Decodes the formats
//! IRIX shipped its own backgrounds in (SGI `.rgb`, XPM) as well as the modern ones.
//!
//! Started by `wlrix-session`. Settings live in `~/.config/wlrix/background.toml` and are written
//! by `wlrix-settings-daemon`, which signals this process to re-read them.

fn main() {
    // No option here combines with another, so only the first is looked at -- a loop would read as
    // though `wlrix-bg --version --help` meant something.
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        None => {}
        Some("--help" | "-h") => {
            println!(
                "wlrix-bg {}\n\n\
                     The desktop background for wlRIX. Started by wlrix-session; needs a running\n\
                     compositor with wlr-layer-shell.\n\n\
                     Usage: wlrix-bg [options]\n\n\
                     Options:\n  \
                       --check-config <path>  say whether that file would be accepted, exit\n  \
                       -h, --help     this message\n  \
                       -V, --version  print the version\n\n\
                     Settings live in ~/.config/wlrix/background.toml:\n\n  \
                       image = \"/path/to/picture.png\"\n  \
                       mode  = \"fill\"   # tile, center, fit, fill, stretch, solid\n  \
                       color = \"#555555\"\n\n\
                     Per-monitor overrides go in [[output]] sections keyed by connector name.\n\
                     Send SIGHUP to re-read the file.",
                env!("CARGO_PKG_VERSION")
            );
            return;
        }
        Some("--version" | "-V") => {
            println!("wlrix-bg {}", env!("CARGO_PKG_VERSION"));
            return;
        }
        // Answers a question about a file rather than doing anything with it, so it needs no
        // compositor and starts nothing. `wlrix-settings-daemon` runs this against a candidate file
        // before renaming it into place, which is what stops a settings app from writing a
        // `background.toml` this program would refuse -- and with `deny_unknown_fields`, one wrong
        // key costs the whole file.
        Some("--check-config") => {
            let Some(path) = args.get(1) else {
                eprintln!("wlrix-bg: --check-config needs a path");
                std::process::exit(2);
            };
            if let Err(why) = wlrix_bg::config::check(std::path::Path::new(path)) {
                eprintln!("{why}");
                std::process::exit(1);
            }
            return;
        }
        Some(other) => {
            eprintln!("wlrix-bg: unknown argument: {other}");
            std::process::exit(2);
        }
    }

    if let Err(err) = wlrix_bg::ui::run() {
        eprintln!("wlrix-bg: {err}");
        std::process::exit(1);
    }
}
