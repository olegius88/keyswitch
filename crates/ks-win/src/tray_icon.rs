//! The tray icon picture (the Python's `windows_tray_native._render` and `constants/tray_icon.py`): a badge with the
//! layout's letters, or a flag, on a canvas `SUPERSAMPLING` times the side the notification area shows, averaged down by
//! whole blocks so that every edge of the picture lands on the icon's own pixels at each scale of the screen. Lengths are
//! given on the Python's 64-pixel picture and scale with the canvas. The glyphs of the badge come from the platform as a
//! coverage mask (`Ink`), since the font is Windows'.

use ks_cast::To as _;
use crate::tray_model::TrayState;

/// How many canvas pixels make one pixel of the icon, along each side.
pub const SUPERSAMPLING: usize = 4;
/// The side of the picture the lengths below are given on.
const DESIGN_SIDE_PIXELS: usize = 64;
const BADGE_CORNER_RADIUS_PIXELS: usize = 13;
/// The box the letters are fitted into, centred on the badge: at 16 pixels it leaves a pixel of badge at each side of
/// the two letters, and room above and below them.
const BADGE_TEXT_WIDTH_PIXELS: usize = 56;
const BADGE_TEXT_HEIGHT_PIXELS: usize = 44;
/// How many font heights that fit are tried, from the largest down, for letters a whole number of icon pixels tall.
const FONT_FIT_CANDIDATES: usize = 6;
const BADGE_EN_RGBA: Rgba = [27, 92, 180, 255];
const BADGE_RU_RGBA: Rgba = [194, 42, 55, 255];
const BADGE_DISABLED_RGBA: Rgba = [105, 105, 105, 255];
const DISABLED_SLASH_NEAR_PIXELS: f64 = 12.0;
const DISABLED_SLASH_FAR_PIXELS: f64 = 52.0;
const DISABLED_SLASH_WIDTH_PIXELS: f64 = 7.0;
const DISABLED_SLASH_RGBA: Rgba = [255, 255, 255, 235];
/// The letters of a switched-off badge: light grey on its grey, dimmer than white but plain to read. Crossed out by the
/// slash, the large letters read wrong at the small sizes (`EN` as `EKI`), so the grey alone says the badge is off; a
/// flag, which has no letters to spoil, keeps its slash.
pub const DISABLED_LETTERS_RGBA: Rgba = [210, 210, 210, 255];
const RUSSIAN_FLAG_BAND_COUNT: usize = 3;
const RUSSIAN_FLAG_SECOND_BAND_NUMERATOR: usize = 2;
const RUSSIAN_FLAG_BLUE_RGBA: Rgba = [0, 57, 166, 255];
const RUSSIAN_FLAG_RED_RGBA: Rgba = [213, 43, 30, 255];
const US_FLAG_STRIPE_COUNT: usize = 13;
const US_FLAG_STRIPE_RED_RGBA: Rgba = [178, 34, 52, 255];
const US_FLAG_CANTON_WIDTH_NUMERATOR: usize = 2;
const US_FLAG_CANTON_WIDTH_DENOMINATOR: usize = 5;
const US_FLAG_CANTON_HEIGHT_STRIPES: usize = 7;
const US_FLAG_CANTON_RGBA: Rgba = [60, 59, 110, 255];
const US_FLAG_STAR_ROWS: i32 = 5;
const US_FLAG_STARS_PER_LONG_ROW: i32 = 4;
const US_FLAG_STARS_PER_SHORT_ROW: i32 = 3;
const US_FLAG_STAR_OFFSET_LONG_ROW_PIXELS: i32 = 3;
const US_FLAG_STAR_OFFSET_SHORT_ROW_PIXELS: i32 = 6;
const US_FLAG_STAR_GRID_SPACING_PIXELS: i32 = 6;
const US_FLAG_STAR_TOP_MARGIN_PIXELS: i32 = 3;
/// A star is the pixels whose centre is this far from its own, on the 64-pixel picture: the pixel and its four
/// neighbours, the ellipse of a 3 by 3 box that Pillow drew.
const US_FLAG_STAR_RADIUS_PIXELS: f64 = 1.0;
/// The middle of a pixel, from its corner.
const PIXEL_CENTRE: f64 = 0.5;
const WHITE: Rgba = [255, 255, 255, 255];

