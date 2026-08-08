// SPDX-License-Identifier: GPL-3.0-or-later
//! Where the picture goes.
//!
//! One function, [`place`], turning a mode and two sizes into a [`Placement`]. It touches no
//! pixels and holds no state, which is the point: the fitting rules are the part of a wallpaper
//! that is easy to get subtly wrong -- an off-by-one in a crop offset, a `fill` that letterboxes
//! on one axis -- and here they can be checked exhaustively without a compositor or a decoder.
//!
//! Sizes are the *surface's* -- physical pixels, after the buffer scale has been applied -- so
//! everything below is integer arithmetic in the buffer's own coordinates.

use crate::config::Mode;

/// A rectangle in buffer coordinates. May extend past the surface: that is what a crop is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl Rect {
    pub fn new(x: i32, y: i32, width: i32, height: i32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }
}

/// What to draw.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placement {
    /// The color over the whole surface, and no picture at all. `solid`, no configured image,
    /// or an image that turned out to have no pixels in it.
    Color,
    /// The picture at its native size, repeated from the top-left corner.
    Tile,
    /// The picture drawn into `Rect`, with the color wherever the rectangle does not reach.
    ///
    /// The rectangle may be larger than the surface and may have negative origin -- that is how
    /// `fill` and an oversized `center` crop. The renderer clips.
    Into(Rect),
}

/// Where `source` goes on a `surface` of this size, under `mode`.
///
/// A source with a zero dimension is [`Placement::Color`]: there is nothing to draw, and every
/// scale factor below would be a division by zero.
pub fn place(mode: Mode, source: (u32, u32), surface: (u32, u32)) -> Placement {
    let (source_w, source_h) = (source.0 as i32, source.1 as i32);
    let (surface_w, surface_h) = (surface.0 as i32, surface.1 as i32);
    if mode == Mode::Solid || source_w <= 0 || source_h <= 0 || surface_w <= 0 || surface_h <= 0 {
        return Placement::Color;
    }

    match mode {
        Mode::Solid => Placement::Color,
        Mode::Tile => Placement::Tile,
        Mode::Stretch => Placement::Into(Rect::new(0, 0, surface_w, surface_h)),
        Mode::Center => Placement::Into(centered(source_w, source_h, surface_w, surface_h)),
        // The only difference between the two is which way the scale is rounded off: `fit` takes
        // the smaller factor and leaves bars, `fill` takes the larger and crops. Everything after
        // that -- rounding, centring -- is identical, which is why they share the code.
        Mode::Fit | Mode::Fill => {
            let horizontal = surface_w as f64 / source_w as f64;
            let vertical = surface_h as f64 / source_h as f64;
            let scale = if mode == Mode::Fit {
                horizontal.min(vertical)
            } else {
                horizontal.max(vertical)
            };
            // Rounded, then floored at one pixel: a source far larger than the surface can scale
            // below half a pixel on one axis, and a zero-width destination is not a picture.
            let width = ((source_w as f64 * scale).round() as i32).max(1);
            let height = ((source_h as f64 * scale).round() as i32).max(1);
            Placement::Into(centered(width, height, surface_w, surface_h))
        }
    }
}

