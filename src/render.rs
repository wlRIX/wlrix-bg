// SPDX-License-Identifier: GPL-3.0-or-later
//! Painting one output's buffer.
//!
//! The buffer is `Xrgb8888`: four bytes per pixel, the top one ignored. A wallpaper is opaque by
//! definition -- there is nothing behind it -- so there is no alpha to carry, and the surface
//! declares itself opaque so the compositor can skip everything underneath. Any transparency in
//! the *picture* is composited against the configured color here, which is the only place it can
//! mean anything.
//!
//! Scaling is `image::imageops::resize` with Lanczos3. This runs once per output per config
//! change -- a wallpaper does not animate -- so a slow, good filter is the right trade; a
//! nearest-neighbor wallpaper looks like a mistake, and this one is on screen all day.

use image::RgbaImage;
use image::imageops::FilterType;

use crate::config::{Color, Wallpaper};
use crate::geometry::{Placement, Rect, place};

/// The filter used for `fit`, `fill` and `stretch`.
///
/// Lanczos3 rather than Triangle: scaling a 1920-wide photograph up to 3840 is exactly where the
/// difference is visible, and the cost is paid once.
const FILTER: FilterType = FilterType::Lanczos3;

/// Paint `wallpaper` into an `Xrgb8888` buffer of `width` x `height`.
///
/// `picture` is the decoded image, or `None` when there is none or it could not be read -- in
/// which case this is a plain fill, which is the fallback for every failure in this program.
///
/// `pixels` must hold at least `width * height * 4` bytes.
pub fn render(
    pixels: &mut [u8],
    width: i32,
    height: i32,
    wallpaper: &Wallpaper,
    picture: Option<&RgbaImage>,
) {
    if width <= 0 || height <= 0 {
        return;
    }
    fill(pixels, wallpaper.color);

    let Some(picture) = picture else {
        return;
    };
    let source = picture.dimensions();

    match place(wallpaper.mode, source, (width as u32, height as u32)) {
        Placement::Color => {}
        Placement::Tile => tile(pixels, width, height, picture, wallpaper.color),
        Placement::Into(rect) => {
            // `center` places the picture at its native size, so resampling it would be a
            // needless pass over every pixel *and* would soften artwork that was meant to land
            // one-to-one -- which matters for the IRIX patterns in particular.
            if rect.width == source.0 as i32 && rect.height == source.1 as i32 {
                blit(pixels, width, height, picture, rect, wallpaper.color);
            } else {
                let scaled = image::imageops::resize(
                    picture,
                    rect.width.max(1) as u32,
                    rect.height.max(1) as u32,
                    FILTER,
                );
                blit(pixels, width, height, &scaled, rect, wallpaper.color);
            }
        }
    }
}

/// Paint every pixel one color.
fn fill(pixels: &mut [u8], color: Color) {
    let bytes = color.pixel().to_ne_bytes();
    for pixel in pixels.chunks_exact_mut(4) {
        pixel.copy_from_slice(&bytes);
    }
}

/// Draw `picture` with its top-left at `rect`, clipped to the buffer.
///
/// `rect.width`/`height` are the picture's own size here: any scaling has already happened. A
/// rectangle that starts off the top-left or runs off the bottom-right is normal -- that is what
/// `fill` and an oversized `center` do -- so the loop runs over the *intersection* rather than
/// over the picture.
fn blit(pixels: &mut [u8], width: i32, height: i32, picture: &RgbaImage, rect: Rect, color: Color) {
    let start_x = rect.x.max(0);
    let start_y = rect.y.max(0);
    let end_x = (rect.x + rect.width).min(width);
    let end_y = (rect.y + rect.height).min(height);

    for y in start_y..end_y {
        for x in start_x..end_x {
            let pixel = picture.get_pixel((x - rect.x) as u32, (y - rect.y) as u32);
            put(pixels, width, x, y, over(pixel.0, color));
        }
    }
}

