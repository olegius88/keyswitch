//! When a hint shows. One rule set, with time and the typing handed in, so that every rule is tested without a desktop:
//!
//! - a hint shows only while `general.hints` is on, on its platforms, and once (the news of an update once per version);
//! - one at a time: while one is on screen, the others wait;
//! - at the caret only once the typing stopped (`HINT_TYPING_SETTLE_SECONDS` without a key press), waiting at most
//!   `HINT_SETTLE_WAIT_MAX_SECONDS`, and never while a rule prompt is open: a prompt that opens takes such a hint away;
//! - `HINT_QUIET_SECONDS` after the hint shown last, and at most `HINTS_PER_DAY_MAX` in a calendar day; the cards in the
//!   settings window, shown only while the user looks at a page, at most one a page each time the window opens, are not
//!   counted;
//! - when several are due, the one first in the catalogue goes first.
//!
//! A hint is seen once it is drawn. One that could not wait for its moment stays unseen until the next event of its kind.

use ks_updates::SemanticVersion;
use std::time::Instant;

use ks_config::history::utc_timestamp_now;

use crate::constants::{
    CARET_FALLBACK_AFTER_ATTEMPTS, HINT_CARET_TIMEOUT_SECONDS, HINT_FIRST_START_DELAY_SECONDS, HINT_QUIET_SECONDS,
    HINT_RULE_PROMPT_FOUND_SECONDS, HINT_SETTLE_WAIT_MAX_SECONDS, HINT_TRAY_TIMEOUT_SECONDS, HINT_TYPING_SETTLE_SECONDS,
    HINT_WHATS_NEW_DELAY_SECONDS, HINTS_PER_DAY_MAX, TIMESTAMP_DATE_CHARACTERS,
};
use super::catalogue::{
    Condition, CorrectionInfo, Counted, HINT_NEVER_LABEL, HINT_OK_LABEL, HINT_TITLE, Hint, Hotkeys, MODE_MANUAL, Opens, Place, Platform,
    SettingsPage, Trigger, Wording,
};
use super::store::{Counter, HintState, How};

/// The settings the hints follow.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HintSettings {
    /// `general.hints`.
    pub enabled: bool,
    /// `general.notifications` and `general.keep_history`: the hints about them wait for them to be on.
    pub notifications: bool,
    pub keep_history: bool,
    pub hotkeys: Hotkeys,
}

/// The time of a call: monotonic seconds, the UTC timestamp the file records, and the key presses the engine has
/// counted (`Shared::typed_presses`), by which the hints see whether the user is typing.
#[derive(Clone, Debug, PartialEq)]
pub struct Moment {
    pub seconds: f64,
    pub timestamp: String,
    pub typed_presses: i64,
}

impl Moment {
    /// Now, counted from `origin`.
    #[must_use]
    pub fn now(origin: Instant, typed_presses: i64) -> Moment {
        Moment { seconds: origin.elapsed().as_secs_f64(), timestamp: utc_timestamp_now(), typed_presses }
    }

    /// The calendar day the daily limit counts.
    #[must_use]
    pub fn day(&self) -> &str {
        self.timestamp.get(..TIMESTAMP_DATE_CHARACTERS).unwrap_or(&self.timestamp)
    }
}

/// A hint to draw, with everything its card says.
#[derive(Clone, Debug, PartialEq)]
pub struct HintView {
    pub id: &'static str,
    pub place: Place,
    pub title: &'static str,
    pub text: String,
    /// The two buttons: «Понятно» and «Больше не показывать подсказки».
    pub ok: &'static str,
    pub never: &'static str,
    /// What a click on the text opens.
    pub opens: Option<Opens>,
    index: usize,
}

/// What a platform does now.
#[derive(Clone, Debug, PartialEq)]
pub enum Step<A> {
    /// Draw above the caret at the anchor.
    Caret(HintView, A),
    /// Draw near the tray icon.
    Tray(HintView),
    /// Take down whatever shows this hint.
    Hide(&'static str),
}

#[derive(Clone, Debug)]
struct Pending<A> {
    index: usize,
    place: Place,
    anchor: Option<A>,
    queued_at: f64,
    not_before: f64,
    presses: i64,
    presses_at: f64,
}

#[derive(Clone, Copy, Debug)]
struct Visible {
    id: &'static str,
    caret: bool,
    until: f64,
}

pub struct Scheduler<A> {
    catalogue: &'static [Hint],
    platform: Platform,
    version: SemanticVersion,
    settings: HintSettings,
    state: HintState,
    dirty: bool,
    pending: Vec<Pending<A>>,
    visible: Option<Visible>,
    last_shown_at: Option<f64>,
    prompt_open: bool,
    engine_running: bool,
    last_manual_at: Option<f64>,
    caret_failures: i64,
    cards: Vec<SettingsPage>,
}

impl<A: Clone> Scheduler<A> {
    /// The hints of a run that began at `moment`, with `state` as the start left it (`HintState::begin`). The news of an
    /// update waits `HINT_WHATS_NEW_DELAY_SECONDS`.
    #[must_use]
    pub fn new(catalogue: &'static [Hint], platform: Platform, version: SemanticVersion, settings: HintSettings, state: HintState, moment: &Moment) -> Scheduler<A> {
        let mut scheduler = Scheduler {
            catalogue,
            platform,
            version,
            settings,
            state,
            dirty: false,
            pending: Vec::new(),
            visible: None,
            last_shown_at: None,
            prompt_open: false,
            engine_running: false,
            last_manual_at: None,
            caret_failures: 0,
            cards: Vec::new(),
        };
        let not_before = moment.seconds + HINT_WHATS_NEW_DELAY_SECONDS;
        for (index, hint) in catalogue.iter().enumerate() {
            if matches!(hint.trigger, Trigger::WhatsNew | Trigger::Updated) && scheduler.may_show(hint) && scheduler.after_update(hint) {
                scheduler.queue(index, Place::Tray, None, moment, not_before);
            }
        }
        scheduler
    }

