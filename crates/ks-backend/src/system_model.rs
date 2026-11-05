//! What every platform reports about installed programs and about autostart.
//!
//! The shapes are the same everywhere - a program the user can exclude, and the answer to "will `KeySwitch` start with
//! the session" - so they are defined once and each platform fills them in its own way.

/// A program as the exclusion list names it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Application {
    pub name: String,
    pub identifier: String,
    pub executable: String,
}

/// What the user actually gets at the next logon, and why.
///
/// `command` is what the system will run, empty when nothing is registered. `blocked_by_windows` records a refusal
/// Windows keeps separately from the entry itself: Task Manager and Settings mark a startup entry disabled, and such an
/// entry is skipped however often it is rewritten. Systems without that notion leave it false. `target_missing` marks an
/// entry pointing at a program that is no longer there.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AutostartStatus {
    pub command: Option<String>,
    pub blocked_by_windows: bool,
    pub target_missing: bool,
}

impl AutostartStatus {
    #[must_use]
    pub fn effective(&self) -> bool {
        self.command.as_deref().is_some_and(|command| !command.is_empty()) && !self.blocked_by_windows && !self.target_missing
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;

    #[test]
    fn autostart_is_effective_only_when_the_next_logon_really_starts_it() {
        let status = |command: Option<&str>, blocked, missing| AutostartStatus {
            command: command.map(str::to_owned),
            blocked_by_windows: blocked,
            target_missing: missing,
        };
        assert!(status(Some("x"), false, false).effective());
        assert!(!status(None, false, false).effective());
        assert!(!status(Some(""), false, false).effective());
        assert!(!status(Some("x"), true, false).effective());
        assert!(!status(Some("x"), false, true).effective());
    }
}