pub type Rgba = [u8; 4];

/// The glyphs of a badge label: `coverage` is `width * height` bytes, 255 where the font covers the pixel fully, cropped
/// to the ink so that the badge can centre it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ink {
    pub width: usize,
    pub height: usize,
    pub coverage: Vec<u8>,
}

/// Straight (not premultiplied) RGBA, row by row from the top.
pub struct Canvas {
    pub size: usize,
    pub pixels: Vec<Rgba>,
}

/// A length of the 64-pixel picture on a canvas of `side` pixels.
fn scaled(length: f64, side: usize) -> f64 {
    length * side.to::<f64>() / DESIGN_SIDE_PIXELS.to::<f64>()
}

/// The canvas offset nearest to `offset` that is a whole number of icon pixels.
fn on_icon_pixel(offset: i32) -> i32 {
    let step = SUPERSAMPLING.low::<i32>();
    (offset + step / 2).div_euclid(step) * step
}

impl Canvas {
    fn new(size: usize) -> Canvas {
        Canvas { size, pixels: vec![[0, 0, 0, 0]; size * size] }
    }

    fn put(&mut self, x: i32, y: i32, color: Rgba) {
        if x >= 0 && y >= 0 && x.to::<usize>() < self.size && y.to::<usize>() < self.size {
            self.pixels[y.to::<usize>() * self.size + x.to::<usize>()] = color;
        }
    }

    /// `draw.rectangle((left, top, right, bottom), fill=...)`: both corners belong to the rectangle.
    fn rectangle(&mut self, left: i32, top: i32, right: i32, bottom: i32, color: Rgba) {
        for y in top..=bottom {
            for x in left..=right {
                self.put(x, y, color);
            }
        }
    }

    /// `draw.rounded_rectangle(..., radius=...)`.
    fn rounded_rectangle(&mut self, left: i32, top: i32, right: i32, bottom: i32, radius: i32, color: Rgba) {
        for y in top..=bottom {
            for x in left..=right {
                // The corner circle's centre is `radius` inside the edges; a pixel past it on both axes must be inside.
                let dx = (left + radius - x).max(x - (right - radius)).max(0);
                let dy = (top + radius - y).max(y - (bottom - radius)).max(0);
                if dx * dx + dy * dy <= radius * radius {
                    self.put(x, y, color);
                }
            }
        }
    }

    /// `draw.line(..., width=...)`: pixels whose centre is within half the width of the segment.
    fn line(&mut self, from: (f64, f64), to: (f64, f64), width: f64, color: Rgba) {
        let (dx, dy) = (to.0 - from.0, to.1 - from.1);
        let length_squared = dx * dx + dy * dy;
        for y in 0..self.size.low::<i32>() {
            for x in 0..self.size.low::<i32>() {
                let (px, py) = (x.to::<f64>() + PIXEL_CENTRE - from.0, y.to::<f64>() + PIXEL_CENTRE - from.1);
                let along = ((px * dx + py * dy) / length_squared).clamp(0.0, 1.0);
                let (ox, oy) = (px - along * dx, py - along * dy);
                if (ox * ox + oy * oy).sqrt() <= width / 2.0 {
                    self.put(x, y, color);
                }
            }
        }
    }

    /// The pixels whose centre is within `radius` of `centre`.
    fn disc(&mut self, centre: (f64, f64), radius: f64, color: Rgba) {
        let (left, right) = ((centre.0 - radius).floor().to::<i32>(), (centre.0 + radius).ceil().to::<i32>());
        let (top, bottom) = ((centre.1 - radius).floor().to::<i32>(), (centre.1 + radius).ceil().to::<i32>());
        for y in top..=bottom {
            for x in left..=right {
                let (dx, dy) = (x.to::<f64>() + PIXEL_CENTRE - centre.0, y.to::<f64>() + PIXEL_CENTRE - centre.1);
                if dx * dx + dy * dy <= radius * radius {
                    self.put(x, y, color);
                }
            }
        }
    }