    #[must_use]
    pub fn state(&self) -> &HintState {
        &self.state
    }

    /// Whether the state changed since the last call: then it is to be saved.
    pub fn take_dirty(&mut self) -> bool {
        std::mem::take(&mut self.dirty)
    }

    #[must_use]
    pub fn settings(&self) -> &HintSettings {
        &self.settings
    }

    /// Whether a platform should look for the caret when a correction is made: some hint at the caret may still show.
    #[must_use]
    pub fn wants_caret_anchor(&self) -> bool {
        self.catalogue.iter().any(|hint| hint.places.contains(&Place::Caret) && self.may_show(hint))
    }

    /// Whether `tick` has anything to do: a hint waits or is on screen.
    #[must_use]
    pub fn needs_tick(&self) -> bool {
        self.visible.is_some() || !self.pending.is_empty()
    }

    fn seen(&self, hint: &Hint) -> bool {
        if hint.trigger == Trigger::Updated {
            return self.state.seen_in(hint.id) == Some(self.version.to_string());
        }
        self.state.is_seen(hint.id)
    }

    /// The rules of every place: hints on, this platform, not seen.
    fn may_show(&self, hint: &Hint) -> bool {
        self.settings.enabled && hint.shows_on(self.platform) && !self.seen(hint)
    }

    /// Whether the news of `hint` is for this run: the last update went to this version, from one before the hint's.
    fn after_update(&self, hint: &Hint) -> bool {
        let Some((from, to)) = self.state.update() else { return false };
        if to != self.version {
            return false;
        }
        match hint.release() {
            Some(release) => from.is_none_or(|from| from < release) && release <= self.version,
            None => true,
        }
    }

    fn holds(&self, condition: Condition) -> bool {
        match condition {
            Condition::NoManualConversion => self.state.counter(Counter::ManualConversions) == 0,
            Condition::Notifications => self.settings.notifications,
            Condition::KeepHistory => self.settings.keep_history,
        }
    }

    /// How far the counters go: as far as a hint that may still show waits for, and no further, so that the file stops
    /// changing with every correction.
    fn ceiling(&self) -> Counted {
        self.catalogue.iter().filter(|hint| self.may_show(hint)).map(Hint::needs).fold(Counted::default(), |most, needs| Counted {
            automatic_corrections: most.automatic_corrections.max(needs.automatic_corrections),
            manual_conversions: most.manual_conversions.max(needs.manual_conversions),
        })
    }

    fn queue(&mut self, index: usize, place: Place, anchor: Option<A>, moment: &Moment, not_before: f64) {
        let entry = Pending {
            index,
            place,
            anchor,
            queued_at: moment.seconds,
            not_before,
            presses: moment.typed_presses,
            presses_at: moment.seconds,
        };
        self.pending.retain(|pending| pending.index != index);
        self.pending.push(entry);
    }

    /// A hint at the caret with no place to go: it waits for another chance, and after
    /// `CARET_FALLBACK_AFTER_ATTEMPTS` such failures running it shows near the tray icon.
    fn caret_missed(&mut self, index: usize, moment: &Moment) {
        self.caret_failures += 1;
        if self.caret_failures >= CARET_FALLBACK_AFTER_ATTEMPTS {
            self.queue(index, Place::Tray, None, moment, moment.seconds);
        }
    }

    /// The first snapshot of a running engine: a new installation is told where the program is.
    pub fn engine_running(&mut self, moment: &Moment) {
        if self.engine_running {
            return;
        }
        self.engine_running = true;
        if !self.state.fresh_install() {
            return;
        }
        let catalogue = self.catalogue;
        for (index, hint) in catalogue.iter().enumerate() {
            if hint.trigger == Trigger::FirstStart && self.may_show(hint) {
                self.queue(index, Place::Tray, None, moment, moment.seconds + HINT_FIRST_START_DELAY_SECONDS);
            }
        }
    }

    /// A correction the engine made; `anchor` is where the caret was, looked for on the engine's thread.
    pub fn correction(&mut self, correction: CorrectionInfo, anchor: Option<A>, moment: &Moment) {
        if correction.mode == MODE_MANUAL {
            self.last_manual_at = Some(moment.seconds);
        }
        let (counted, ceiling) = (self.state.counted(), self.ceiling());
        if correction.automatic && counted.automatic_corrections < ceiling.automatic_corrections {
            self.state.count(Counter::AutomaticCorrections);
            self.dirty = true;
        }
        if correction.manual() && counted.manual_conversions < ceiling.manual_conversions {
            self.state.count(Counter::ManualConversions);
            self.dirty = true;
        }
        let counted = self.state.counted();
        let catalogue = self.catalogue;
        for (index, hint) in catalogue.iter().enumerate() {
            // A hint on screen was seen when it was drawn, so it is not set off again here.
            if !self.may_show(hint) || !hint.fired_by(correction, counted, |condition| self.holds(condition)) {
                continue;
            }
            match (hint.shown_at(), anchor.clone()) {
                (Some(Place::Caret), None) => self.caret_missed(index, moment),
                (Some(place), anchor) => self.queue(index, place, anchor, moment, moment.seconds),
                (None, _) => {}
            }
        }
    }

