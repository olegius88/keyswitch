//! The arithmetic of the hint card (`hint_card`), which needs no window: where its parts go, what a click at a point means,
//! where the card goes on the screen, and which page of the settings window a hint's page is. Compiled and tested on every
//! system.

use ks_ui::hints::{Click, SettingsPage};

use crate::constants::HINT_CARD_BORDER_PIXELS;
use crate::windows_ui_model::{PAGE_NAMES, prompt_position};

/// A rectangle in pixels, of the screen or of a window's client area.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Area {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Area {
    #[must_use]
    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.left && x < self.right && y >= self.top && y < self.bottom
    }

    /// Nothing is there: a rectangle with no pixels.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.right <= self.left || self.bottom <= self.top
    }
}

/// What the parts of a card measure, in pixels at its scale.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CardMetrics {
    pub width: i32,
    pub pad_x: i32,
    pub pad_y: i32,
    pub line_gap: i32,
    pub link_gap: i32,
    pub title_height: i32,
    pub text_height: i32,
    pub link_height: i32,
    pub ok_width: i32,
    pub never_width: i32,
}

/// Where the parts of a card go, in its client area.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CardLayout {
    pub size: (i32, i32),
    pub title: Area,
    pub text: Area,
    pub ok: Area,
    pub never: Area,
}

/// The width the text of a card of `width` wraps at.
#[must_use]
pub fn card_text_width(width: i32, pad_x: i32) -> i32 {
    width - 2 * (HINT_CARD_BORDER_PIXELS + pad_x)
}

/// The two links, on the last line of a card `height` pixels high.
#[must_use]
pub fn link_row(metrics: &CardMetrics, height: i32) -> (Area, Area) {
    let top = height - HINT_CARD_BORDER_PIXELS - metrics.pad_y - metrics.link_height;
    let bottom = top + metrics.link_height;
    let left = HINT_CARD_BORDER_PIXELS + metrics.pad_x;
    let ok = Area { left, top, right: left + metrics.ok_width, bottom };
    let never_left = ok.right + metrics.link_gap;
    (ok, Area { left: never_left, top, right: never_left + metrics.never_width, bottom })
}

/// The title on top, the text under it, the links on the last line.
#[must_use]
pub fn card_layout(metrics: &CardMetrics) -> CardLayout {
    let height = 2 * (HINT_CARD_BORDER_PIXELS + metrics.pad_y)
        + metrics.title_height
        + metrics.line_gap
        + metrics.text_height
        + metrics.line_gap
        + metrics.link_height;
    let left = HINT_CARD_BORDER_PIXELS + metrics.pad_x;
    let right = metrics.width - left;
    let title_top = HINT_CARD_BORDER_PIXELS + metrics.pad_y;
    let title = Area { left, top: title_top, right, bottom: title_top + metrics.title_height };
    let text_top = title.bottom + metrics.line_gap;
    let text = Area { left, top: text_top, right, bottom: text_top + metrics.text_height };
    let (ok, never) = link_row(metrics, height);
    CardLayout { size: (metrics.width, height), title, text, ok, never }
}

/// What a click at `x`, `y` of the card means: the links, and the text when it leads somewhere.
#[must_use]
pub fn card_hit(layout: &CardLayout, x: i32, y: i32, text_opens: bool) -> Option<Click> {
    if layout.ok.contains(x, y) {
        return Some(Click::Ok);
    }
    if layout.never.contains(x, y) {
        return Some(Click::Never);
    }
    (text_opens && layout.text.contains(x, y)).then_some(Click::Text)
}

/// Where a card of `size` goes for something at `target` on the screen: centred over it and above it, or below it when
/// there is no room above; inside `screen`, `(x, y, width, height)` of the virtual screen.
#[must_use]
pub fn card_position(target: Area, size: (i32, i32), screen: (i32, i32, i32, i32), gap: i32, margin: i32) -> (i32, i32) {
    let centre = i32::midpoint(target.left, target.right);
    let (x, above) = prompt_position((centre, target.top), size, screen, gap, margin);
    let (_, top, _, height) = screen;
    if target.top - size.1 - gap >= top + margin {
        return (x, above);
    }
    let below = target.bottom + gap;
    (x, (top + margin).max(below.min(top + height - size.1 - margin)))
}

/// Whether what the shell says about the icon's place can be believed: it has an area and lies on the screen.
#[must_use]
pub fn icon_is_on_screen(icon: Area, screen: (i32, i32, i32, i32)) -> bool {
    let (left, top, width, height) = screen;
    !icon.is_empty() && icon.left >= left && icon.top >= top && icon.right <= left + width && icon.bottom <= top + height
}

const HINT_PAGES: [SettingsPage; 6] = [
    SettingsPage::Overview,
    SettingsPage::Autocorrection,
    SettingsPage::Hotkeys,
    SettingsPage::Exclusions,
    SettingsPage::System,
    SettingsPage::History,
];

