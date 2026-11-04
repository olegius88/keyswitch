//! Session detection of the Wayland desktop layer (the Python's `wayland_desktop.py`). The rest of that module, the
//! compositor helpers, belongs to the Wayland port.

/// Whether the current Linux session runs on Wayland rather than X11, from the given environment.
pub fn wayland_session_active_in(environment: &dyn Fn(&str) -> Option<String>) -> bool {
    let get = |name: &str| environment(name).unwrap_or_default();
    let backend = ks_text::pystr::casefold(ks_text::pystr::strip(&get("GDK_BACKEND")));
    if backend == "x11" {
        return false;
    }
    if backend == "wayland" {
        return true;
    }
    if !ks_text::pystr::strip(&get("WAYLAND_DISPLAY")).is_empty() {
        return true;
    }
    ks_text::pystr::casefold(ks_text::pystr::strip(&get("XDG_SESSION_TYPE"))) == "wayland" && ks_text::pystr::strip(&get("DISPLAY")).is_empty()
}

pub fn wayland_session_active() -> bool {
    wayland_session_active_in(&|name| std::env::var(name).ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn active(pairs: &[(&str, &str)]) -> bool {
        wayland_session_active_in(&|name| pairs.iter().find(|(key, _)| *key == name).map(|(_, value)| (*value).to_owned()))
    }

    #[test]
    fn the_session_kind_follows_the_environment() {
        assert!(!active(&[]));
        assert!(!active(&[("GDK_BACKEND", " X11 "), ("WAYLAND_DISPLAY", "wayland-0")]));
        assert!(active(&[("GDK_BACKEND", "wayland")]));
        assert!(active(&[("WAYLAND_DISPLAY", "wayland-0")]));
        assert!(active(&[("XDG_SESSION_TYPE", "Wayland")]));
        assert!(!active(&[("XDG_SESSION_TYPE", "wayland"), ("DISPLAY", ":0")]));
        assert!(!active(&[("WAYLAND_DISPLAY", "  ")]));
    }
}