/// A `width` x `height` rectangle in the middle of a `surface_w` x `surface_h` surface.
///
/// Biased so the leftover pixel of an odd difference falls on the right and the bottom, which is
/// where a half-pixel of letterbox is least noticeable and, more usefully, is deterministic.
fn centered(width: i32, height: i32, surface_w: i32, surface_h: i32) -> Rect {
    Rect::new(
        (surface_w - width).div_euclid(2),
        (surface_h - height).div_euclid(2),
        width,
        height,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 16:9 screen, and a 4:3 picture -- the pair that actually shows the difference between
    /// the modes.
    const SCREEN: (u32, u32) = (1920, 1080);
    const PICTURE: (u32, u32) = (800, 600);

    #[test]
    fn solid_draws_no_picture_even_when_there_is_one() {
        assert_eq!(place(Mode::Solid, PICTURE, SCREEN), Placement::Color);
    }

    #[test]
    fn tile_draws_at_native_size() {
        assert_eq!(place(Mode::Tile, (64, 64), SCREEN), Placement::Tile);
        // Including a tile bigger than the screen: the renderer clips, and one repetition of a
        // too-large tile is just a top-left crop.
        assert_eq!(place(Mode::Tile, (4000, 4000), SCREEN), Placement::Tile);
    }

    #[test]
    fn stretch_takes_the_whole_surface_and_loses_the_aspect() {
        assert_eq!(
            place(Mode::Stretch, PICTURE, SCREEN),
            Placement::Into(Rect::new(0, 0, 1920, 1080))
        );
    }

    #[test]
    fn center_keeps_the_native_size_and_centres_it() {
        assert_eq!(
            place(Mode::Center, PICTURE, SCREEN),
            Placement::Into(Rect::new(560, 240, 800, 600))
        );
    }

    #[test]
    fn center_crops_a_picture_larger_than_the_screen() {
        // Negative origin, on both axes: the middle of the picture is shown and the edges fall
        // off. Cropping rather than scaling is the whole meaning of `center`.
        let Placement::Into(rect) = place(Mode::Center, (2560, 1440), SCREEN) else {
            panic!("center should place a rectangle");
        };
        assert_eq!(rect, Rect::new(-320, -180, 2560, 1440));
        // Symmetric: what falls off the left equals what falls off the right.
        assert_eq!(rect.x, SCREEN.0 as i32 - (rect.x + rect.width));
    }

    #[test]
    fn fit_letterboxes_and_never_crops() {
        let Placement::Into(rect) = place(Mode::Fit, PICTURE, SCREEN) else {
            panic!("fit should place a rectangle");
        };
        // 4:3 into 16:9 fits by height: 1080/600 = 1.8, so 1440x1080 with bars either side.
        assert_eq!(rect, Rect::new(240, 0, 1440, 1080));
        // The defining property, which is what these assertions are really for: the whole
        // picture is on screen.
        assert!(rect.x >= 0 && rect.y >= 0);
        assert!(rect.x + rect.width <= SCREEN.0 as i32);
        assert!(rect.y + rect.height <= SCREEN.1 as i32);
    }

    #[test]
    fn fill_covers_and_never_letterboxes() {
        let Placement::Into(rect) = place(Mode::Fill, PICTURE, SCREEN) else {
            panic!("fill should place a rectangle");
        };
        // Covering by width: 1920/800 = 2.4, so 1920x1440 with the top and bottom cropped.
        assert_eq!(rect, Rect::new(0, -180, 1920, 1440));
        // The defining property: no pixel of the surface is left uncovered.
        assert!(rect.x <= 0 && rect.y <= 0);
        assert!(rect.x + rect.width >= SCREEN.0 as i32);
        assert!(rect.y + rect.height >= SCREEN.1 as i32);
    }

    #[test]
    fn fit_and_fill_keep_the_aspect_ratio() {
        // Across a spread of shapes, not just the one above -- this is the property that tells
        // them apart from `stretch`, and a rounding mistake would break it quietly.
        for source in [(800, 600), (1920, 1080), (100, 3000), (3000, 100), (7, 13)] {
            for mode in [Mode::Fit, Mode::Fill] {
                let Placement::Into(rect) = place(mode, source, SCREEN) else {
                    panic!("{mode:?} should place a rectangle");
                };
                let wanted = source.0 as f64 / source.1 as f64;
                let got = rect.width as f64 / rect.height as f64;
                // Loose, because the destination is whole pixels: a 7x13 source scaled to fit
                // 1080 tall cannot land on the exact ratio.
                assert!(
                    (wanted - got).abs() / wanted < 0.02,
                    "{mode:?} {source:?}: wanted {wanted}, got {got} ({rect:?})"
                );
            }
        }
    }

    #[test]
    fn a_picture_the_same_shape_as_the_screen_fits_and_fills_identically() {
        for mode in [Mode::Fit, Mode::Fill] {
            assert_eq!(
                place(mode, (3840, 2160), SCREEN),
                Placement::Into(Rect::new(0, 0, 1920, 1080)),
                "{mode:?}"
            );
        }
    }

    #[test]
    fn a_picture_with_no_pixels_is_just_the_color() {
        // A decoder can hand back a 0-width image, and every scale factor here divides by the
        // source size. This is the guard, not an assertion about taste.
        for source in [(0, 0), (0, 100), (100, 0)] {
            for mode in [
                Mode::Tile,
                Mode::Center,
                Mode::Fit,
                Mode::Fill,
                Mode::Stretch,
            ] {
                assert_eq!(place(mode, source, SCREEN), Placement::Color, "{mode:?}");
            }
        }
    }

    #[test]
    fn a_surface_with_no_pixels_asks_for_nothing() {
        // A layer surface is 0x0 until the compositor configures it. Nothing should be drawn,
        // and nothing should divide by zero on the way to finding that out.
        for surface in [(0, 0), (0, 1080), (1920, 0)] {
            for mode in [
                Mode::Tile,
                Mode::Center,
                Mode::Fit,
                Mode::Fill,
                Mode::Stretch,
            ] {
                assert_eq!(place(mode, PICTURE, surface), Placement::Color, "{mode:?}");
            }
        }
    }

    #[test]
    fn an_enormous_source_still_scales_to_something_drawable() {
        // 32768 wide into a 64px surface is a scale of 0.002; naive truncation gives a
        // zero-width destination, which is a picture nobody can see and a divide-by-zero in the
        // renderer's sampling.
        let Placement::Into(rect) = place(Mode::Fit, (32768, 4), (64, 64)) else {
            panic!("fit should place a rectangle");
        };
        assert!(rect.width >= 1 && rect.height >= 1, "{rect:?}");
    }
}
