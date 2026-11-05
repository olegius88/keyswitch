//! Where the focused text caret is on the screen (the Python's `focused_caret_anchor`), read through the accessibility bus.

use crate::atspi_context::{AtspiApi, INTERFACE_TEXT, NodeRef};
use crate::constants::ATSPI_STATE_FOCUSED;
use crate::gtk_model::{MAX_CARET_SEARCH_CHILDREN, MAX_CARET_SEARCH_NODES};

/// The point under the caret: the right edge of the character before it, and its bottom.
fn caret_anchor_for_node(api: &dyn AtspiApi, node: &NodeRef) -> Option<(i32, i32)> {
    let caret = api.caret_offset(node).unwrap_or(0).max(0);
    let (x, y, width, height) = api.character_extents(node, (caret - 1).max(0)).ok()?;
    // A field that has not laid its text out answers all zeros: that is no position.
    if x == 0 && y == 0 && width == 0 && height == 0 {
        return None;
    }
    Some((x + if caret > 0 { width } else { 0 }, y + height))
}

/// The caret of the focused text field, in screen coordinates. With a process id only the applications of that process
/// are searched, as the Python does when the backend knows the window's process; the search is bounded.
pub fn focused_caret_anchor(api: &dyn AtspiApi, window_pid: u32) -> Option<(i32, i32)> {
    if std::env::var("GTK_A11Y").is_ok_and(|value| value.trim().eq_ignore_ascii_case("none")) {
        return None;
    }
    let mut pending: Vec<Option<NodeRef>> = Vec::new();
    if window_pid > 0 {
        let top = api.child_count(None).ok()?.min(MAX_CARET_SEARCH_CHILDREN);
        for index in (0..top).rev() {
            if let Ok(Some(application)) = api.child_at(None, index)
                && api.process_id(&application).is_ok_and(|process| process == window_pid)
            {
                pending.push(Some(application));
            }
        }
    } else {
        pending.push(None);
    }
    let mut visited = 0;
    while let Some(node) = pending.pop() {
        if visited >= MAX_CARET_SEARCH_NODES {
            break;
        }
        visited += 1;
        if let Some(node) = &node
            && api.states(node).is_ok_and(|states| states >> ATSPI_STATE_FOCUSED & 1 == 1)
            && api.has_interface(node, INTERFACE_TEXT).unwrap_or(false)
        {
            return caret_anchor_for_node(api, node);
        }
        let count = api.child_count(node.as_ref()).unwrap_or(0).min(MAX_CARET_SEARCH_CHILDREN);
        for index in (0..count).rev() {
            if let Ok(Some(child)) = api.child_at(node.as_ref(), index) {
                pending.push(Some(child));
            }
        }
    }
    None
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;
    use crate::atspi_context::{AtspiError, AtspiResult};

    struct Tree;

    fn node(path: &str) -> NodeRef {
        NodeRef { bus_name: ":1.1".to_owned(), path: path.to_owned() }
    }

    fn missing<T>() -> AtspiResult<T> {
        Err(AtspiError::Call("not in the fake".to_owned()))
    }

    impl AtspiApi for Tree {
        fn child_count(&self, parent: Option<&NodeRef>) -> AtspiResult<usize> {
            Ok(match parent.map(|parent| parent.path.as_str()) {
                None => 2,
                Some("/app") => 2,
                _ => 0,
            })
        }
        fn child_at(&self, parent: Option<&NodeRef>, index: usize) -> AtspiResult<Option<NodeRef>> {
            Ok(Some(match (parent.map(|parent| parent.path.as_str()), index) {
                (None, 0) => node("/other"),
                (None, _) => node("/app"),
                (Some(_), 0) => node("/label"),
                (Some(_), _) => node("/field"),
            }))
        }
        fn name(&self, _: &NodeRef) -> AtspiResult<String> {
            missing()
        }
        fn process_id(&self, application: &NodeRef) -> AtspiResult<u32> {
            Ok(if application.path == "/app" { 42 } else { 7 })
        }
        fn states(&self, field: &NodeRef) -> AtspiResult<u64> {
            Ok(if field.path == "/field" { 1 << ATSPI_STATE_FOCUSED } else { 0 })
        }
        fn role(&self, _: &NodeRef) -> AtspiResult<u32> {
            missing()
        }
        fn has_interface(&self, field: &NodeRef, _: &str) -> AtspiResult<bool> {
            Ok(field.path == "/field")
        }
        fn n_selections(&self, _: &NodeRef) -> AtspiResult<i32> {
            missing()
        }
        fn caret_offset(&self, _: &NodeRef) -> AtspiResult<i32> {
            Ok(3)
        }
        fn character_count(&self, _: &NodeRef) -> AtspiResult<i32> {
            missing()
        }
        fn get_text(&self, _: &NodeRef, _: i32, _: i32) -> AtspiResult<String> {
            missing()
        }
        fn get_selection(&self, _: &NodeRef, _: i32) -> AtspiResult<(i32, i32)> {
            missing()
        }
        fn set_selection(&self, _: &NodeRef, _: i32, _: i32, _: i32) -> AtspiResult<bool> {
            missing()
        }
        fn delete_text(&self, _: &NodeRef, _: i32, _: i32) -> AtspiResult<bool> {
            missing()
        }
        fn insert_text(&self, _: &NodeRef, _: i32, _: &str, _: i32) -> AtspiResult<bool> {
            missing()
        }
        fn character_extents(&self, _: &NodeRef, offset: i32) -> AtspiResult<(i32, i32, i32, i32)> {
            assert_eq!(offset, 2, "the character before the caret");
            Ok((100, 200, 8, 16))
        }
    }

    #[test]
    fn the_caret_is_the_end_of_the_character_before_it() {
        assert_eq!(focused_caret_anchor(&Tree, 42), Some((108, 216)));
        assert_eq!(focused_caret_anchor(&Tree, 0), Some((108, 216)));
        // Another process owns the window: its applications only.
        assert_eq!(focused_caret_anchor(&Tree, 9), None);
    }
}
