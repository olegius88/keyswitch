//! Where a hint goes on a Mac and how its buttons are told apart: plain arithmetic and the accessibility questions, kept
//! apart from `AppKit` so that every system tests them. `hint_panel` and the settings window draw what this decides.
//!
//! Two coordinate systems meet here. The accessibility tree counts from the top left corner of the primary display, with
//! y growing down (the coordinates of Quartz); `AppKit` counts from its bottom left corner, with y growing up. A rectangle
//! read from the tree is turned into `AppKit`'s by [`appkit_frame`] when it is drawn, on the main thread, the only one
//! that may ask `AppKit` how tall the primary display is.

use ks_ui::hints::{Click, SettingsPage};

use crate::constants::{
    HINT_BUTTON_HEIGHT_POINTS, HINT_BUTTON_ROW_GAP_POINTS, HINT_CARD_PADDING_POINTS, HINT_CLICK_NEVER, HINT_CLICK_OK, HINT_CLICK_TEXT, HINT_CLICKS_PER_SLOT,
    HINT_FIRST_PAGE_SLOT, HINT_PANEL_GAP_POINTS, HINT_SCREEN_MARGIN_POINTS,
};
use crate::macos_context::{AccessibilityAPI, SECURE_TEXT_FIELD_SUBROLE, SELECTED_RANGE_ATTRIBUTE, SUBROLE_ATTRIBUTE};

/// The parameterized attribute of the rectangle of a range of text (`kAXBoundsForRangeParameterizedAttribute`).
pub const BOUNDS_FOR_RANGE_ATTRIBUTE: &str = "AXBoundsForRange";

/// A rectangle of the screen: its corner and its size, in the coordinates of whoever produced it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Frame {
    #[must_use]
    pub fn right(&self) -> f64 {
        self.x + self.width
    }

    #[must_use]
    pub fn top(&self) -> f64 {
        self.y + self.height
    }

    /// A rectangle that covers something: an empty one says nothing about where the text is.
    #[must_use]
    pub fn covers_area(&self) -> bool {
        self.width > 0.0 && self.height > 0.0
    }

    fn holds(&self, x: f64, y: f64) -> bool {
        x >= self.x && x < self.right() && y >= self.y && y < self.top()
    }
}

/// A display as `AppKit` gives it: all of it, and the part menus and the Dock leave free.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Display {
    pub frame: Frame,
    pub visible: Frame,
}

/// Where the caret of the field typed in was found.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CaretAnchor {
    /// The caret's rectangle, in the coordinates of the accessibility tree.
    At(Frame),
    /// The field does not say where its caret is: the hint goes under the menu bar item instead.
    Unknown,
}

/// The accessibility call the caret needs besides those the reader of fields already makes.
pub trait CaretAccessibilityAPI: AccessibilityAPI {
    /// The rectangle of `length` UTF-16 units at `location`, in the coordinates of the accessibility tree, when the field
    /// gives one.
    fn bounds_for_range(&self, element: usize, location: i64, length: i64) -> Option<Frame>;
}

/// Where the caret of the focused field is. Only rectangles are asked for, never the text, and nothing at all of a
/// password field.
///
/// A range of no length cannot be asked about: `TextEdit` on macOS 13 answers it with an empty rectangle at the bottom of
/// the screen, and with an error for one past the text. So this asks for the character before the caret, and for the one
/// after it when the caret stands at the very start. Their left edge is off by at most one character.
pub fn caret_anchor<A: CaretAccessibilityAPI>(api: &A) -> CaretAnchor {
    let element = api.focused_element();
    if element == 0 {
        return CaretAnchor::Unknown;
    }
    let found = if api.string_attribute(element, SUBROLE_ATTRIBUTE).as_deref() == Some(SECURE_TEXT_FIELD_SUBROLE) {
        None
    } else {
        caret_of(api, element)
    };
    api.release(element);
    found.map_or(CaretAnchor::Unknown, CaretAnchor::At)
}