    /// The rule prompt shown or gone. One shown right after a manual conversion means the user found the double press
    /// alone; a hint at the caret on screen gives way to it.
    pub fn prompt(&mut self, shown: bool, moment: &Moment) -> Option<Step<A>> {
        self.prompt_open = shown;
        if !shown {
            return None;
        }
        let found = self.last_manual_at.is_some_and(|at| moment.seconds - at <= HINT_RULE_PROMPT_FOUND_SECONDS);
        if found {
            let catalogue = self.catalogue;
            for (index, hint) in catalogue.iter().enumerate() {
                if hint.trigger == Trigger::ManualConversion && self.may_show(hint) {
                    self.state.see(hint.id, self.version, &moment.timestamp, How::Skipped);
                    self.pending.retain(|pending| pending.index != index);
                    self.dirty = true;
                }
            }
        }
        match self.visible {
            Some(visible) if visible.caret => {
                self.visible = None;
                Some(Step::Hide(visible.id))
            }
            _ => None,
        }
    }

    /// The settings changed. Hints turned off take down what shows and forget what waits.
    pub fn set_settings(&mut self, settings: HintSettings) -> Option<Step<A>> {
        let turned_off = self.settings.enabled && !settings.enabled;
        self.settings = settings;
        if !turned_off {
            return None;
        }
        self.pending.clear();
        self.visible.take().map(|visible| Step::Hide(visible.id))
    }

    /// The settings window opened: each page may show a card again.
    pub fn settings_opened(&mut self) {
        self.cards.clear();
    }

    /// The card for `page` of the settings window, now being built: it is seen once asked for.
    pub fn settings_card(&mut self, page: SettingsPage, moment: &Moment) -> Option<HintView> {
        if self.cards.contains(&page) {
            return None;
        }
        let catalogue = self.catalogue;
        for (index, hint) in catalogue.iter().enumerate() {
            let Some(place) = hint.card_on(page) else { continue };
            let due = match hint.trigger {
                Trigger::SettingsPage => true,
                Trigger::WhatsNew => self.after_update(hint),
                Trigger::FirstStart
                | Trigger::LayoutCorrection
                | Trigger::Mode(_)
                | Trigger::ManualConversion
                | Trigger::AutomaticCorrections { .. }
                | Trigger::ManualConversions { .. }
                | Trigger::Updated => false,
            };
            if !due || !self.may_show(hint) {
                continue;
            }
            let Some(view) = self.view(index, place) else { continue };
            self.state.see(hint.id, self.version, &moment.timestamp, How::Shown);
            self.pending.retain(|pending| pending.index != index);
            self.cards.push(page);
            self.dirty = true;
            return Some(view);
        }
        None
    }

    fn view(&self, index: usize, place: Place) -> Option<HintView> {
        let hint = &self.catalogue[index];
        let wording = Wording { platform: self.platform, version: self.version, hotkeys: &self.settings.hotkeys };
        Some(HintView {
            id: hint.id,
            place,
            title: HINT_TITLE,
            text: hint.text(&wording)?,
            ok: HINT_OK_LABEL,
            never: HINT_NEVER_LABEL,
            opens: hint.opens.map(|destination| destination.resolve(self.version)),
            index,
        })
    }

    /// What a click on the text of `id` opens.
    #[must_use]
    pub fn opens_of(&self, id: &str) -> Option<Opens> {
        let hint = self.catalogue.iter().find(|hint| hint.id == id)?;
        hint.opens.map(|destination| destination.resolve(self.version))
    }

    fn room(&self, moment: &Moment) -> bool {
        let quiet = self.last_shown_at.is_none_or(|at| moment.seconds - at >= HINT_QUIET_SECONDS);
        quiet && self.state.shown_on(moment.day()) < HINTS_PER_DAY_MAX
    }

    fn ready(&self, pending: &Pending<A>, now: f64) -> bool {
        if now < pending.not_before {
            return false;
        }
        pending.place != Place::Caret || (!self.prompt_open && now - pending.presses_at >= HINT_TYPING_SETTLE_SECONDS)
    }

    /// What to draw or take down now: called every `HINT_TICK_INTERVAL_MS` while `needs_tick`, and after every event.
    /// A hint handed out to draw is reported back with `drawn` or `not_drawn`.
    pub fn tick(&mut self, moment: &Moment) -> Option<Step<A>> {
        let now = moment.seconds;
        if let Some(visible) = self.visible {
            if now < visible.until {
                return None;
            }
            self.visible = None;
            self.state.set_how(visible.id, How::Timeout);
            self.dirty = true;
            return Some(Step::Hide(visible.id));
        }
        self.pending.retain(|pending| pending.place != Place::Caret || now - pending.queued_at <= HINT_SETTLE_WAIT_MAX_SECONDS);
        for pending in &mut self.pending {
            if pending.presses != moment.typed_presses {
                pending.presses = moment.typed_presses;
                pending.presses_at = now;
            }
        }
        if !self.room(moment) {
            return None;
        }
        let chosen = (0..self.pending.len()).filter(|at| self.ready(&self.pending[*at], now)).min_by_key(|at| self.pending[*at].index)?;
        let pending = self.pending.remove(chosen);
        let view = self.view(pending.index, pending.place)?;
        match pending.anchor {
            Some(anchor) if pending.place == Place::Caret => Some(Step::Caret(view, anchor)),
            _ => Some(Step::Tray(view)),
        }
    }

