//! One-time hints about what KeySwitch can do and what a release brought: at the caret after a correction, near the
//! tray icon, and as cards in the settings window. Each shows once, never while the user types or a rule prompt is
//! open, and none of them touches the engine: they listen to its corrections, prompts and snapshots, and the replays and
//! traces of the engine do not see them.
//!
//! Apart from any toolkit and any clock: a platform hands in the time ([`Moment`]) and draws through
//! [`HintPresenter`]. `catalogue` holds the hints and their texts, `schedule` the rules of when one shows, `store` the
//! file, `controller` the wiring.

pub mod catalogue;
pub mod controller;
pub mod schedule;
pub mod store;

pub use catalogue::{
    CATALOGUE, CorrectionInfo, Destination, HINT_NEVER_LABEL, HINT_OK_LABEL, HINT_TITLE, Hint, Opens, Place, Platform, RUNNING_PLATFORM,
    SettingsPage,
};
pub use controller::{Click, EngineInput, HintPresenter, Hints, subscribe};
pub use schedule::{HintSettings, HintView, Moment, Scheduler, Step};
pub use store::{HintState, HintStore, How, Startup};