/// The page of the hints at `index` of the settings window, if the hints have that page.
#[must_use]
pub fn settings_page(index: usize) -> Option<SettingsPage> {
    let (name, _) = PAGE_NAMES.get(index)?;
    HINT_PAGES.into_iter().find(|page| page.window_page() == *name)
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;

    const SCREEN: (i32, i32, i32, i32) = (0, 0, 1920, 1080);
    const SIZE: (i32, i32) = (460, 120);

    fn metrics() -> CardMetrics {
        CardMetrics { width: 460, pad_x: 16, pad_y: 12, line_gap: 6, link_gap: 18, title_height: 18, text_height: 40, link_height: 16, ok_width: 60, never_width: 200 }
    }

    #[test]
    fn a_card_has_its_title_text_and_links_in_this_order() {
        let layout = card_layout(&metrics());
        assert_eq!(layout.size, (460, 2 + 2 * 12 + 18 + 6 + 40 + 6 + 16));
        assert_eq!(layout.title, Area { left: 17, top: 13, right: 443, bottom: 31 });
        assert_eq!(layout.text, Area { left: 17, top: 37, right: 443, bottom: 77 });
        assert_eq!(layout.ok, Area { left: 17, top: 83, right: 77, bottom: 99 });
        assert_eq!(layout.never, Area { left: 95, top: 83, right: 295, bottom: 99 });
        assert_eq!(card_text_width(460, 16), 426);
        assert_eq!(link_row(&metrics(), layout.size.1), (layout.ok, layout.never));
    }

    #[test]
    fn a_click_means_the_link_it_landed_on_and_the_text_only_when_it_leads_somewhere() {
        let layout = card_layout(&metrics());
        assert_eq!(card_hit(&layout, 17, 83, true), Some(Click::Ok));
        assert_eq!(card_hit(&layout, 76, 98, false), Some(Click::Ok));
        assert_eq!(card_hit(&layout, 95, 83, false), Some(Click::Never));
        assert_eq!(card_hit(&layout, 77, 90, true), None, "the gap between the links is nothing");
        assert_eq!(card_hit(&layout, 100, 50, true), Some(Click::Text));
        assert_eq!(card_hit(&layout, 100, 50, false), None);
        assert_eq!(card_hit(&layout, 100, 20, true), None, "the title is nothing");
        assert_eq!(card_hit(&layout, 5, 5, true), None);
    }

    #[test]
    fn an_area_knows_what_is_inside_it_and_whether_it_is_empty() {
        let area = Area { left: 10, top: 20, right: 30, bottom: 40 };
        assert!(area.contains(10, 20) && area.contains(29, 39));
        assert!(!area.contains(30, 39) && !area.contains(29, 40) && !area.contains(9, 25) && !area.contains(25, 19));
        assert!(!area.is_empty());
        assert!(Area::default().is_empty());
        assert!(Area { left: 10, top: 0, right: 10, bottom: 5 }.is_empty());
        assert!(Area { left: 10, top: 5, right: 12, bottom: 5 }.is_empty());
    }

    #[test]
    fn a_card_goes_above_what_it_is_for_and_below_at_the_top_of_the_screen() {
        let caret = Area { left: 960, top: 500, right: 960, bottom: 500 };
        assert_eq!(card_position(caret, SIZE, SCREEN, 12, 8), (730, 500 - 120 - 12));
        let high = Area { left: 960, top: 50, right: 960, bottom: 70 };
        assert_eq!(card_position(high, SIZE, SCREEN, 12, 8), (730, 70 + 12), "no room above");
        let edge = Area { left: 1900, top: 900, right: 1900, bottom: 900 };
        assert_eq!(card_position(edge, SIZE, SCREEN, 12, 8), (1920 - 460 - 8, 900 - 120 - 12), "kept on the screen");
    }

    #[test]
    fn a_card_is_kept_on_a_screen_too_small_for_either_side() {
        let short = (0, 0, 800, 150);
        let target = Area { left: 400, top: 40, right: 400, bottom: 60 };
        assert_eq!(card_position(target, SIZE, short, 12, 8), (170, 22), "above would be off the top");
    }

    #[test]
    fn the_icon_counts_when_it_has_an_area_on_the_screen() {
        let icon = Area { left: 1800, top: 1040, right: 1824, bottom: 1064 };
        assert!(icon_is_on_screen(icon, SCREEN));
        assert!(!icon_is_on_screen(Area::default(), SCREEN));
        assert!(!icon_is_on_screen(Area { left: 2000, top: 1040, right: 2024, bottom: 1064 }, SCREEN));
        assert!(!icon_is_on_screen(Area { left: 1800, top: 1070, right: 1824, bottom: 1090 }, SCREEN));
        assert!(!icon_is_on_screen(Area { left: -30, top: 1040, right: -6, bottom: 1064 }, SCREEN));
        assert!(!icon_is_on_screen(Area { left: 1800, top: -40, right: 1824, bottom: -16 }, SCREEN));
    }

    #[test]
    fn the_pages_of_the_hints_are_the_pages_of_the_window() {
        assert_eq!(settings_page(0), Some(SettingsPage::Overview));
        assert_eq!(settings_page(1), Some(SettingsPage::Autocorrection));
        assert_eq!(settings_page(2), None, "the layouts page has no hints");
        assert_eq!(settings_page(3), Some(SettingsPage::Hotkeys));
        assert_eq!(settings_page(4), Some(SettingsPage::Exclusions));
        assert_eq!(settings_page(5), Some(SettingsPage::System));
        assert_eq!(settings_page(6), Some(SettingsPage::History));
        assert_eq!(settings_page(PAGE_NAMES.len()), None);
        for page in HINT_PAGES {
            assert!(PAGE_NAMES.iter().any(|(name, _)| *name == page.window_page()));
        }
    }
}