    /// The platform drew `view`: it is seen, and the next one waits the quiet period.
    pub fn drawn(&mut self, view: &HintView, moment: &Moment) {
        let caret = view.place == Place::Caret;
        let timeout = if caret { HINT_CARET_TIMEOUT_SECONDS } else { HINT_TRAY_TIMEOUT_SECONDS };
        if caret {
            self.caret_failures = 0;
        }
        self.visible = Some(Visible { id: view.id, caret, until: moment.seconds + timeout });
        self.last_shown_at = Some(moment.seconds);
        self.state.count_shown(moment.day());
        self.state.see(view.id, self.version, &moment.timestamp, How::Shown);
        self.dirty = true;
    }

    /// The platform could not draw `view`: it stays unseen. At the caret this counts as a caret not found.
    pub fn not_drawn(&mut self, view: &HintView, moment: &Moment) {
        if view.place == Place::Caret {
            self.caret_missed(view.index, moment);
        }
    }

    /// The user closed a hint. «Больше не показывать подсказки» turns them off: true then, and the caller writes
    /// `general.hints`.
    pub fn dismissed(&mut self, id: &str, how: How) -> bool {
        if self.visible.is_some_and(|visible| visible.id == id) {
            self.visible = None;
        }
        self.state.set_how(id, how);
        self.dirty = true;
        if how != How::Forever {
            return false;
        }
        self.settings.enabled = false;
        self.pending.clear();
        true
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;
    use crate::hints::catalogue::{CATALOGUE, Destination, HotkeyName, LAYOUT_SWITCH_KEY_SETTING, MODE_EARLY, MODE_SYMBOLS, MODE_TYPO, MODE_UNDO};
    use crate::hints::store::Startup;

    type Anchor = (i32, i32);
    const AT: Anchor = (100, 200);
    const TODAY: &str = "2026-11-07T10:00:00+00:00";
    const LAYOUT: CorrectionInfo = CorrectionInfo { mode: "boundary", automatic: true, case_fix: false };

    fn version(text: &str) -> SemanticVersion {
        SemanticVersion::parse(text).unwrap()
    }

    fn current() -> SemanticVersion {
        version("0.71.0")
    }

    fn at(seconds: f64) -> Moment {
        typing(seconds, 0)
    }

    fn typing(seconds: f64, typed_presses: i64) -> Moment {
        Moment { seconds, timestamp: TODAY.to_owned(), typed_presses }
    }

    fn hotkeys() -> Hotkeys {
        Hotkeys {
            undo: "Ctrl+Alt+Z".into(),
            convert_last: "Pause".into(),
            selection_layout: "Shift+Pause".into(),
            selection_case: "Alt+Pause".into(),
            selection_transliterate: "Alt+Scroll_Lock".into(),
        }
    }

    fn on() -> HintSettings {
        HintSettings { enabled: true, notifications: true, keep_history: true, hotkeys: hotkeys() }
    }

    fn started(startup: Startup) -> HintState {
        let mut state = HintState::default();
        state.begin(startup, current(), TODAY);
        state
    }

    /// A state where every hint but `ids` was seen.
    fn only(ids: &[&str]) -> HintState {
        let mut state = started(Startup::Same);
        for hint in &CATALOGUE {
            if !ids.contains(&hint.id) {
                state.see(hint.id, current(), TODAY, How::Ok);
            }
        }
        state
    }

    fn with(state: HintState) -> Scheduler<Anchor> {
        Scheduler::new(&CATALOGUE, Platform::Linux, current(), on(), state, &at(0.0))
    }

    fn scheduler(startup: Startup) -> Scheduler<Anchor> {
        with(started(startup))
    }

    fn leak(hints: Vec<Hint>) -> &'static [Hint] {
        Box::leak(hints.into_boxed_slice())
    }

    fn info(mode: &'static str, automatic: bool) -> CorrectionInfo {
        CorrectionInfo { mode, automatic, case_fix: false }
    }

    fn describe(step: Option<&Step<Anchor>>) -> String {
        match step {
            None => "none".into(),
            Some(Step::Caret(view, anchor)) => {
                assert_eq!(*anchor, AT);
                format!("caret {}", view.id)
            }
            Some(Step::Tray(view)) => format!("tray {}", view.id),
            Some(Step::Hide(id)) => format!("hide {id}"),
        }
    }

    /// What is due at `moment`, reported drawn.
    fn show(scheduler: &mut Scheduler<Anchor>, moment: &Moment) -> String {
        let step = scheduler.tick(moment);
        if let Some(Step::Caret(view, _) | Step::Tray(view)) = &step {
            scheduler.drawn(view, moment);
        }
        describe(step.as_ref())
    }

    #[test]
    fn a_new_installation_is_told_where_the_program_is_once_the_engine_runs() {
        let mut scheduler = scheduler(Startup::Fresh);
        assert!(!scheduler.needs_tick());
        scheduler.engine_running(&at(10.0));
        assert!(scheduler.needs_tick());
        assert_eq!(show(&mut scheduler, &at(14.0)), "none");
        assert_eq!(show(&mut scheduler, &at(15.0)), "tray first_start");
        assert_eq!(show(&mut scheduler, &at(34.0)), "none");
        assert_eq!(show(&mut scheduler, &at(35.0)), "hide first_start");
        assert_eq!(scheduler.state().how("first_start").as_deref(), Some("timeout"));
        scheduler.engine_running(&at(40.0));
        assert!(!scheduler.needs_tick());
        assert!(scheduler.take_dirty() && !scheduler.take_dirty());
        // A new installation that already saw it hears nothing.
        let mut state = started(Startup::Fresh);
        state.see("first_start", current(), TODAY, How::Ok);
        let mut again = with(state);
        again.engine_running(&at(0.0));
        assert!(!again.needs_tick());
    }