    /// Blend `color` over the badge with the glyph coverage, the ink centred and its corner on an icon pixel.
    fn text(&mut self, ink: &Ink, color: Rgba) {
        let left = on_icon_pixel((self.size.low::<i32>() - ink.width.low::<i32>()) / 2);
        let top = on_icon_pixel((self.size.low::<i32>() - ink.height.low::<i32>()) / 2);
        for row in 0..ink.height {
            for column in 0..ink.width {
                let cover = (ink.coverage[row * ink.width + column]).to::<u32>();
                let (x, y) = (left + column.low::<i32>(), top + row.low::<i32>());
                if cover == 0 || x < 0 || y < 0 || x.to::<usize>() >= self.size || y.to::<usize>() >= self.size {
                    continue;
                }
                let pixel = &mut self.pixels[y.to::<usize>() * self.size + x.to::<usize>()];
                for channel in 0..3 {
                    pixel[channel] =
                        ((pixel[channel].to::<u32>() * (255 - cover) + color[channel].to::<u32>() * cover + 127) / 255).low::<u8>();
                }
            }
        }
    }
}

fn draw_flag(canvas: &mut Canvas, group: i64) {
    let size = canvas.size;
    let last = size.low::<i32>() - 1;
    if group == 1 {
        let first_edge = (size / RUSSIAN_FLAG_BAND_COUNT).low::<i32>();
        let second_edge = (size * RUSSIAN_FLAG_SECOND_BAND_NUMERATOR / RUSSIAN_FLAG_BAND_COUNT).low::<i32>();
        canvas.rectangle(0, 0, last, first_edge - 1, WHITE);
        canvas.rectangle(0, first_edge, last, second_edge - 1, RUSSIAN_FLAG_BLUE_RGBA);
        canvas.rectangle(0, second_edge, last, last, RUSSIAN_FLAG_RED_RGBA);
        return;
    }
    for index in 0..US_FLAG_STRIPE_COUNT {
        let top = (index * size / US_FLAG_STRIPE_COUNT).low::<i32>();
        let bottom = ((index + 1) * size / US_FLAG_STRIPE_COUNT).low::<i32>() - 1;
        canvas.rectangle(0, top, last, bottom, if index % 2 == 0 { US_FLAG_STRIPE_RED_RGBA } else { WHITE });
    }
    let canton_right = (size * US_FLAG_CANTON_WIDTH_NUMERATOR / US_FLAG_CANTON_WIDTH_DENOMINATOR).low::<i32>() - 1;
    let canton_bottom = (size * US_FLAG_CANTON_HEIGHT_STRIPES / US_FLAG_STRIPE_COUNT).low::<i32>() - 1;
    canvas.rectangle(0, 0, canton_right, canton_bottom, US_FLAG_CANTON_RGBA);
    for row in 0..US_FLAG_STAR_ROWS {
        let long_row = row % 2 == 0;
        let stars = if long_row { US_FLAG_STARS_PER_LONG_ROW } else { US_FLAG_STARS_PER_SHORT_ROW };
        let offset = if long_row { US_FLAG_STAR_OFFSET_LONG_ROW_PIXELS } else { US_FLAG_STAR_OFFSET_SHORT_ROW_PIXELS };
        for column in 0..stars {
            let centre_x = (offset + column * US_FLAG_STAR_GRID_SPACING_PIXELS).to::<f64>() + PIXEL_CENTRE;
            let centre_y = (US_FLAG_STAR_TOP_MARGIN_PIXELS + row * US_FLAG_STAR_GRID_SPACING_PIXELS).to::<f64>() + PIXEL_CENTRE;
            canvas.disc((scaled(centre_x, size), scaled(centre_y, size)), scaled(US_FLAG_STAR_RADIUS_PIXELS, size), WHITE);
        }
    }
}

/// Whether the picture is the flag one; the badge otherwise, with the label `badge_text` returns.
#[must_use]
pub fn is_flag(state: &TrayState) -> bool {
    state.indicator_style == "flags" && (state.group == 0 || state.group == 1)
}

/// The text the badge carries: the layout label, "?" for no layout.
#[must_use]
pub fn badge_text(state: &TrayState) -> &'static str {
    if state.group >= 0 { state.label() } else { "?" }
}

/// The width and the height the letters may take on a canvas of `side` pixels.
#[must_use]
pub fn text_box(side: usize) -> (usize, usize) {
    (side * BADGE_TEXT_WIDTH_PIXELS / DESIGN_SIDE_PIXELS, side * BADGE_TEXT_HEIGHT_PIXELS / DESIGN_SIDE_PIXELS)
}

