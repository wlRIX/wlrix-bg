// SPDX-License-Identifier: GPL-3.0-or-later
//! Turning a wallpaper file into pixels.
//!
//! Everything goes through `image`, with `image-extras` registered on top of it. That registration
//! is a hook into `image`'s own reader rather than a second entry point, so after [`Pictures::new`]
//! an SGI file and a PNG are both just `ImageReader::open(..).decode()` and this module has no
//! per-format code at all.
//!
//! ## Why the extra formats
//!
//! IRIX shipped its backgrounds as **SGI** images -- `.rgb`, `.rgba`, `.bw`, and occasionally
//! `.sgi` or no extension -- and the 4Dwm-era patterns are **XPM**. Being able to point
//! `background.toml` straight at the original artwork, rather than at a PNG somebody converted, is
//! most of the reason this program exists instead of a `swaybg` line.
//!
//! The format is decided by **content first, extension second**
//! (`ImageReader::with_guessed_format`, which replaces the extension's guess when the signature
//! says otherwise and leaves it alone when it cannot tell). That ordering matters here more than
//! for most decoders: `.rgb` is used for raw pixel dumps as well as for SGI images, and plenty of
//! IRIX-era files have no extension worth trusting. XBM is the one format with no signature to
//! sniff -- it is C source with no fixed preamble -- so it is found by its extension alone, which
//! is why the extension guess is kept rather than overwritten.
//!
//! ## Two rules
//!
//! **A bad file must cost the wallpaper, not the session.** `image-extras` says outright that
//! "fuzzing is not a priority for this crate and decoders may panic or worse on malformed input",
//! and a wallpaper path is whatever the user wrote in a config file. If a panic here took the
//! process down, the failure mode would be a session with no background and no way to fix it
//! without logging out -- so decoding runs inside [`std::panic::catch_unwind`] and a panic is
//! reported and turned into the plain color, exactly like a file that would not open.
//!
//! `image::Limits` does the rest of the work: it is enforced *inside* the decoder, from the header
//! onwards, so a file claiming to be 60000x60000 is refused before anything is allocated rather
//! than after.
//!
//! **Decode once per file, not once per monitor.** Two screens showing the same wallpaper share
//! one decode, and a file that failed is remembered as having failed rather than being retried on
//! every configure.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use image::RgbaImage;

/// The largest picture that will be decoded, per axis.
///
/// Comfortably past any real wallpaper -- an 8K screen is 7680 wide -- and small enough that a
/// header claiming something absurd is refused from the header rather than from a failed
/// allocation. Strict, in `image`'s sense: a decoder that cannot promise to honor it must fail.
const MAX_DIMENSION: u32 = 16_384;

/// The most a decode may allocate.
///
/// `MAX_DIMENSION` squared in RGBA would be a gigabyte, so this is the binding limit for a
/// picture with an outlandish aspect ratio. `image`'s own default is the same number; it is
/// written out here so that changing it is a decision rather than an upgrade.
const MAX_ALLOC: u64 = 512 * 1024 * 1024;

/// Wallpapers already decoded, keyed by the file they came from.
pub struct Pictures {
    /// `None` records a file that failed, so a broken or missing wallpaper is not re-opened on
    /// every configure event.
    cache: HashMap<PathBuf, Option<RgbaImage>>,
}

impl Default for Pictures {
    fn default() -> Self {
        Self::new()
    }
}

impl Pictures {
    /// Register the extra formats and start with an empty cache.
    ///
    /// `image_extras::register` is idempotent -- it is a `Once` internally -- so more than one of
    /// these is harmless, but there is only ever one.
    pub fn new() -> Self {
        image_extras::register();
        Self {
            cache: HashMap::new(),
        }
    }

    /// The picture in `path`, decoded, or `None` if it could not be read.
    ///
    /// Cached both ways: a success is not decoded twice, and a failure is not retried.
    pub fn load(&mut self, path: &Path) -> Option<&RgbaImage> {
        if !self.cache.contains_key(path) {
            let decoded = match decode(path) {
                Ok(picture) => {
                    eprintln!(
                        "wlrix-bg: loaded {} ({}x{})",
                        path.display(),
                        picture.width(),
                        picture.height()
                    );
                    Some(picture)
                }
                Err(why) => {
                    eprintln!("wlrix-bg: {why}");
                    None
                }
            };
            self.cache.insert(path.to_path_buf(), decoded);
        }
        self.cache.get(path)?.as_ref()
    }

    /// Forget everything. Called on reload, so a wallpaper edited in place is picked up rather
    /// than served from the cache under its unchanged name.
    pub fn clear(&mut self) {
        self.cache.clear();
    }

    /// How many files are remembered, successes and failures alike.
    #[cfg(test)]
    fn len(&self) -> usize {
        self.cache.len()
    }
}