    #[test]
    fn an_update_from_a_version_before_the_hints_brings_both_news_in_turn() {
        for from in [None, Some(version("0.66.2"))] {
            let mut scheduler = scheduler(Startup::Updated { from });
            scheduler.engine_running(&at(1.0));
            assert_eq!(show(&mut scheduler, &at(29.0)), "none");
            assert_eq!(show(&mut scheduler, &at(30.0)), "tray new_0_70_switch_layout_key");
            assert_eq!(show(&mut scheduler, &at(50.0)), "hide new_0_70_switch_layout_key");
            assert_eq!(show(&mut scheduler, &at(51.0)), "none");
            assert_eq!(show(&mut scheduler, &at(630.0)), "tray updated");
            assert_eq!(scheduler.state().shown_on("2026-11-07"), 2);
            assert_eq!(show(&mut scheduler, &at(650.0)), "hide updated");
            assert!(!scheduler.needs_tick());
        }
    }

    #[test]
    fn an_update_from_0_70_brings_only_the_news_of_the_update() {
        let mut scheduler = scheduler(Startup::Updated { from: Some(version("0.70.0")) });
        assert_eq!(show(&mut scheduler, &at(30.0)), "tray updated");
        assert_eq!(show(&mut scheduler, &at(50.0)), "hide updated");
        assert!(!scheduler.needs_tick());
    }

