//! Platform-neutral facts about key events, and the bounds of what a platform reads from a field.

// Internal modifier bits intentionally match the X11 core masks. Platform backends normalize their
// native state into these values before emitting an event, keeping the engine independent from an
// operating system API.
pub const SHIFT_MASK: u32 = 1 << 0;
pub const LOCK_MASK: u32 = 1 << 1;
pub const CONTROL_MASK: u32 = 1 << 2;
pub const ALT_MASK: u32 = 1 << 3;
pub const SUPER_MASK: u32 = 1 << 6;
/// The keyboard event pair (down, up) `complete_action` sends for one key.
pub const COMPLETED_ACTION_EVENT_COUNT: i32 = 2;
/// Language layout groups the application works with: 0 (EN) and 1 (RU). A third language would need
/// a third physical layout group, which nothing supports yet.
pub const LAYOUT_GROUP_COUNT: i32 = 2;
/// Layout group a synthetic pointer event carries: it belongs to no layout.
pub const POINTER_EVENT_GROUP: i32 = -1;
/// Key name of a key event that carries a character instead of a key position (VK_PACKET on
/// Windows, sent with KEYEVENTF_UNICODE by remote control and on-screen keyboards).
pub const UNICODE_PACKET_KEY_NAME: &str = "Packet";

pub const FIELD_CONTEXT_MAX_CHARACTERS: usize = 512;
pub const FIELD_CONTEXT_APPLICATION_MAX_CHARACTERS: usize = 128;
pub const FIELD_CONTEXT_FIELD_ID_MAX_CHARACTERS: usize = 128;
pub const FIELD_CONTEXT_SOURCE_MAX_CHARACTERS: usize = 32;
pub const FIELD_INPUT_BUFFER_MAX_CHARACTERS: usize = 1;

pub const INPUT_DELAY_LATE_MS: i64 = 100;
pub const INPUT_DELAY_UNSTAMPED_MS: i64 = 60_000;
pub const INPUT_DELAY_SLOW_CALLBACK_MS: i64 = 20;
pub const HOOK_GAP_MS: i64 = 280;