/// Draw `picture` at its native size, repeated from the top-left corner.
///
/// The 1x1 case is special-cased: a one-pixel tile is a solid color, and looping over every
/// pixel of a 4K screen to work that out would be several million redundant composites. It is not
/// a hypothetical -- a single-pixel PNG is exactly how somebody sets a flat color without
/// discovering `mode = "solid"`.
fn tile(pixels: &mut [u8], width: i32, height: i32, picture: &RgbaImage, color: Color) {
    let (tile_w, tile_h) = (picture.width() as i32, picture.height() as i32);
    if tile_w <= 0 || tile_h <= 0 {
        return;
    }
    if tile_w == 1 && tile_h == 1 {
        let single = over(picture.get_pixel(0, 0).0, color);
        let bytes = single.to_ne_bytes();
        for pixel in pixels.chunks_exact_mut(4) {
            pixel.copy_from_slice(&bytes);
        }
        return;
    }

    for y in 0..height {
        let source_y = y.rem_euclid(tile_h) as u32;
        for x in 0..width {
            let source_x = x.rem_euclid(tile_w) as u32;
            let pixel = picture.get_pixel(source_x, source_y);
            put(pixels, width, x, y, over(pixel.0, color));
        }
    }
}

/// One straight-alpha RGBA pixel composited over `color`, as an opaque `Xrgb8888` pixel.
///
/// The two ends are the ones that matter and both are exact: a fully opaque pixel is itself, and
/// a fully transparent one is the color untouched.
fn over(rgba: [u8; 4], color: Color) -> u32 {
    let alpha = u32::from(rgba[3]);
    if alpha == 255 {
        return (u32::from(rgba[0]) << 16) | (u32::from(rgba[1]) << 8) | u32::from(rgba[2]);
    }
    let (br, bg, bb) = color.channels();
    // Rounded, so 50% over an equal color comes back as that color rather than one below it.
    let mix = |source: u8, behind: u8| {
        ((u32::from(source) * alpha + u32::from(behind) * (255 - alpha) + 127) / 255) & 0xff
    };
    (mix(rgba[0], br) << 16) | (mix(rgba[1], bg) << 8) | mix(rgba[2], bb)
}

