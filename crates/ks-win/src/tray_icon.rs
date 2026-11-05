//! The tray icon picture (the Python's `windows_tray_native._render` and `constants/tray_icon.py`): a 64 by 64 canvas drawn
//! as Pillow draws it (rectangles, a rounded badge, a slash, flag stripes), then scaled to the size the tray asks for. The
//! glyphs of the badge come from the platform as a coverage mask (`Ink`), since the font is Windows'.

use ks_cast::To as _;
use crate::tray_model::TrayState;

pub const TRAY_ICON_SIZE_PIXELS: usize = 64;
const BADGE_INSET_PIXELS: i32 = 2;
const BADGE_EDGE_PIXELS: i32 = 61;
const BADGE_CORNER_RADIUS_PIXELS: i32 = 13;
pub const BADGE_FONT_SIZE_PIXELS: i32 = 27;
const BADGE_EN_RGBA: Rgba = [27, 92, 180, 255];
const BADGE_RU_RGBA: Rgba = [194, 42, 55, 255];
const BADGE_DISABLED_RGBA: Rgba = [105, 105, 105, 255];
const DISABLED_SLASH_NEAR_PIXELS: f64 = 12.0;
const DISABLED_SLASH_FAR_PIXELS: f64 = 52.0;
const DISABLED_SLASH_WIDTH_PIXELS: f64 = 7.0;
const DISABLED_SLASH_RGBA: Rgba = [255, 255, 255, 235];
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
const WHITE: Rgba = [255, 255, 255, 255];

pub type Rgba = [u8; 4];

/// The glyphs of a badge label: `coverage` is `width * height` bytes, 255 where the font covers the pixel fully, cropped
/// to the ink so that the badge can centre it.
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
                let (px, py) = (x.to::<f64>() + 0.5 - from.0, y.to::<f64>() + 0.5 - from.1);
                let along = ((px * dx + py * dy) / length_squared).clamp(0.0, 1.0);
                let (ox, oy) = (px - along * dx, py - along * dy);
                if (ox * ox + oy * oy).sqrt() <= width / 2.0 {
                    self.put(x, y, color);
                }
            }
        }
    }

    /// Blend white over the badge with the glyph coverage, the ink centred.
    fn text(&mut self, ink: &Ink) {
        let left = (self.size.low::<i32>() - ink.width.low::<i32>()) / 2;
        let top = (self.size.low::<i32>() - ink.height.low::<i32>()) / 2;
        for row in 0..ink.height {
            for column in 0..ink.width {
                let cover = (ink.coverage[row * ink.width + column]).to::<u32>();
                let (x, y) = (left + column.low::<i32>(), top + row.low::<i32>());
                if cover == 0 || x < 0 || y < 0 || x.to::<usize>() >= self.size || y.to::<usize>() >= self.size {
                    continue;
                }
                let pixel = &mut self.pixels[y.to::<usize>() * self.size + x.to::<usize>()];
                for channel in 0..3 {
                    pixel[channel] = ((pixel[channel].to::<u32>() * (255 - cover) + 255 * cover + 127) / 255).low::<u8>();
                }
            }
        }
    }
}

fn draw_flag(canvas: &mut Canvas, group: i64) {
    let size = TRAY_ICON_SIZE_PIXELS;
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
            let center_x = offset + column * US_FLAG_STAR_GRID_SPACING_PIXELS;
            let center_y = US_FLAG_STAR_TOP_MARGIN_PIXELS + row * US_FLAG_STAR_GRID_SPACING_PIXELS;
            // A star is the ellipse of a 3 by 3 box: the pixel and its four neighbours.
            for (dx, dy) in [(0, 0), (-1, 0), (1, 0), (0, -1), (0, 1)] {
                canvas.put(center_x + dx, center_y + dy, WHITE);
            }
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

/// `_render`: the 64 by 64 picture of `state`; `ink` is the glyphs of `badge_text`, not needed for a flag.
#[must_use]
pub fn render(state: &TrayState, ink: Option<&Ink>) -> Canvas {
    let mut canvas = Canvas::new(TRAY_ICON_SIZE_PIXELS);
    if is_flag(state) {
        draw_flag(&mut canvas, state.group);
    } else {
        let background = if !state.enabled {
            BADGE_DISABLED_RGBA
        } else if state.group == 1 {
            BADGE_RU_RGBA
        } else {
            BADGE_EN_RGBA
        };
        canvas.rounded_rectangle(
            BADGE_INSET_PIXELS,
            BADGE_INSET_PIXELS,
            BADGE_EDGE_PIXELS,
            BADGE_EDGE_PIXELS,
            BADGE_CORNER_RADIUS_PIXELS,
            background,
        );
        if let Some(ink) = ink {
            canvas.text(ink);
        }
    }
    if !state.enabled {
        canvas.line(
            (DISABLED_SLASH_NEAR_PIXELS, DISABLED_SLASH_FAR_PIXELS),
            (DISABLED_SLASH_FAR_PIXELS, DISABLED_SLASH_NEAR_PIXELS),
            DISABLED_SLASH_WIDTH_PIXELS,
            DISABLED_SLASH_RGBA,
        );
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

    #[test]
    fn badge_has_transparent_corners_and_the_layout_colour() {
        let canvas = render(&TrayState { group: 1, ..TrayState::default() }, None);
        assert_eq!(canvas.pixels[0][3], 0);
        assert_eq!(canvas.pixels[32 * 64 + 32], BADGE_RU_RGBA);
    }

    #[test]
    fn flags_and_the_slash() {
        let state = TrayState { group: 1, indicator_style: "flags", enabled: false, ..TrayState::default() };
        let canvas = render(&state, None);
        assert_eq!(canvas.pixels[5 * 64 + 5], WHITE);
        assert_eq!(canvas.pixels[63 * 64], RUSSIAN_FLAG_RED_RGBA);
        assert_eq!(canvas.pixels[32 * 64 + 32], DISABLED_SLASH_RGBA);
        let us = render(&TrayState { group: 0, indicator_style: "flags", ..TrayState::default() }, None);
        assert_eq!(us.pixels[63 * 64 + 63], US_FLAG_STRIPE_RED_RGBA);
        assert_eq!(us.pixels[3 * 64 + 3], WHITE);
    }

    #[test]
    fn scaling_keeps_a_flat_colour() {
        let canvas = render(&TrayState { group: 1, ..TrayState::default() }, None);
        let bytes = scaled_bgra(&canvas, 16);
        assert_eq!(bytes.len(), 16 * 16 * 4);
        assert_eq!(&bytes[(8 * 16 + 8) * 4..(8 * 16 + 8) * 4 + 4], &[55, 42, 194, 255]);
    }
}