    #[test]
    fn the_same_version_and_a_downgrade_bring_no_news() {
        assert!(!scheduler(Startup::Same).needs_tick());
        let downgraded = HintState::parse(r#"{"update": {"from": "0.70.0", "to": "0.72.0"}}"#).unwrap();
        assert!(!with(downgraded).needs_tick());
        // A version older than the news it carries: only the news of the update.
        let mut state = HintState::default();
        state.begin(Startup::Updated { from: None }, version("0.69.0"), TODAY);
        let mut older = Scheduler::<Anchor>::new(&CATALOGUE, Platform::Linux, version("0.69.0"), on(), state, &at(0.0));
        assert_eq!(show(&mut older, &at(30.0)), "tray updated");
    }

    #[test]
    fn the_news_of_an_update_shows_once_for_each_version() {
        let mut state = started(Startup::Updated { from: Some(version("0.70.0")) });
        state.see("updated", current(), TODAY, How::Ok);
        assert!(!with(state).needs_tick());
        let mut state = started(Startup::Updated { from: Some(version("0.70.0")) });
        state.see("updated", version("0.70.0"), TODAY, How::Ok);
        assert!(with(state).needs_tick());
    }

    #[test]
    fn a_hint_at_the_caret_waits_until_the_typing_stops() {
        let mut scheduler = scheduler(Startup::Same);
        assert!(scheduler.wants_caret_anchor());
        scheduler.correction(LAYOUT, Some(AT), &typing(0.0, 10));
        assert_eq!(show(&mut scheduler, &typing(0.5, 11)), "none");
        assert_eq!(show(&mut scheduler, &typing(1.2, 11)), "none");
        assert_eq!(show(&mut scheduler, &typing(1.5, 11)), "caret first_correction_undo");
        assert_eq!(show(&mut scheduler, &typing(13.4, 11)), "none");
        assert_eq!(show(&mut scheduler, &typing(13.5, 11)), "hide first_correction_undo");
    }

    #[test]
    fn a_hint_at_the_caret_gives_up_while_the_typing_goes_on() {
        let mut scheduler = scheduler(Startup::Same);
        scheduler.correction(LAYOUT, Some(AT), &typing(0.0, 0));
        for step in 1..=13 {
            assert_eq!(show(&mut scheduler, &typing(f64::from(step) * 0.5, i64::from(step))), "none");
        }
        assert!(!scheduler.needs_tick());
        assert!(!scheduler.state().is_seen("first_correction_undo"));
        scheduler.correction(LAYOUT, Some(AT), &typing(10.0, 13));
        assert_eq!(show(&mut scheduler, &typing(11.0, 13)), "caret first_correction_undo");
    }

    #[test]
    fn hints_keep_quiet_between_them_and_count_the_day() {
        let mut scheduler = scheduler(Startup::Same);
        scheduler.correction(LAYOUT, Some(AT), &at(0.0));
        assert_eq!(show(&mut scheduler, &at(1.0)), "caret first_correction_undo");
        assert!(!scheduler.dismissed("first_correction_undo", How::Ok));
        assert_eq!(scheduler.state().how("first_correction_undo").as_deref(), Some("ok"));
        scheduler.correction(info(MODE_EARLY, true), Some(AT), &at(4.0));
        assert_eq!(show(&mut scheduler, &at(5.0)), "none");
        scheduler.correction(info(MODE_EARLY, true), Some(AT), &at(601.0));
        assert_eq!(show(&mut scheduler, &at(602.0)), "caret early_switch");

        let mut state = started(Startup::Same);
        for _ in 0..3 {
            state.count_shown("2026-11-07");
        }
        let mut full = with(state);
        full.correction(LAYOUT, Some(AT), &at(0.0));
        assert_eq!(show(&mut full, &at(1.0)), "none");
        let tomorrow = Moment { seconds: 2.0, timestamp: "2026-11-08T10:00:00+00:00".into(), typed_presses: 0 };
        assert_eq!(show(&mut full, &tomorrow), "caret first_correction_undo");
        assert_eq!(full.state().shown_on("2026-11-08"), 1);
    }

    #[test]
    fn no_hint_at_the_caret_while_a_rule_prompt_is_open() {
        let mut scheduler = scheduler(Startup::Same);
        assert_eq!(scheduler.prompt(true, &at(0.0)), None);
        scheduler.correction(LAYOUT, Some(AT), &at(0.0));
        assert_eq!(show(&mut scheduler, &at(2.0)), "none");
        assert_eq!(scheduler.prompt(false, &at(3.0)), None);
        assert_eq!(show(&mut scheduler, &at(3.0)), "caret first_correction_undo");
        assert_eq!(describe(scheduler.prompt(true, &at(4.0)).as_ref()), "hide first_correction_undo");
        assert!(!scheduler.needs_tick());
        // One near the tray icon stays.
        let mut fresh = self::scheduler(Startup::Fresh);
        fresh.engine_running(&at(0.0));
        assert_eq!(show(&mut fresh, &at(5.0)), "tray first_start");
        assert_eq!(fresh.prompt(true, &at(6.0)), None);
        assert!(fresh.needs_tick());
    }

    #[test]
    fn a_rule_prompt_right_after_a_manual_conversion_means_the_double_press_was_found() {
        let mut scheduler = scheduler(Startup::Same);
        scheduler.correction(info(MODE_MANUAL, false), Some(AT), &at(0.0));
        assert!(scheduler.prompt(true, &at(0.4)).is_none());
        assert_eq!(scheduler.state().how("double_pause_rule").as_deref(), Some("skipped"));
        assert!(!scheduler.needs_tick());
        let mut later = self::scheduler(Startup::Same);
        later.correction(info(MODE_MANUAL, false), Some(AT), &at(0.0));
        later.prompt(true, &at(2.5));
        assert!(!later.state().is_seen("double_pause_rule"));
        let mut seen = with(only(&[]));
        seen.correction(info(MODE_MANUAL, false), Some(AT), &at(0.0));
        seen.prompt(true, &at(0.4));
        assert_eq!(seen.state().how("double_pause_rule").as_deref(), Some("ok"));
    }

    #[test]
    fn each_kind_of_correction_has_its_hint() {
        let kinds = [
            (info(MODE_EARLY, true), "early_switch"),
            (info(MODE_TYPO, true), "typo"),
            (info(MODE_UNDO, false), "exclusions"),
            (info(MODE_MANUAL, false), "double_pause_rule"),
        ];
        for (correction, expected) in kinds {
            let mut scheduler = scheduler(Startup::Same);
            scheduler.correction(correction, Some(AT), &at(0.0));
            assert_eq!(show(&mut scheduler, &at(1.0)), format!("caret {expected}"));
        }
        for correction in [CorrectionInfo { mode: "case", automatic: true, case_fix: true }, info(MODE_SYMBOLS, false)] {
            let mut scheduler = scheduler(Startup::Same);
            scheduler.correction(correction, Some(AT), &at(0.0));
            assert!(!scheduler.needs_tick(), "{correction:?}");
        }
    }

    #[test]
    fn the_counted_corrections_bring_their_hints() {
        let mut pause = with(only(&["pause_converts"]));
        for second in 0..4 {
            pause.correction(LAYOUT, Some(AT), &at(f64::from(second)));
        }
        assert!(!pause.needs_tick());
        pause.correction(LAYOUT, Some(AT), &at(4.0));
        assert_eq!(show(&mut pause, &at(5.0)), "caret pause_converts");
        let mut converted = with(only(&["pause_converts"]));
        converted.correction(info(MODE_MANUAL, false), Some(AT), &at(0.0));
        for second in 1..8 {
            converted.correction(LAYOUT, Some(AT), &at(f64::from(second)));
        }
        assert!(!converted.needs_tick());

        let mut selection = with(only(&["selection_commands"]));
        selection.correction(info(MODE_MANUAL, false), Some(AT), &at(0.0));
        selection.correction(info(MODE_MANUAL, false), Some(AT), &at(1.0));
        assert!(!selection.needs_tick());
        selection.correction(info(MODE_SYMBOLS, false), Some(AT), &at(2.0));
        assert_eq!(show(&mut selection, &at(3.0)), "caret selection_commands");

        for (id, count) in [("notifications_sound", 3), ("history", 10)] {
            let mut tray = with(only(&[id]));
            for second in 0..count {
                tray.correction(LAYOUT, Some(AT), &at(f64::from(second)));
            }
            assert_eq!(show(&mut tray, &at(f64::from(count))), format!("tray {id}"));
            // Another correction while it shows does not set it off again.
            tray.correction(LAYOUT, Some(AT), &at(f64::from(count) + 1.0));
            assert_eq!(show(&mut tray, &at(f64::from(count) + 20.0)), format!("hide {id}"));
            assert!(!tray.needs_tick());
            let mut off = with(only(&[id]));
            off.set_settings(HintSettings { notifications: false, keep_history: false, ..on() });
            for second in 0..count {
                off.correction(LAYOUT, None, &at(f64::from(second)));
            }
            assert!(!off.needs_tick());
        }
    }

    #[test]
    fn the_counters_stop_at_the_most_a_hint_that_may_show_waits_for() {
        let mut history = with(only(&["history"]));
        history.set_settings(HintSettings { keep_history: false, ..on() });
        for second in 0..12 {
            history.correction(LAYOUT, None, &at(f64::from(second)));
            assert_eq!(history.take_dirty(), second < 10, "{second}");
        }
        assert_eq!(history.state().counted(), Counted { automatic_corrections: 10, manual_conversions: 0 });
        let mut selection = with(only(&["selection_commands"]));
        for second in 0..4 {
            selection.correction(info(MODE_MANUAL, false), None, &at(f64::from(second) * 700.0));
        }
        assert_eq!(selection.state().counted().manual_conversions, 3);
        // The hint of Pause needs to know only that there was a manual conversion.
        let mut pause = with(only(&["pause_converts"]));
        pause.correction(info(MODE_SYMBOLS, false), None, &at(0.0));
        pause.correction(info(MODE_SYMBOLS, false), None, &at(1.0));
        assert_eq!(pause.state().counted(), Counted { automatic_corrections: 0, manual_conversions: 1 });
        let mut none = with(only(&["first_start"]));
        none.correction(LAYOUT, Some(AT), &at(0.0));
        none.correction(info(MODE_UNDO, false), Some(AT), &at(1.0));
        assert!(!none.take_dirty());
        assert_eq!(none.state().counted(), Counted::default());
    }

    #[test]
    fn hints_turned_off_take_down_what_shows_and_forget_what_waits() {
        let mut scheduler = with(only(&["first_correction_undo", "early_switch"]));
        scheduler.correction(LAYOUT, Some(AT), &at(0.0));
        assert_eq!(show(&mut scheduler, &at(1.0)), "caret first_correction_undo");
        scheduler.correction(info(MODE_EARLY, true), Some(AT), &at(2.0));
        let off = HintSettings { enabled: false, ..on() };
        assert_eq!(describe(scheduler.set_settings(off.clone()).as_ref()), "hide first_correction_undo");
        assert!(!scheduler.needs_tick() && !scheduler.wants_caret_anchor());
        assert_eq!(scheduler.set_settings(off), None);
        scheduler.correction(info(MODE_EARLY, true), Some(AT), &at(3.0));
        assert!(!scheduler.needs_tick());
        assert_eq!(scheduler.set_settings(on()), None);
        scheduler.correction(LAYOUT, Some(AT), &at(700.0));
        assert!(!scheduler.needs_tick());
        scheduler.correction(info(MODE_EARLY, true), Some(AT), &at(701.0));
        assert_eq!(show(&mut scheduler, &at(702.0)), "caret early_switch");
        let mut quiet = self::scheduler(Startup::Same);
        assert_eq!(quiet.set_settings(HintSettings { enabled: false, ..on() }), None);
    }

    #[test]
    fn never_again_turns_the_hints_off() {
        let mut scheduler = scheduler(Startup::Same);
        scheduler.correction(LAYOUT, Some(AT), &at(0.0));
        assert_eq!(show(&mut scheduler, &at(1.0)), "caret first_correction_undo");
        assert!(scheduler.dismissed("first_correction_undo", How::Forever));
        assert!(!scheduler.settings().enabled);
        assert_eq!(scheduler.state().how("first_correction_undo").as_deref(), Some("forever"));
        scheduler.correction(info(MODE_EARLY, true), Some(AT), &at(700.0));
        assert!(!scheduler.needs_tick());
    }

    #[test]
    fn a_caret_not_found_three_times_running_sends_the_hint_to_the_tray() {
        let mut scheduler = scheduler(Startup::Same);
        scheduler.correction(LAYOUT, None, &at(0.0));
        scheduler.correction(LAYOUT, None, &at(1.0));
        assert!(!scheduler.needs_tick());
        scheduler.correction(LAYOUT, None, &at(2.0));
        assert_eq!(show(&mut scheduler, &at(2.0)), "tray first_correction_undo");

        // A card that could not be drawn counts the same, and one drawn starts the count again.
        fn miss(scheduler: &mut Scheduler<Anchor>, seconds: f64) {
            scheduler.correction(info(MODE_EARLY, true), Some(AT), &at(seconds));
            let Some(Step::Caret(view, _)) = scheduler.tick(&at(seconds + 1.0)) else { panic!("a caret hint") };
            scheduler.not_drawn(&view, &at(seconds + 1.0));
        }
        let mut drawing = with(only(&["first_correction_undo", "early_switch"]));
        miss(&mut drawing, 0.0);
        miss(&mut drawing, 10.0);
        drawing.correction(LAYOUT, Some(AT), &at(20.0));
        assert_eq!(show(&mut drawing, &at(21.0)), "caret first_correction_undo");
        drawing.dismissed("first_correction_undo", How::Ok);
        miss(&mut drawing, 700.0);
        miss(&mut drawing, 710.0);
        assert!(!drawing.needs_tick());
        miss(&mut drawing, 720.0);
        assert_eq!(show(&mut drawing, &at(722.0)), "tray early_switch");
        // A hint near the tray icon that could not be drawn is let go.
        let mut fresh = self::scheduler(Startup::Fresh);
        fresh.engine_running(&at(0.0));
        let Some(Step::Tray(view)) = fresh.tick(&at(5.0)) else { panic!("the first start") };
        fresh.not_drawn(&view, &at(5.0));
        assert!(!fresh.needs_tick() && !fresh.state().is_seen("first_start"));
    }

    #[test]
    fn a_settings_card_shows_once_on_its_page_and_is_not_counted() {
        let mut scheduler = scheduler(Startup::Same);
        assert_eq!(scheduler.settings_card(SettingsPage::Hotkeys, &at(0.0)), None);
        let card = scheduler.settings_card(SettingsPage::Overview, &at(0.0)).unwrap();
        assert_eq!(card.id, "hints_intro");
        assert_eq!(card.place, Place::Settings { page: SettingsPage::Overview, control: None });
        assert_eq!(card.opens, Some(Opens::Settings { page: SettingsPage::System, setting: Some("general.hints") }));
        assert_eq!((card.title, card.ok, card.never), ("KeySwitch", "Понятно", "Больше не показывать подсказки"));
        assert!(scheduler.state().is_seen("hints_intro"));
        assert_eq!(scheduler.settings_card(SettingsPage::Overview, &at(1.0)), None);
        scheduler.settings_opened();
        assert_eq!(scheduler.settings_card(SettingsPage::Overview, &at(2.0)), None);
        scheduler.correction(LAYOUT, Some(AT), &at(3.0));
        assert_eq!(show(&mut scheduler, &at(4.0)), "caret first_correction_undo");
        assert!(!scheduler.dismissed("hints_intro", How::Ok));
        assert!(scheduler.needs_tick());

        let mut updated = self::scheduler(Startup::Updated { from: None });
        let news = updated.settings_card(SettingsPage::Hotkeys, &at(0.0)).unwrap();
        assert_eq!(news.place, Place::Settings { page: SettingsPage::Hotkeys, control: Some(LAYOUT_SWITCH_KEY_SETTING) });
        assert_eq!(show(&mut updated, &at(30.0)), "tray updated");
    }

    #[test]
    fn a_page_shows_one_card_each_time_the_window_opens() {
        let card = |id: &'static str| Hint {
            id,
            trigger: Trigger::SettingsPage,
            places: &[Place::Settings { page: SettingsPage::Overview, control: None }],
            ..CATALOGUE[10]
        };
        let catalogue = leak(vec![card("a"), card("b")]);
        let mut scheduler = Scheduler::<Anchor>::new(catalogue, Platform::Linux, current(), on(), started(Startup::Same), &at(0.0));
        assert_eq!(scheduler.settings_card(SettingsPage::Overview, &at(0.0)).map(|view| view.id), Some("a"));
        assert_eq!(scheduler.settings_card(SettingsPage::Overview, &at(0.0)), None);
        scheduler.settings_opened();
        assert_eq!(scheduler.settings_card(SettingsPage::Overview, &at(0.0)).map(|view| view.id), Some("b"));
    }