/// The glyphs of a label as large as the text box of a canvas of `side` pixels takes them: `glyphs` draws the label at a
/// font height in pixels. The largest height whose ink fits is preferred, unless one of the few below it gives ink a
/// whole number of icon pixels tall, whose tops and feet then fall on the icon's pixel edges instead of blurring across
/// two. None when the label has no ink at any height.
pub fn fitted_ink(side: usize, mut glyphs: impl FnMut(i32) -> Option<Ink>) -> Option<Ink> {
    let (width, height) = text_box(side);
    // A first drawing at the box's height says how large the box lets the glyphs grow: their ink grows with the font.
    let probe = glyphs(height.low::<i32>())?;
    let growth = (width.to::<f64>() / probe.width.max(1).to::<f64>()).min(height.to::<f64>() / probe.height.max(1).to::<f64>());
    let largest = (height.to::<f64>() * growth).floor().to::<i32>() + 1;
    let mut fitting = Vec::new();
    for font in (1..=largest).rev() {
        let Some(ink) = glyphs(font) else { continue };
        if ink.width > width || ink.height > height {
            continue;
        }
        if ink.height % SUPERSAMPLING == 0 {
            return Some(ink);
        }
        fitting.push(ink);
        if fitting.len() == FONT_FIT_CANDIDATES {
            break;
        }
    }
    fitting.into_iter().next()
}

/// `_render`: the picture of `state` on a canvas of `side` pixels; `ink` is the glyphs of `badge_text`, not needed for a
/// flag.
#[must_use]
pub fn render(state: &TrayState, ink: Option<&Ink>, side: usize) -> Canvas {
    let mut canvas = Canvas::new(side);
    if is_flag(state) {
        draw_flag(&mut canvas, state.group);
        if !state.enabled {
            canvas.line(
                (scaled(DISABLED_SLASH_NEAR_PIXELS, side), scaled(DISABLED_SLASH_FAR_PIXELS, side)),
                (scaled(DISABLED_SLASH_FAR_PIXELS, side), scaled(DISABLED_SLASH_NEAR_PIXELS, side)),
                scaled(DISABLED_SLASH_WIDTH_PIXELS, side),
                DISABLED_SLASH_RGBA,
            );
        }
    } else {
        let background = if !state.enabled {
            BADGE_DISABLED_RGBA
        } else if state.group == 1 {
            BADGE_RU_RGBA
        } else {
            BADGE_EN_RGBA
        };
        // The badge takes the whole icon, as the flags do: its edges are the icon's, and the letters have all of it.
        let last = side.low::<i32>() - 1;
        let radius = scaled(BADGE_CORNER_RADIUS_PIXELS.to::<f64>(), side).round().to::<i32>();
        canvas.rounded_rectangle(0, 0, last, last, radius, background);
        if let Some(ink) = ink {
            canvas.text(ink, if state.enabled { WHITE } else { DISABLED_LETTERS_RGBA });
        }
    }
    canvas
}