fn caret_of<A: CaretAccessibilityAPI>(api: &A, element: usize) -> Option<Frame> {
    let (location, length) = api.range_attribute(element, SELECTED_RANGE_ATTRIBUTE)?;
    let end = location + length;
    [end - 1, end].into_iter().filter(|at| *at >= 0).find_map(|at| api.bounds_for_range(element, at, 1).filter(Frame::covers_area))
}

/// A rectangle of the accessibility tree in `AppKit`'s coordinates.
#[must_use]
pub fn appkit_frame(frame: Frame, primary_height: f64) -> Frame {
    Frame { y: primary_height - frame.y - frame.height, ..frame }
}

/// The display that shows the point `x`, `y`, or the first one (the primary) when none does.
#[must_use]
pub fn display_at(displays: &[Display], x: f64, y: f64) -> Option<Display> {
    displays.iter().copied().find(|display| display.frame.holds(x, y)).or_else(|| displays.first().copied())
}

/// The lower bound wins when the screen is too small, as `prompt_position` of `ks-ui` does.
fn inside(value: f64, low: f64, high: f64) -> f64 {
    value.min(high).max(low)
}

fn across(center: f64, width: f64, screen: Frame) -> f64 {
    inside(center - width / 2.0, screen.x + HINT_SCREEN_MARGIN_POINTS, screen.right() - width - HINT_SCREEN_MARGIN_POINTS)
}

/// Where the lower left corner of a panel of `size` goes: above the caret, centred on it, or under the line of the caret
/// when no room is left above it. Everything is in `AppKit`'s coordinates.
#[must_use]
pub fn origin_above_caret(caret: Frame, size: (f64, f64), screen: Frame) -> (f64, f64) {
    let (width, height) = size;
    let above = caret.top() + HINT_PANEL_GAP_POINTS;
    let y = if above + height <= screen.top() - HINT_SCREEN_MARGIN_POINTS { above } else { caret.y - HINT_PANEL_GAP_POINTS - height };
    (across(caret.x, width, screen), inside(y, screen.y + HINT_SCREEN_MARGIN_POINTS, screen.top() - height - HINT_SCREEN_MARGIN_POINTS))
}

/// Where the lower left corner of a panel of `size` goes under the menu bar item whose window is `item`, centred on it.
#[must_use]
pub fn origin_under_item(item: Frame, size: (f64, f64), screen: Frame) -> (f64, f64) {
    let (width, height) = size;
    (
        across(item.x + item.width / 2.0, width, screen),
        inside(item.y - HINT_PANEL_GAP_POINTS - height, screen.y + HINT_SCREEN_MARGIN_POINTS, screen.top() - height - HINT_SCREEN_MARGIN_POINTS),
    )
}

/// Where the panel goes without a menu bar item (the setting that shows it is off): the corner the item would be near.
#[must_use]
pub fn origin_in_corner(size: (f64, f64), screen: Frame) -> (f64, f64) {
    (screen.right() - size.0 - HINT_SCREEN_MARGIN_POINTS, screen.top() - size.1 - HINT_SCREEN_MARGIN_POINTS)
}

/// A card's rows in a view that counts y up from its bottom: the text over a row of buttons.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CardLayout {
    pub height: f64,
    pub text_y: f64,
    pub buttons_y: f64,
}

#[must_use]
pub fn card_layout(text_height: f64) -> CardLayout {
    let buttons_y = HINT_CARD_PADDING_POINTS;
    let text_y = buttons_y + HINT_BUTTON_HEIGHT_POINTS + HINT_BUTTON_ROW_GAP_POINTS;
    CardLayout { height: text_y + text_height + HINT_CARD_PADDING_POINTS, text_y, buttons_y }
}

/// The slot of a card on the page `index` of the settings window; the panel's is `PANEL_SLOT`.
#[must_use]
pub fn page_slot(index: isize) -> isize {
    HINT_FIRST_PAGE_SLOT + index
}

/// The tag a button of `slot` carries. A close by the system has no button.
#[must_use]
pub fn tag_of(slot: isize, click: Click) -> isize {
    let code = match click {
        Click::Ok | Click::Closed => HINT_CLICK_OK,
        Click::Never => HINT_CLICK_NEVER,
        Click::Text => HINT_CLICK_TEXT,
    };
    slot * HINT_CLICKS_PER_SLOT + code
}