/// Decode one file to straight (non-premultiplied) RGBA.
///
/// The `catch_unwind` is the point; see the module docs. The default panic hook still prints its
/// usual message first, which is worth keeping -- a panic in a decoder is an upstream bug, and the
/// backtrace in the session log is what somebody would report it with.
fn decode(path: &Path) -> Result<RgbaImage, String> {
    let attempt = std::panic::catch_unwind(|| decode_inner(path));
    match attempt {
        Ok(result) => result,
        Err(_) => Err(format!(
            "the decoder panicked on {} -- treating it as unreadable. \
             This is a bug in the image decoder, not in the file's owner.",
            path.display()
        )),
    }
}

fn decode_inner(path: &Path) -> Result<RgbaImage, String> {
    let mut reader = image::ImageReader::open(path)
        .map_err(|err| format!("could not open {}: {err}", path.display()))?
        // Content first, extension second. See the module docs.
        .with_guessed_format()
        .map_err(|err| format!("could not read {}: {err}", path.display()))?;

    // Built from the default rather than with a struct literal: `Limits` is `#[non_exhaustive]`,
    // so a limit added upstream should arrive at its default here rather than failing to compile
    // or, worse, being silently reset to "no limit".
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_DIMENSION);
    limits.max_image_height = Some(MAX_DIMENSION);
    limits.max_alloc = Some(MAX_ALLOC);
    reader.limits(limits);

    let decoded = reader
        .decode()
        .map_err(|err| format!("could not decode {}: {err}", path.display()))?;

    // Straight RGBA, not premultiplied: the buffer this ends up in is `Xrgb8888` and opaque, so
    // the alpha is composited against the configured color here rather than carried into the
    // compositor. See `crate::render`.
    Ok(decoded.into_rgba8())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(test: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("wlrix-bg-decode-{test}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("make the scratch directory");
        dir
    }

    /// A 2x2 PNG: red on top, blue underneath.
    fn png_bytes() -> Vec<u8> {
        let mut picture = RgbaImage::new(2, 2);
        for x in 0..2 {
            picture.put_pixel(x, 0, image::Rgba([255, 0, 0, 255]));
            picture.put_pixel(x, 1, image::Rgba([0, 0, 255, 255]));
        }
        let mut out = std::io::Cursor::new(Vec::new());
        picture
            .write_to(&mut out, image::ImageFormat::Png)
            .expect("encode");
        out.into_inner()
    }

    /// The same 2x2 picture as an uncompressed SGI image.
    ///
    /// Hand-built, because `image-extras` only decodes -- there is no encoder to round-trip
    /// through, and a checked-in binary fixture would be a file nobody can review in a diff.
    /// 512-byte header, then one plane per channel, each stored **bottom row first**.
    fn sgi_bytes() -> Vec<u8> {
        let mut out = vec![0u8; 512];
        out[0..2].copy_from_slice(&[0x01, 0xda]); // magic
        out[2] = 0; // storage: verbatim, not RLE
        out[3] = 1; // one byte per channel
        out[4..6].copy_from_slice(&3u16.to_be_bytes()); // dimension 3: x, y and channels
        out[6..8].copy_from_slice(&2u16.to_be_bytes()); // xsize
        out[8..10].copy_from_slice(&2u16.to_be_bytes()); // ysize
        out[10..12].copy_from_slice(&3u16.to_be_bytes()); // zsize: RGB
        // The name field at 24..104 and the colormap at 104..108 are already zero, which is a
        // valid empty name and colormap NORMAL.

        // Bottom row first, within each plane: red is (255,0,0) on top, blue (0,0,255) below.
        out.extend_from_slice(&[0, 0, 255, 255]); // red plane:   blue row, then red row
        out.extend_from_slice(&[0, 0, 0, 0]); // green plane: neither
        out.extend_from_slice(&[255, 255, 0, 0]); // blue plane:  blue row, then red row
        out
    }

    /// The same 2x2 picture as XPM, which is C source.
    fn xpm_bytes() -> Vec<u8> {
        // Strict decoder: the trailing comma on every string and the final semicolon are both
        // required, and the crate's own tests say so.
        br#"/* XPM */
static char *wlrix[] = {
"2 2 2 1",
"r c #FF0000",
"b c #0000FF",
"rr",
"bb",
};
"#
        .to_vec()
    }

    fn assert_red_over_blue(picture: &RgbaImage, what: &str) {
        assert_eq!(picture.dimensions(), (2, 2), "{what}");
        assert_eq!(
            picture.get_pixel(0, 0),
            &image::Rgba([255, 0, 0, 255]),
            "{what} top-left"
        );
        assert_eq!(
            picture.get_pixel(1, 0),
            &image::Rgba([255, 0, 0, 255]),
            "{what} top-right"
        );
        assert_eq!(
            picture.get_pixel(0, 1),
            &image::Rgba([0, 0, 255, 255]),
            "{what} bottom-left"
        );
        assert_eq!(
            picture.get_pixel(1, 1),
            &image::Rgba([0, 0, 255, 255]),
            "{what} bottom-right"
        );
    }

    #[test]
    fn the_irix_formats_decode() {
        // The whole reason `image-extras` is a dependency. If SGI or XPM ever stops decoding,
        // pointing `background.toml` at the original IRIX artwork silently stops working, which
        // is the one thing this program does that `swaybg` could not.
        let dir = scratch("formats");
        let mut pictures = Pictures::new();

        for (name, bytes) in [
            ("wallpaper.png", png_bytes()),
            ("wallpaper.rgb", sgi_bytes()),
            ("wallpaper.xpm", xpm_bytes()),
        ] {
            let path = dir.join(name);
            std::fs::write(&path, &bytes).unwrap();
            let picture = pictures
                .load(&path)
                .unwrap_or_else(|| panic!("{name} should decode"));
            assert_red_over_blue(picture, name);
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_content_decides_the_format_not_the_extension() {
        // IRIX-era files are routinely misnamed, and `.rgb` in particular is used for raw pixel
        // dumps as well as for SGI images. An SGI file called `.png` must still decode.
        let dir = scratch("sniffing");
        let mut pictures = Pictures::new();

        for name in ["backdrop.png", "backdrop.jpg", "backdrop"] {
            let path = dir.join(name);
            std::fs::write(&path, sgi_bytes()).unwrap();
            let picture = pictures
                .load(&path)
                .unwrap_or_else(|| panic!("an SGI image named {name} should decode"));
            assert_red_over_blue(picture, name);
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_file_that_is_not_a_picture_is_refused_rather_than_fatal() {
        let dir = scratch("nonsense");
        let mut pictures = Pictures::new();

        for (name, bytes) in [
            ("empty", Vec::new()),
            ("prose.png", b"this is not a picture at all".to_vec()),
            // A truncated SGI: a valid header promising pixels that are not there. This is the
            // shape of input the decoders are least careful with.
            ("truncated.rgb", sgi_bytes()[..520].to_vec()),
        ] {
            let path = dir.join(name);
            std::fs::write(&path, &bytes).unwrap();
            assert!(pictures.load(&path).is_none(), "{name} should not decode");
        }
        // ...and a file that is not there at all.
        assert!(pictures.load(&dir.join("absent.png")).is_none());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_absurd_header_is_refused_from_the_header() {
        // 60000x60000 RGBA is 14GB. The limits are enforced inside the decoder, so this must fail
        // without ever allocating -- if this test hangs or is killed, the limits are not applied.
        const CLAIMED: u16 = 60_000;
        const { assert!(CLAIMED as u32 > MAX_DIMENSION) };

        let dir = scratch("limits");
        let mut header = sgi_bytes();
        header[6..8].copy_from_slice(&CLAIMED.to_be_bytes());
        header[8..10].copy_from_slice(&CLAIMED.to_be_bytes());
        let path = dir.join("enormous.rgb");
        std::fs::write(&path, &header).unwrap();

        let mut pictures = Pictures::new();
        assert!(pictures.load(&path).is_none());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_file_is_decoded_once_and_a_failure_is_not_retried() {
        // Two monitors showing the same wallpaper is the ordinary case, and a failure retried on
        // every configure event would re-open a missing file forever.
        let dir = scratch("cache");
        let good = dir.join("good.png");
        std::fs::write(&good, png_bytes()).unwrap();
        let missing = dir.join("missing.png");

        let mut pictures = Pictures::new();
        assert!(pictures.load(&good).is_some());
        assert!(pictures.load(&good).is_some());
        assert!(pictures.load(&missing).is_none());
        assert!(pictures.load(&missing).is_none());
        assert_eq!(pictures.len(), 2, "one entry each, success and failure");

        // A reload has to forget both: the file may have been edited in place under the same name.
        pictures.clear();
        assert_eq!(pictures.len(), 0);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn transparency_survives_the_decode() {
        // The alpha is composited against the configured color by the renderer, not here, so it
        // has to still be there when this hands the picture over.
        let dir = scratch("alpha");
        let mut picture = RgbaImage::new(1, 1);
        picture.put_pixel(0, 0, image::Rgba([255, 0, 0, 128]));
        let path = dir.join("half.png");
        let mut out = std::io::Cursor::new(Vec::new());
        picture.write_to(&mut out, image::ImageFormat::Png).unwrap();
        std::fs::write(&path, out.into_inner()).unwrap();

        let mut pictures = Pictures::new();
        let loaded = pictures.load(&path).expect("should decode");
        assert_eq!(loaded.get_pixel(0, 0), &image::Rgba([255, 0, 0, 128]));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