/// The picture at `target` pixels square, as bytes of straight-alpha BGRA (what a 32-bit icon bitmap holds), averaged over
/// the source pixels each target pixel covers.
#[must_use]
pub fn scaled_bgra(canvas: &Canvas, target: usize) -> Vec<u8> {
    let target = target.max(1);
    let mut out = Vec::with_capacity(target * target * 4);
    for ty in 0..target {
        let (y0, y1) = (ty * canvas.size / target, ((ty + 1) * canvas.size / target).max(ty * canvas.size / target + 1));
        for tx in 0..target {
            let (x0, x1) = (tx * canvas.size / target, ((tx + 1) * canvas.size / target).max(tx * canvas.size / target + 1));
            let (mut sums, mut alpha, mut count) = ([0u32; 3], 0u32, 0u32);
            for y in y0..y1.min(canvas.size) {
                for x in x0..x1.min(canvas.size) {
                    let pixel = canvas.pixels[y * canvas.size + x];
                    for channel in 0..3 {
                        sums[channel] += pixel[channel].to::<u32>() * pixel[3].to::<u32>();
                    }
                    alpha += pixel[3].to::<u32>();
                    count += 1;
                }
            }
            let color = |channel: usize| (sums[channel] + alpha / 2).checked_div(alpha).unwrap_or(0).low::<u8>();
            out.extend_from_slice(&[color(2), color(1), color(0), ((alpha + count / 2) / count.max(1)).low::<u8>()]);
        }
    }
    out
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;

    const SIDE: usize = DESIGN_SIDE_PIXELS;

    #[test]
    fn a_badge_says_its_layout_or_a_question() {
        let ink = Ink { width: 80, height: 3, coverage: (0..240).map(|index| if index % 3 == 0 { 0 } else { 200 }).collect() };
        for (state, text) in [
            (TrayState { group: 0, ..TrayState::default() }, "EN"),
            (TrayState { group: 1, ..TrayState::default() }, "RU"),
            (TrayState { group: -1, ..TrayState::default() }, "?"),
            (TrayState { group: 0, enabled: false, ..TrayState::default() }, "EN"),
        ] {
            assert_eq!(badge_text(&state), text);
            let canvas = render(&state, Some(&ink), SIDE);
            assert_eq!(canvas.pixels.len(), SIDE * SIDE);
        }
        let tall = Ink { width: 2, height: 80, coverage: vec![255; 160] };
        let _ = render(&TrayState { group: 0, ..TrayState::default() }, Some(&tall), SIDE);
    }

    #[test]
    fn a_point_off_the_picture_is_not_drawn() {
        let mut canvas = Canvas::new(4);
        for (x, y) in [(-1, 0), (0, -1), (4, 0), (0, 4)] {
            canvas.put(x, y, WHITE);
        }
        assert!(canvas.pixels.iter().all(|pixel| *pixel == [0, 0, 0, 0]));
        canvas.put(1, 1, WHITE);
        assert_eq!(canvas.pixels[5], WHITE);
    }

    #[test]
    fn the_badge_takes_the_whole_icon_but_its_round_corners() {
        let canvas = render(&TrayState { group: 1, ..TrayState::default() }, None, SIDE);
        assert_eq!(canvas.pixels[0][3], 0);
        assert_eq!(canvas.pixels[32 * SIDE], BADGE_RU_RGBA);
        assert_eq!(canvas.pixels[32 * SIDE + SIDE - 1], BADGE_RU_RGBA);
        assert_eq!(canvas.pixels[32 * SIDE + 32], BADGE_RU_RGBA);
    }

    #[test]
    fn letters_sit_on_icon_pixels_in_the_middle_of_the_badge() {
        let ink = Ink { width: 6, height: 8, coverage: vec![255; 48] };
        let canvas = render(&TrayState { group: 0, ..TrayState::default() }, Some(&ink), 16);
        let white: Vec<usize> = (0..256).filter(|index| canvas.pixels[*index] == WHITE).collect();
        // (16 - 6) / 2 = 5 rounds to 4, the nearest whole icon pixel; (16 - 8) / 2 = 4 is one already.
        assert_eq!(white.first(), Some(&(4 * 16 + 4)));
        assert_eq!(white.len(), 48);
        assert_eq!(on_icon_pixel(-3), -4);
        assert_eq!(on_icon_pixel(6), 8);
    }

    #[test]
    fn flags_and_the_slash() {
        let state = TrayState { group: 1, indicator_style: "flags", enabled: false, ..TrayState::default() };
        let canvas = render(&state, None, SIDE);
        assert_eq!(canvas.pixels[5 * 64 + 5], WHITE);
        assert_eq!(canvas.pixels[63 * 64], RUSSIAN_FLAG_RED_RGBA);
        assert_eq!(canvas.pixels[32 * 64 + 32], DISABLED_SLASH_RGBA);
        let us = render(&TrayState { group: 0, indicator_style: "flags", ..TrayState::default() }, None, SIDE);
        assert_eq!(us.pixels[63 * 64 + 63], US_FLAG_STRIPE_RED_RGBA);
        // A star of the 64-pixel flag is the pixel and its four neighbours, as Pillow drew it.
        assert_eq!(us.pixels[3 * 64 + 3], WHITE);
        assert_eq!(us.pixels[3 * 64 + 4], WHITE);
        assert_eq!(us.pixels[4 * 64 + 3], WHITE);
        assert_eq!(us.pixels[4 * 64 + 4], US_FLAG_CANTON_RGBA);
    }

    #[test]
    fn a_switched_off_badge_is_grey_with_dim_letters_and_no_slash() {
        let ink = Ink { width: 40, height: 24, coverage: vec![255; 960] };
        let off = render(&TrayState { group: 0, enabled: false, ..TrayState::default() }, Some(&ink), SIDE);
        assert_eq!(off.pixels[32 * SIDE + 32], DISABLED_LETTERS_RGBA);
        assert_eq!(off.pixels[2 * SIDE + 32], BADGE_DISABLED_RGBA);
        // Where the slash crossed the badge, the badge is grey.
        assert_eq!(off.pixels[52 * SIDE + 12], BADGE_DISABLED_RGBA);
        assert!(off.pixels.iter().all(|pixel| *pixel != WHITE && *pixel != DISABLED_SLASH_RGBA));
    }

    #[test]
    fn a_flag_scales_with_its_canvas() {
        let us = render(&TrayState { group: 0, indicator_style: "flags", ..TrayState::default() }, None, 2 * SIDE);
        assert_eq!(us.pixels.len(), 4 * SIDE * SIDE);
        // The star of the 64-pixel picture at (3, 3) is a disc around (7, 7) here.
        assert_eq!(us.pixels[7 * 128 + 7], WHITE);
        assert_eq!(us.pixels[7 * 128 + 8], WHITE);
        assert_eq!(us.pixels[9 * 128 + 9], US_FLAG_CANTON_RGBA);
    }

    #[test]
    fn scaling_keeps_a_flat_colour() {
        let canvas = render(&TrayState { group: 1, ..TrayState::default() }, None, SIDE);
        let bytes = scaled_bgra(&canvas, 16);
        assert_eq!(bytes.len(), 16 * 16 * 4);
        assert_eq!(&bytes[(8 * 16 + 8) * 4..(8 * 16 + 8) * 4 + 4], &[55, 42, 194, 255]);
    }

    /// Glyphs whose ink grows with the font height as two letters do: twice as wide as tall, 70 % of the font tall.
    fn letters(font: i32) -> Option<Ink> {
        let height = (font.to::<f64>() * 0.7).round().to::<usize>();
        (height > 0).then(|| Ink { width: 2 * height, height, coverage: vec![255; 2 * height * height] })
    }

    #[test]
    fn the_letters_fill_the_width_and_prefer_whole_icon_pixels() {
        let (width, height) = text_box(SIDE);
        assert_eq!((width, height), (56, 44));
        let ink = fitted_ink(SIDE, letters).unwrap();
        // 28 pixels tall and 56 wide fill the width and are seven icon pixels.
        assert_eq!((ink.width, ink.height), (56, 28));
        // At 80 pixels the box is 70 by 55: 35 tall would fit, 32 is the tallest whole number of icon pixels.
        let ink = fitted_ink(80, letters).unwrap();
        assert_eq!((ink.width, ink.height), (64, 32));
    }

    #[test]
    fn without_a_whole_pixel_height_near_the_largest_the_largest_is_taken() {
        // Ink always three canvas pixels taller than a multiple of four: no candidate is whole.
        let odd = |font: i32| {
            let height = font.to::<usize>() / 4 * 4 + 3;
            Some(Ink { width: height, height, coverage: vec![255; height * height] })
        };
        let ink = fitted_ink(SIDE, odd).unwrap();
        assert_eq!(ink.height, 43);
        // Glyphs that never fit, and a label without ink, give none.
        assert_eq!(fitted_ink(SIDE, |_| Some(Ink { width: 99, height: 1, coverage: vec![255; 99] })), None);
        assert_eq!(fitted_ink(SIDE, |_| None), None);
        // A height the platform cannot draw is passed over.
        let ink = fitted_ink(SIDE, |font| if font == 40 { None } else { letters(font) }).unwrap();
        assert!(ink.width <= 56 && ink.height <= 44);
        // A single tall glyph is held by the height of the box.
        let tall = |font: i32| Some(Ink { width: 4, height: font.to::<usize>(), coverage: vec![255; 4 * font.to::<usize>()] });
        assert_eq!(fitted_ink(SIDE, tall).unwrap().height, 44);
    }
}