    #[test]
    fn odd_entries_of_a_catalogue_are_held_to_its_rules() {
        let windows_only = Hint { id: "windows", platforms: &[Platform::Windows], ..CATALOGUE[1] };
        let in_the_window = Hint {
            id: "window",
            trigger: Trigger::Mode("boundary"),
            places: &[Place::Settings { page: SettingsPage::Overview, control: None }],
            ..CATALOGUE[1]
        };
        let named = Hint {
            id: "named",
            trigger: Trigger::SettingsPage,
            places: &[Place::Settings { page: SettingsPage::Overview, control: None }],
            hotkeys: &[HotkeyName::Undo],
            ..CATALOGUE[1]
        };
        let catalogue = leak(vec![windows_only, in_the_window, named]);
        let cleared = HintSettings { hotkeys: Hotkeys { undo: String::new(), ..hotkeys() }, ..on() };
        let mut linux = Scheduler::<Anchor>::new(catalogue, Platform::Linux, current(), cleared, started(Startup::Same), &at(0.0));
        linux.correction(LAYOUT, Some(AT), &at(0.0));
        assert!(!linux.needs_tick());
        assert_eq!(linux.settings_card(SettingsPage::Overview, &at(0.0)), None);
        let mut windows = Scheduler::<Anchor>::new(catalogue, Platform::Windows, current(), on(), started(Startup::Same), &at(0.0));
        windows.correction(LAYOUT, Some(AT), &at(0.0));
        assert_eq!(show(&mut windows, &at(1.0)), "caret windows");
    }

