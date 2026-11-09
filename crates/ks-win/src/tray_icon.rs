//! The tray icon picture (the Python's `windows_tray_native._render` and `constants/tray_icon.py`): a badge with the
//! layout's letters, or a flag, on a canvas `SUPERSAMPLING` times the side the notification area shows, averaged down by
//! whole blocks so that every edge of the picture lands on the icon's own pixels at each scale of the screen. Lengths are
//! given on the Python's 64-pixel picture and scale with the canvas. The glyphs of the badge come from the platform as a
//! coverage mask (`Ink`), since the font is Windows'.

use ks_cast::To as _;
use crate::constants::{
    FLAG_CORNER_RADIUS_ICON_PIXELS, FLAG_EDGE_RGBA, FLAG_HEIGHT_DENOMINATOR, FLAG_HEIGHT_NUMERATOR, FLAG_SLASH_RIM_PIXELS, FLAG_SLASH_RIM_RGBA,
    US_FLAG_STAR_FIRST_ROW, US_FLAG_STAR_STEP,
};
use crate::tray_model::TrayState;

/// How many canvas pixels make one pixel of the icon, along each side.
pub const SUPERSAMPLING: usize = 4;
/// The side of the picture the lengths below are given on.
const DESIGN_SIDE_PIXELS: usize = 64;
const BADGE_CORNER_RADIUS_PIXELS: usize = 13;
/// The box the letters are fitted into, centred on the badge: at 16 pixels it leaves a pixel and a half of badge at each
/// side of the two letters, two pixels from 20 on, and room above and below them.
const BADGE_TEXT_WIDTH_PIXELS: usize = 52;
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

    /// Leaves the picture inside the rounded rectangle and clears the rest; the pixels along its inside edge become `edge`.
    fn frame(&mut self, bounds: (i32, i32, i32, i32), radius: i32, edge: Rgba) {
        let (left, top, right, bottom) = bounds;
        let inside = |x: i32, y: i32| {
            let dx = (left + radius - x).max(x - (right - radius)).max(0);
            let dy = (top + radius - y).max(y - (bottom - radius)).max(0);
            x >= left && x <= right && y >= top && y <= bottom && dx * dx + dy * dy <= radius * radius
        };
        for y in 0..self.size.low::<i32>() {
            for x in 0..self.size.low::<i32>() {
                if !inside(x, y) {
                    self.put(x, y, [0, 0, 0, 0]);
                } else if !(inside(x - 1, y) && inside(x + 1, y) && inside(x, y - 1) && inside(x, y + 1)) {
                    self.put(x, y, edge);
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

/// The flag's rectangle on a canvas of `size` pixels, both corners in it: the whole width, `FLAG_HEIGHT_NUMERATOR /
/// FLAG_HEIGHT_DENOMINATOR` of it tall, centred, with its top and bottom on the edges of the icon's pixels.
#[must_use]
pub fn flag_bounds(size: usize) -> (i32, i32, i32, i32) {
    let icon = (size / SUPERSAMPLING).max(1);
    let height = (icon * FLAG_HEIGHT_NUMERATOR + FLAG_HEIGHT_DENOMINATOR / 2) / FLAG_HEIGHT_DENOMINATOR;
    let top = (icon - height) / 2 * SUPERSAMPLING;
    (0, top.low::<i32>(), size.low::<i32>() - 1, (top + height * SUPERSAMPLING).low::<i32>() - 1)
}

fn draw_flag(canvas: &mut Canvas, group: i64) {
    let (left, top, right, bottom) = flag_bounds(canvas.size);
    let (width, height) = ((right - left + 1).to::<usize>(), (bottom - top + 1).to::<usize>());
    if group == 1 {
        // The bands end on icon pixels: 4, 3 and 4 of the 11 at 16 pixels.
        let rows = height / SUPERSAMPLING;
        let edge = |numerator: usize| top + ((rows * numerator + RUSSIAN_FLAG_BAND_COUNT / 2) / RUSSIAN_FLAG_BAND_COUNT * SUPERSAMPLING).low::<i32>();
        let (first_edge, second_edge) = (edge(1), edge(RUSSIAN_FLAG_SECOND_BAND_NUMERATOR));
        canvas.rectangle(left, top, right, first_edge - 1, WHITE);
        canvas.rectangle(left, first_edge, right, second_edge - 1, RUSSIAN_FLAG_BLUE_RGBA);
        canvas.rectangle(left, second_edge, right, bottom, RUSSIAN_FLAG_RED_RGBA);
        return;
    }
    // Each stripe is a whole number of icon pixels, or the stripes blur into pink: thirteen where the flag has the rows
    // for them, and as many as it has (an odd number, red at both edges) where it has fewer, 11 at 16 pixels.
    let rows = height / SUPERSAMPLING;
    let stripes = US_FLAG_STRIPE_COUNT.min(rows - (1 - rows % 2));
    let stripe_edge = |index: usize| top + ((rows * index + stripes / 2) / stripes * SUPERSAMPLING).low::<i32>();
    for index in 0..stripes {
        canvas.rectangle(left, stripe_edge(index), right, stripe_edge(index + 1) - 1, if index % 2 == 0 { US_FLAG_STRIPE_RED_RGBA } else { WHITE });
    }
    let columns = width / SUPERSAMPLING;
    let canton_columns = (columns * US_FLAG_CANTON_WIDTH_NUMERATOR + US_FLAG_CANTON_WIDTH_DENOMINATOR / 2) / US_FLAG_CANTON_WIDTH_DENOMINATOR;
    let canton_width = canton_columns * SUPERSAMPLING;
    let canton_stripes = (stripes * US_FLAG_CANTON_HEIGHT_STRIPES + US_FLAG_STRIPE_COUNT / 2) / US_FLAG_STRIPE_COUNT;
    let canton_height = (stripe_edge(canton_stripes) - top).to::<usize>();
    canvas.rectangle(left, top, left + canton_width.low::<i32>() - 1, top + canton_height.low::<i32>() - 1, US_FLAG_CANTON_RGBA);
    // A star is one icon pixel, in every other row of the canton and every other column, the rows staggered: a dot drawn
    // smaller than a pixel only greys the blue.
    let star = SUPERSAMPLING.low::<i32>();
    for row in (US_FLAG_STAR_FIRST_ROW..canton_height / SUPERSAMPLING).step_by(US_FLAG_STAR_STEP) {
        let first_column = if row % (2 * US_FLAG_STAR_STEP) == US_FLAG_STAR_FIRST_ROW { US_FLAG_STAR_FIRST_ROW } else { US_FLAG_STAR_STEP };
        for column in (first_column..canton_columns).step_by(US_FLAG_STAR_STEP) {
            let (x, y) = (left + (column * SUPERSAMPLING).low::<i32>(), top + (row * SUPERSAMPLING).low::<i32>());
            canvas.rectangle(x, y, x + star - 1, y + star - 1, WHITE);
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
        let bounds = flag_bounds(side);
        if !state.enabled {
            // The slash crosses the flag as it crossed the square one, squeezed to the flag's height.
            let (top, height) = (bounds.1.to::<f64>(), (bounds.3 - bounds.1 + 1).to::<f64>());
            let across = |length: f64| top + height * length / DESIGN_SIDE_PIXELS.to::<f64>();
            let ends = (
                (scaled(DISABLED_SLASH_NEAR_PIXELS, side), across(DISABLED_SLASH_FAR_PIXELS)),
                (scaled(DISABLED_SLASH_FAR_PIXELS, side), across(DISABLED_SLASH_NEAR_PIXELS)),
            );
            // A dark rim first, so that the white slash still shows where it crosses the white of a flag.
            let width = scaled(DISABLED_SLASH_WIDTH_PIXELS, side);
            canvas.line(ends.0, ends.1, width + scaled(FLAG_SLASH_RIM_PIXELS, side), FLAG_SLASH_RIM_RGBA);
            canvas.line(ends.0, ends.1, width, DISABLED_SLASH_RGBA);
        }
        let radius = (FLAG_CORNER_RADIUS_ICON_PIXELS * SUPERSAMPLING).low::<i32>();
        canvas.frame(bounds, radius, FLAG_EDGE_RGBA);
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
    fn a_flag_is_wider_than_tall_in_the_middle_of_the_icon() {
        // 16 icon pixels: 16 by 11, two pixels clear above it and three below.
        assert_eq!(flag_bounds(SIDE), (0, 8, 63, 51));
        // 20 and 24 icon pixels: 14 and 17 rows.
        assert_eq!(flag_bounds(80), (0, 12, 79, 67));
        assert_eq!(flag_bounds(96), (0, 12, 95, 79));
        let canvas = render(&TrayState { group: 1, indicator_style: "flags", ..TrayState::default() }, None, SIDE);
        let at = |x: usize, y: usize| canvas.pixels[y * SIDE + x];
        // Nothing above or below the flag, and nothing in its rounded corners.
        assert_eq!((at(32, 7)[3], at(32, 52)[3], at(0, 8)[3], at(63, 51)[3]), (0, 0, 0, 0));
        // Bands of 4, 3 and 4 icon pixels.
        assert_eq!((at(32, 23), at(32, 24)), (WHITE, RUSSIAN_FLAG_BLUE_RGBA));
        assert_eq!((at(32, 35), at(32, 36)), (RUSSIAN_FLAG_BLUE_RGBA, RUSSIAN_FLAG_RED_RGBA));
        // The edge runs along all four sides.
        assert_eq!((at(32, 8), at(32, 51), at(0, 30), at(63, 30)), (FLAG_EDGE_RGBA, FLAG_EDGE_RGBA, FLAG_EDGE_RGBA, FLAG_EDGE_RGBA));
    }

    #[test]
    fn flags_and_the_slash() {
        let state = TrayState { group: 1, indicator_style: "flags", enabled: false, ..TrayState::default() };
        let canvas = render(&state, None, SIDE);
        assert_eq!(canvas.pixels[12 * 64 + 5], WHITE);
        assert_eq!(canvas.pixels[45 * 64 + 5], RUSSIAN_FLAG_RED_RGBA);
        // The slash through the middle of the flag, and its dark rim beside it.
        assert_eq!(canvas.pixels[30 * 64 + 32], DISABLED_SLASH_RGBA);
        assert_eq!(canvas.pixels[33 * 64 + 34], FLAG_SLASH_RIM_RGBA);
        assert_eq!(canvas.pixels[0][3], 0, "the slash stays in the flag");
        let us = render(&TrayState { group: 0, indicator_style: "flags", ..TrayState::default() }, None, SIDE);
        let at = |x: usize, y: usize| us.pixels[y * SIDE + x];
        // Eleven stripes of one icon pixel, red at both edges.
        assert_eq!((at(40, 9), at(40, 13), at(40, 49)), (US_FLAG_STRIPE_RED_RGBA, WHITE, US_FLAG_STRIPE_RED_RGBA));
        // The canton is six icon pixels each way, a star in every other pixel of it and the rows staggered.
        assert_eq!((at(5, 13), at(9, 13), at(21, 13)), (WHITE, US_FLAG_CANTON_RGBA, WHITE));
        assert_eq!((at(5, 21), at(9, 21)), (US_FLAG_CANTON_RGBA, WHITE));
        assert_eq!((at(23, 26), at(24, 26)), (US_FLAG_CANTON_RGBA, US_FLAG_STRIPE_RED_RGBA));
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
        assert_eq!(flag_bounds(2 * SIDE), (0, 20, 127, 107));
        // 22 rows hold all thirteen stripes, each one or two icon pixels: the first is two, the second one.
        assert_eq!((us.pixels[25 * 128 + 100], us.pixels[29 * 128 + 100], us.pixels[33 * 128 + 100]), (US_FLAG_STRIPE_RED_RGBA, WHITE, US_FLAG_STRIPE_RED_RGBA));
        // A star is still one icon pixel.
        assert_eq!((us.pixels[25 * 128 + 5], us.pixels[25 * 128 + 9]), (WHITE, US_FLAG_CANTON_RGBA));
        // 14 rows, an even number, hold thirteen as well, and the last one is red.
        let us = render(&TrayState { group: 0, indicator_style: "flags", ..TrayState::default() }, None, 80);
        assert_eq!((us.pixels[61 * 80 + 40], us.pixels[65 * 80 + 40]), (WHITE, US_FLAG_STRIPE_RED_RGBA));
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
        assert_eq!((width, height), (52, 44));
        let ink = fitted_ink(SIDE, letters).unwrap();
        // 26 pixels tall would fill the width; 24 tall and 48 wide is the tallest whole number of icon pixels near it.
        assert_eq!((ink.width, ink.height), (48, 24));
        // At 80 pixels the box is 65 by 55: 32 tall fills its width and is eight icon pixels.
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
        // A height the platform cannot draw is passed over: 37, the first height the box of a 64-pixel canvas tries.
        let ink = fitted_ink(SIDE, |font| if font == 37 { None } else { letters(font) }).unwrap();
        assert!(ink.width <= 52 && ink.height <= 44);
        // A single tall glyph is held by the height of the box.
        let tall = |font: i32| Some(Ink { width: 4, height: font.to::<usize>(), coverage: vec![255; 4 * font.to::<usize>()] });
        assert_eq!(fitted_ink(SIDE, tall).unwrap().height, 44);
    }
}