/// The slot and the click a button's tag says.
#[must_use]
pub fn click_of(tag: isize) -> (isize, Click) {
    let click = match tag.rem_euclid(HINT_CLICKS_PER_SLOT) {
        HINT_CLICK_NEVER => Click::Never,
        HINT_CLICK_TEXT => Click::Text,
        _ => Click::Ok,
    };
    (tag.div_euclid(HINT_CLICKS_PER_SLOT), click)
}

/// The hints' name for a page of the settings window; a page no hint is about has none.
#[must_use]
pub fn settings_page(identifier: &str) -> Option<SettingsPage> {
    [
        SettingsPage::Overview,
        SettingsPage::Autocorrection,
        SettingsPage::Hotkeys,
        SettingsPage::Exclusions,
        SettingsPage::System,
        SettingsPage::History,
    ]
    .into_iter()
    .find(|page| page.window_page() == identifier)
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;
    use crate::constants::PANEL_SLOT;
    use std::cell::RefCell;
    use std::collections::HashMap;

    const FRAME: Frame = Frame { x: 343.0, y: 132.0, width: 6.6, height: 13.0 };
    const SCREEN: Frame = Frame { x: 0.0, y: 47.0, width: 2560.0, height: 1008.0 };
    const PANEL: (f64, f64) = (460.0, 120.0);

    #[derive(Default)]
    struct Fake {
        focused: usize,
        strings: HashMap<String, String>,
        range: Option<(i64, i64)>,
        rectangles: HashMap<i64, Frame>,
        asked: RefCell<Vec<(i64, i64)>>,
        released: RefCell<Vec<usize>>,
    }

    impl AccessibilityAPI for Fake {
        fn focused_element(&self) -> usize {
            self.focused
        }
        fn string_attribute(&self, _element: usize, name: &str) -> Option<String> {
            self.strings.get(name).cloned()
        }
        fn range_attribute(&self, _element: usize, name: &str) -> Option<(i64, i64)> {
            assert_eq!(name, SELECTED_RANGE_ATTRIBUTE);
            self.range
        }
        fn release(&self, element: usize) {
            self.released.borrow_mut().push(element);
        }
    }

    impl CaretAccessibilityAPI for Fake {
        fn bounds_for_range(&self, element: usize, location: i64, length: i64) -> Option<Frame> {
            assert_eq!(element, self.focused);
            self.asked.borrow_mut().push((location, length));
            self.rectangles.get(&location).copied()
        }
    }

    fn same(first: f64, second: f64) -> bool {
        (first - second).abs() < 1e-9
    }

    fn field(range: Option<(i64, i64)>, rectangles: &[(i64, Frame)]) -> Fake {
        Fake { focused: 7, range, rectangles: rectangles.iter().copied().collect(), ..Fake::default() }
    }

    #[test]
    fn the_caret_is_the_rectangle_of_the_character_before_it() {
        let tree = field(Some((29, 0)), &[(28, FRAME)]);
        assert_eq!(caret_anchor(&tree), CaretAnchor::At(FRAME));
        assert_eq!(*tree.asked.borrow(), [(28, 1)]);
        assert_eq!(*tree.released.borrow(), [7]);
    }

    #[test]
    fn at_the_start_of_the_text_the_next_character_says_where_it_is() {
        let tree = field(Some((0, 0)), &[(0, FRAME)]);
        assert_eq!(caret_anchor(&tree), CaretAnchor::At(FRAME));
        assert_eq!(*tree.asked.borrow(), [(0, 1)]);
    }

    #[test]
    fn a_selection_counts_from_its_end() {
        let tree = field(Some((3, 4)), &[(6, FRAME)]);
        assert_eq!(caret_anchor(&tree), CaretAnchor::At(FRAME));
        assert_eq!(*tree.asked.borrow(), [(6, 1)]);
    }

    #[test]
    fn an_empty_rectangle_is_the_same_as_no_answer() {
        let empty = Frame { x: 0.0, y: 1080.0, width: 0.0, height: 0.0 };
        let tree = field(Some((29, 0)), &[(28, empty), (29, Frame { width: 1.0, height: 0.0, ..empty })]);
        assert_eq!(caret_anchor(&tree), CaretAnchor::Unknown);
        assert_eq!(*tree.asked.borrow(), [(28, 1), (29, 1)]);
        assert_eq!(*tree.released.borrow(), [7]);
    }

    #[test]
    fn with_nothing_focused_or_no_range_the_caret_is_not_known() {
        let tree = Fake::default();
        assert_eq!(caret_anchor(&tree), CaretAnchor::Unknown);
        assert!(tree.released.borrow().is_empty());
        let tree = field(None, &[(28, FRAME)]);
        assert_eq!(caret_anchor(&tree), CaretAnchor::Unknown);
        assert!(tree.asked.borrow().is_empty());
        assert_eq!(*tree.released.borrow(), [7]);
    }

    #[test]
    fn a_password_field_is_not_asked_about_its_caret() {
        let mut tree = field(Some((29, 0)), &[(28, FRAME)]);
        tree.strings.insert(SUBROLE_ATTRIBUTE.into(), SECURE_TEXT_FIELD_SUBROLE.into());
        assert_eq!(caret_anchor(&tree), CaretAnchor::Unknown);
        assert!(tree.asked.borrow().is_empty());
        assert_eq!(*tree.released.borrow(), [7]);
        let mut other = field(Some((29, 0)), &[(28, FRAME)]);
        other.strings.insert(SUBROLE_ATTRIBUTE.into(), "AXSearchField".into());
        assert_eq!(caret_anchor(&other), CaretAnchor::At(FRAME));
    }

    #[test]
    fn the_coordinates_of_the_tree_turn_into_those_of_appkit() {
        assert_eq!(appkit_frame(FRAME, 1080.0), Frame { y: 1080.0 - 132.0 - 13.0, ..FRAME });
        assert!(same(appkit_frame(Frame { x: 1.0, y: 1080.0, width: 2.0, height: 0.0 }, 1080.0).y, 0.0));
    }

    fn display(x: f64, width: f64) -> Display {
        let frame = Frame { x, y: 0.0, width, height: 1080.0 };
        Display { frame, visible: Frame { y: 47.0, height: 1008.0, ..frame } }
    }

    #[test]
    fn the_display_of_a_point_is_the_one_that_shows_it_and_else_the_primary() {
        let (primary, second) = (display(0.0, 2560.0), display(2560.0, 1920.0));
        let both = [primary, second];
        assert_eq!(display_at(&both, 3000.0, 500.0), Some(second));
        assert_eq!(display_at(&both, 100.0, 500.0), Some(primary));
        assert_eq!(display_at(&both, 9000.0, 500.0), Some(primary));
        assert_eq!(display_at(&both, 100.0, 5000.0), Some(primary));
        assert_eq!(display_at(&both, 100.0, -1.0), Some(primary));
        assert_eq!(display_at(&both, -1.0, 500.0), Some(primary));
        assert_eq!(display_at(&[], 1.0, 1.0), None);
    }

    #[test]
    fn a_hint_stands_above_the_caret_and_centred_on_it() {
        let caret = appkit_frame(Frame { y: 400.0, ..FRAME }, 1080.0);
        let (x, y) = origin_above_caret(caret, PANEL, SCREEN);
        assert!(same(x, 343.0 - 230.0));
        assert!(same(y, caret.top() + HINT_PANEL_GAP_POINTS));
    }

    #[test]
    fn at_the_edges_of_the_screen_it_stays_inside_and_goes_under_the_line_of_the_caret() {
        let near_top = Frame { x: 10.0, y: 1000.0, width: 7.0, height: 13.0 };
        let (x, y) = origin_above_caret(near_top, PANEL, SCREEN);
        assert!(same(x, SCREEN.x + HINT_SCREEN_MARGIN_POINTS));
        assert!(same(y, near_top.y - HINT_PANEL_GAP_POINTS - PANEL.1));
        let near_right = Frame { x: 2550.0, y: 500.0, width: 7.0, height: 13.0 };
        assert!(same(origin_above_caret(near_right, PANEL, SCREEN).0, SCREEN.right() - PANEL.0 - HINT_SCREEN_MARGIN_POINTS));
        // A caret on the lowest line of the screen: under it there is no room, the bottom margin holds.
        let squeezed = Frame { x: 500.0, y: SCREEN.y - 200.0, width: 7.0, height: 13.0 };
        assert!(same(origin_above_caret(squeezed, (460.0, 900.0), Frame { y: SCREEN.y, height: 300.0, ..SCREEN }).1, SCREEN.y + HINT_SCREEN_MARGIN_POINTS));
    }

    #[test]
    fn near_the_menu_bar_item_a_hint_stands_under_it_and_centred() {
        let item = Frame { x: 2155.0, y: 900.0, width: 40.0, height: 24.0 };
        let (x, y) = origin_under_item(item, PANEL, SCREEN);
        assert!(same(x, 2175.0 - 230.0));
        assert!(same(y, 900.0 - HINT_PANEL_GAP_POINTS - PANEL.1));
        // The menu bar's own item stands above the free part of the screen, and the panel keeps inside that part.
        let in_the_bar = Frame { y: 1056.0, ..item };
        assert!(same(origin_under_item(in_the_bar, PANEL, SCREEN).1, SCREEN.top() - PANEL.1 - HINT_SCREEN_MARGIN_POINTS));
        let right = Frame { x: 2500.0, ..item };
        assert!(same(origin_under_item(right, PANEL, SCREEN).0, SCREEN.right() - PANEL.0 - HINT_SCREEN_MARGIN_POINTS));
        let tall = origin_under_item(item, (460.0, 1200.0), SCREEN);
        assert!(same(tall.1, SCREEN.y + HINT_SCREEN_MARGIN_POINTS));
    }

    #[test]
    fn with_no_item_a_hint_goes_to_the_corner_the_item_would_be_near() {
        assert_eq!(origin_in_corner(PANEL, SCREEN), (SCREEN.right() - PANEL.0 - HINT_SCREEN_MARGIN_POINTS, SCREEN.top() - PANEL.1 - HINT_SCREEN_MARGIN_POINTS));
    }

    #[test]
    fn a_card_is_the_text_over_its_buttons_inside_the_padding() {
        let layout = card_layout(40.0);
        assert!(same(layout.buttons_y, HINT_CARD_PADDING_POINTS));
        assert!(same(layout.text_y, HINT_CARD_PADDING_POINTS + HINT_BUTTON_HEIGHT_POINTS + HINT_BUTTON_ROW_GAP_POINTS));
        assert!(same(layout.height, layout.text_y + 40.0 + HINT_CARD_PADDING_POINTS));
    }

    #[test]
    fn a_tag_says_its_card_and_its_button() {
        for slot in [PANEL_SLOT, page_slot(0), page_slot(5)] {
            for click in [Click::Ok, Click::Never, Click::Text] {
                assert_eq!(click_of(tag_of(slot, click)), (slot, click));
            }
        }
        assert_eq!(tag_of(PANEL_SLOT, Click::Closed), tag_of(PANEL_SLOT, Click::Ok));
        assert_eq!(click_of(tag_of(page_slot(5), Click::Never)).0, 6);
        assert_eq!(click_of(-1), (-1, Click::Text));
    }

    #[test]
    fn every_page_a_hint_is_about_has_its_name_and_no_other() {
        for page in [SettingsPage::Overview, SettingsPage::Autocorrection, SettingsPage::Hotkeys, SettingsPage::Exclusions, SettingsPage::System, SettingsPage::History] {
            assert_eq!(settings_page(page.window_page()), Some(page));
        }
        assert_eq!(settings_page("appearance"), Some(SettingsPage::System));
        assert_eq!(settings_page("about"), None);
    }
}