    #[test]
    fn a_hint_whose_hotkey_was_cleared_is_let_go_when_due() {
        let mut scheduler = with(started(Startup::Same));
        scheduler.set_settings(HintSettings { hotkeys: Hotkeys { undo: String::new(), ..hotkeys() }, ..on() });
        scheduler.correction(LAYOUT, Some(AT), &at(0.0));
        assert!(scheduler.needs_tick());
        assert_eq!(show(&mut scheduler, &at(1.0)), "none");
        assert!(!scheduler.needs_tick());
    }

    #[test]
    fn the_caret_is_wanted_while_a_hint_at_it_may_show() {
        assert!(scheduler(Startup::Same).wants_caret_anchor());
        assert!(!with(only(&["first_start", "history"])).wants_caret_anchor());
    }

    #[test]
    fn a_click_on_a_text_opens_its_destination() {
        let scheduler = scheduler(Startup::Same);
        assert_eq!(scheduler.opens_of("updated"), Some(Opens::Url("https://github.com/olegius88/keyswitch/releases/tag/v0.71.0".into())));
        assert_eq!(scheduler.opens_of("first_start"), None);
        assert_eq!(scheduler.opens_of("nothing"), None);
        let typo = CATALOGUE.iter().find(|hint| hint.id == "typo").unwrap();
        assert_eq!(typo.opens, Some(Destination::Settings { page: SettingsPage::Autocorrection, setting: Some("detection.correct_typos") }));
    }

    #[test]
    fn a_moment_says_its_day() {
        let moment = Moment::now(Instant::now(), 3);
        assert!(moment.seconds >= 0.0 && moment.typed_presses == 3);
        assert_eq!(moment.day().len(), 10);
        assert_eq!(Moment { seconds: 0.0, timestamp: "short".into(), typed_presses: 0 }.day(), "short");
    }
}