/// Write one pixel. The caller has already clipped, so this is an unconditional store.
fn put(pixels: &mut [u8], width: i32, x: i32, y: i32, value: u32) {
    let offset = ((y * width + x) * 4) as usize;
    pixels[offset..offset + 4].copy_from_slice(&value.to_ne_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Mode;

    const RED: u32 = 0x00ff_0000;
    const BLUE: u32 = 0x0000_00ff;
    const GRAY: Color = Color(0x0055_5555);

    fn buffer(width: i32, height: i32) -> Vec<u8> {
        vec![0; (width * height * 4) as usize]
    }

    fn pixel_at(pixels: &[u8], width: i32, x: i32, y: i32) -> u32 {
        let offset = ((y * width + x) * 4) as usize;
        u32::from_ne_bytes(pixels[offset..offset + 4].try_into().unwrap())
    }

    /// A 2x2 picture: red on top, blue underneath.
    fn picture() -> RgbaImage {
        let mut picture = RgbaImage::new(2, 2);
        for x in 0..2 {
            picture.put_pixel(x, 0, image::Rgba([255, 0, 0, 255]));
            picture.put_pixel(x, 1, image::Rgba([0, 0, 255, 255]));
        }
        picture
    }

    fn wallpaper(mode: Mode) -> Wallpaper {
        Wallpaper {
            image: Some(std::path::PathBuf::from("/pictures/test.png")),
            mode,
            color: GRAY,
        }
    }

    #[test]
    fn no_picture_is_a_plain_fill() {
        let mut pixels = buffer(4, 4);
        render(&mut pixels, 4, 4, &wallpaper(Mode::Fill), None);
        for y in 0..4 {
            for x in 0..4 {
                assert_eq!(pixel_at(&pixels, 4, x, y), GRAY.pixel(), "{x},{y}");
            }
        }
    }

    #[test]
    fn solid_ignores_the_picture_it_was_handed() {
        // Belt and braces over `Config::resolve`, which drops the path: if a picture reaches here
        // under `solid` anyway, it still must not be drawn.
        let mut pixels = buffer(4, 4);
        render(&mut pixels, 4, 4, &wallpaper(Mode::Solid), Some(&picture()));
        assert_eq!(pixel_at(&pixels, 4, 0, 0), GRAY.pixel());
        assert_eq!(pixel_at(&pixels, 4, 3, 3), GRAY.pixel());
    }

    #[test]
    fn tile_repeats_from_the_top_left() {
        let mut pixels = buffer(4, 4);
        render(&mut pixels, 4, 4, &wallpaper(Mode::Tile), Some(&picture()));
        // Rows alternate red, blue, red, blue and every column matches.
        for x in 0..4 {
            assert_eq!(pixel_at(&pixels, 4, x, 0), RED, "column {x}");
            assert_eq!(pixel_at(&pixels, 4, x, 1), BLUE, "column {x}");
            assert_eq!(pixel_at(&pixels, 4, x, 2), RED, "column {x}");
            assert_eq!(pixel_at(&pixels, 4, x, 3), BLUE, "column {x}");
        }
        // Nothing of the color is left showing: a tile covers the screen.
        assert!(
            (0..4).all(|y| (0..4).all(|x| pixel_at(&pixels, 4, x, y) != GRAY.pixel())),
            "a tile should leave no background visible"
        );
    }

    #[test]
    fn tile_covers_a_screen_that_is_not_a_multiple_of_the_tile() {
        // 5 is not a multiple of 2, so the last row and column are a partial repetition. This is
        // where an off-by-one shows up as an unpainted strip down the edge of the screen.
        let mut pixels = buffer(5, 5);
        render(&mut pixels, 5, 5, &wallpaper(Mode::Tile), Some(&picture()));
        for y in 0..5 {
            for x in 0..5 {
                let expected = if y % 2 == 0 { RED } else { BLUE };
                assert_eq!(pixel_at(&pixels, 5, x, y), expected, "{x},{y}");
            }
        }
    }

    #[test]
    fn a_single_pixel_tile_is_a_solid_fill() {
        // The fast path, and it must agree with the slow one to the bit.
        let mut single = RgbaImage::new(1, 1);
        single.put_pixel(0, 0, image::Rgba([255, 0, 0, 255]));
        let mut pixels = buffer(4, 4);
        render(&mut pixels, 4, 4, &wallpaper(Mode::Tile), Some(&single));
        for y in 0..4 {
            for x in 0..4 {
                assert_eq!(pixel_at(&pixels, 4, x, y), RED, "{x},{y}");
            }
        }
    }

    #[test]
    fn center_draws_at_native_size_with_the_color_around_it() {
        let mut pixels = buffer(4, 4);
        render(
            &mut pixels,
            4,
            4,
            &wallpaper(Mode::Center),
            Some(&picture()),
        );
        // A 2x2 picture in a 4x4 screen sits at (1,1).
        assert_eq!(pixel_at(&pixels, 4, 1, 1), RED);
        assert_eq!(pixel_at(&pixels, 4, 2, 1), RED);
        assert_eq!(pixel_at(&pixels, 4, 1, 2), BLUE);
        assert_eq!(pixel_at(&pixels, 4, 2, 2), BLUE);
        // ...with the color everywhere else.
        for (x, y) in [(0, 0), (3, 0), (0, 3), (3, 3), (0, 1), (3, 2)] {
            assert_eq!(pixel_at(&pixels, 4, x, y), GRAY.pixel(), "{x},{y}");
        }
    }

    #[test]
    fn center_crops_rather_than_scaling_when_the_picture_is_too_big() {
        // The picture is 4x4 and the screen 2x2, so the middle 2x2 is shown. Nothing may be
        // written outside the buffer, which is what the clipping in `blit` is for -- if it were
        // wrong this would panic rather than fail.
        let mut big = RgbaImage::new(4, 4);
        for y in 0..4 {
            for x in 0..4 {
                let inner = (1..3).contains(&x) && (1..3).contains(&y);
                big.put_pixel(
                    x,
                    y,
                    if inner {
                        image::Rgba([255, 0, 0, 255])
                    } else {
                        image::Rgba([0, 0, 255, 255])
                    },
                );
            }
        }
        let mut pixels = buffer(2, 2);
        render(&mut pixels, 2, 2, &wallpaper(Mode::Center), Some(&big));
        for y in 0..2 {
            for x in 0..2 {
                assert_eq!(
                    pixel_at(&pixels, 2, x, y),
                    RED,
                    "{x},{y} should be the middle"
                );
            }
        }
    }

    #[test]
    fn stretch_covers_every_pixel() {
        let mut pixels = buffer(8, 8);
        render(
            &mut pixels,
            8,
            8,
            &wallpaper(Mode::Stretch),
            Some(&picture()),
        );
        // Not a single pixel of the color survives, and the top half is nearer red than blue.
        for y in 0..8 {
            for x in 0..8 {
                assert_ne!(pixel_at(&pixels, 8, x, y), GRAY.pixel(), "{x},{y}");
            }
        }
        let top = pixel_at(&pixels, 8, 4, 0);
        let bottom = pixel_at(&pixels, 8, 4, 7);
        assert!((top >> 16) & 0xff > top & 0xff, "the top should be reddish");
        assert!(
            bottom & 0xff > (bottom >> 16) & 0xff,
            "the bottom should be bluish"
        );
    }

    #[test]
    fn fit_leaves_bars_and_fill_does_not() {
        // The pair that people actually notice. A square picture on a wide screen: `fit` must
        // leave the configured color at the left and right edges, and `fill` must not.
        let square = picture();

        let mut fitted = buffer(16, 8);
        render(&mut fitted, 16, 8, &wallpaper(Mode::Fit), Some(&square));
        assert_eq!(
            pixel_at(&fitted, 16, 0, 4),
            GRAY.pixel(),
            "fit should letterbox"
        );
        assert_eq!(
            pixel_at(&fitted, 16, 15, 4),
            GRAY.pixel(),
            "fit should letterbox"
        );
        assert_ne!(
            pixel_at(&fitted, 16, 8, 4),
            GRAY.pixel(),
            "and draw in the middle"
        );

        let mut filled = buffer(16, 8);
        render(&mut filled, 16, 8, &wallpaper(Mode::Fill), Some(&square));
        for y in 0..8 {
            for x in 0..16 {
                assert_ne!(
                    pixel_at(&filled, 16, x, y),
                    GRAY.pixel(),
                    "fill leaves no gap at {x},{y}"
                );
            }
        }
    }

    #[test]
    fn transparency_is_composited_against_the_color() {
        // The buffer is opaque, so alpha has to be resolved here or not at all.
        let mut half = RgbaImage::new(1, 1);
        half.put_pixel(0, 0, image::Rgba([255, 255, 255, 0]));
        let mut pixels = buffer(2, 2);
        render(&mut pixels, 2, 2, &wallpaper(Mode::Tile), Some(&half));
        // Fully transparent white over gray is gray, exactly -- not a shade off it.
        assert_eq!(pixel_at(&pixels, 2, 0, 0), GRAY.pixel());

        // ...and fully opaque is the pixel itself, untouched by the color behind it.
        half.put_pixel(0, 0, image::Rgba([255, 255, 255, 255]));
        render(&mut pixels, 2, 2, &wallpaper(Mode::Tile), Some(&half));
        assert_eq!(pixel_at(&pixels, 2, 0, 0), 0x00ff_ffff);
    }

    #[test]
    fn half_transparent_lands_between_the_two() {
        let mut half = RgbaImage::new(1, 1);
        half.put_pixel(0, 0, image::Rgba([255, 255, 255, 128]));
        let mut pixels = buffer(1, 1);
        render(&mut pixels, 1, 1, &wallpaper(Mode::Tile), Some(&half));
        let red = (pixel_at(&pixels, 1, 0, 0) >> 16) & 0xff;
        assert!((0x55..=0xff).contains(&red), "{red:#x}");
        // Roughly halfway between 0x55 and 0xff, allowing for the rounding.
        assert!((red as i32 - 0xaa).abs() <= 1, "{red:#x}");
    }

    #[test]
    fn a_zero_sized_surface_draws_nothing_and_does_not_panic() {
        // A layer surface is 0x0 until it is configured, and the buffer is empty to match.
        let mut pixels: Vec<u8> = Vec::new();
        render(&mut pixels, 0, 0, &wallpaper(Mode::Fill), Some(&picture()));
        assert!(pixels.is_empty());
    }
}
