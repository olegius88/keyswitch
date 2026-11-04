//! How late the keyboard hook saw the user's keys, as evidence of a stall in the technical log.
//!
//! A low-level hook stands between every key and every window: when it answers late, all typing in
//! the system waits for it. The hook thread counts here how long after the key it ran and how long
//! it took; the engine's worker takes the counts once a minute and logs them when a key was late.
//!
//! Keys other programs inject (TeamViewer types every character it receives as one) carry no time
//! the user pressed them at; only how long the hook took over them is counted, so that on a
//! computer controlled from another one the log still says whether KeySwitch held its input up.
//!
//! A key the hook reached HOOK_GAP_MS late, or answered that slowly, also leaves a gap: Windows
//! gives the keys that waited behind it that long to the window without the hook, and the engine
//! no longer knows what stands after the word it tracks.

use std::sync::Mutex;

use crate::constants::{HOOK_GAP_MS, INPUT_DELAY_LATE_MS, INPUT_DELAY_SLOW_CALLBACK_MS, INPUT_DELAY_UNSTAMPED_MS};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InputDelayReport {
    pub keys: i64,
    /// Keys the hook saw INPUT_DELAY_LATE_MS or more after the system stamped them.
    pub late_keys: i64,
    pub worst_late_ms: i64,
    /// Keys, typed or another program's, whose hook callback took INPUT_DELAY_SLOW_CALLBACK_MS or more.
    pub slow_callbacks: i64,
    pub worst_callback_ms: i64,
    /// Typed keys stamped INPUT_DELAY_UNSTAMPED_MS or more before the hook saw them: a time of
    /// their sender's own, no delay.
    pub unstamped_keys: i64,
    /// Keys another program injected, timed by the hook's answer alone.
    pub foreign_keys: i64,
}

impl InputDelayReport {
    pub fn stalled(&self) -> bool {
        self.late_keys != 0 || self.slow_callbacks != 0
    }
}

/// The key after which keys may have passed the hook: how late the hook ran for it and how long it took.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HookGap {
    pub late_ms: i64,
    pub callback_ms: i64,
}

#[derive(Default)]
struct Counts {
    report: InputDelayReport,
    gap: Option<HookGap>,
}

/// Counts the hook thread adds to and the engine's worker takes.
#[derive(Default)]
pub struct InputDelay {
    counts: Mutex<Counts>,
}

impl InputDelay {
    pub fn new() -> Self {
        Self::default()
    }

    fn counts(&self) -> std::sync::MutexGuard<'_, Counts> {
        self.counts.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// A typed key the hook has just reached: a gap is known before the key goes on to the engine.
    pub fn note_wait(&self, late_ms: i64) {
        if (HOOK_GAP_MS..INPUT_DELAY_UNSTAMPED_MS).contains(&late_ms) {
            self.counts().gap = Some(HookGap { late_ms, callback_ms: 0 });
        }
    }

    /// A typed key: how long after its key time the hook ran, and how long the answer took.
    pub fn record(&self, late_ms: i64, callback_ms: i64) {
        let stamped = late_ms < INPUT_DELAY_UNSTAMPED_MS;
        let mut counts = self.counts();
        let report = &mut counts.report;
        report.keys += 1;
        report.late_keys += i64::from(stamped && late_ms >= INPUT_DELAY_LATE_MS);
        if stamped {
            report.worst_late_ms = report.worst_late_ms.max(late_ms);
        }
        report.slow_callbacks += i64::from(callback_ms >= INPUT_DELAY_SLOW_CALLBACK_MS);
        report.worst_callback_ms = report.worst_callback_ms.max(callback_ms);
        report.unstamped_keys += i64::from(!stamped);
        // A key already late enough was noted as the hook reached it (note_wait).
        if stamped && late_ms < HOOK_GAP_MS && HOOK_GAP_MS <= late_ms + callback_ms {
            counts.gap = Some(HookGap { late_ms, callback_ms });
        }
    }

    /// A key another program injected: how long the answer took.
    pub fn record_foreign(&self, callback_ms: i64) {
        let mut counts = self.counts();
        let report = &mut counts.report;
        report.slow_callbacks += i64::from(callback_ms >= INPUT_DELAY_SLOW_CALLBACK_MS);
        report.worst_callback_ms = report.worst_callback_ms.max(callback_ms);
        report.foreign_keys += 1;
        if callback_ms >= HOOK_GAP_MS {
            counts.gap = Some(HookGap { late_ms: 0, callback_ms });
        }
    }

    /// The counts since the last take, and a fresh start.
    pub fn take(&self) -> InputDelayReport {
        std::mem::take(&mut self.counts().report)
    }

    /// The last gap since the previous call, if any.
    pub fn take_gap(&self) -> Option<HookGap> {
        self.counts().gap.take()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_late_key_is_counted_and_leaves_a_gap() {
        let delay = InputDelay::new();
        delay.note_wait(300);
        delay.record(300, 1);
        let report = delay.take();
        assert_eq!((report.keys, report.late_keys, report.worst_late_ms), (1, 1, 300));
        assert_eq!(delay.take_gap(), Some(HookGap { late_ms: 300, callback_ms: 0 }));
        assert_eq!(delay.take_gap(), None);
        assert_eq!(delay.take().keys, 0);
    }

    #[test]
    fn an_unstamped_key_is_no_delay() {
        let delay = InputDelay::new();
        delay.note_wait(INPUT_DELAY_UNSTAMPED_MS);
        delay.record(INPUT_DELAY_UNSTAMPED_MS, 0);
        let report = delay.take();
        assert_eq!((report.late_keys, report.unstamped_keys, report.worst_late_ms), (0, 1, 0));
        assert_eq!(delay.take_gap(), None);
    }

    #[test]
    fn a_slow_answer_leaves_a_gap_of_its_own() {
        let delay = InputDelay::new();
        delay.record(100, 200);
        assert_eq!(delay.take_gap(), Some(HookGap { late_ms: 100, callback_ms: 200 }));
        delay.record_foreign(HOOK_GAP_MS);
        assert_eq!(delay.take_gap(), Some(HookGap { late_ms: 0, callback_ms: HOOK_GAP_MS }));
        assert_eq!(delay.take().foreign_keys, 1);
    }
}
