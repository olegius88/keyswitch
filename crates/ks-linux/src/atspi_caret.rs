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

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tree_tests {
    use std::collections::HashMap;

    use super::*;
    use crate::atspi_context::{AtspiError, AtspiResult};

    /// A tree of named nodes; a node not named has no children, and a name in `gone` answers no child.
    #[derive(Default)]
    struct Tree {
        children: HashMap<String, Vec<String>>,
        focused: Vec<String>,
        texts: Vec<String>,
        pids: HashMap<String, u32>,
        extents: (i32, i32, i32, i32),
        caret: i32,
        gone: Vec<String>,
        all_children: Option<usize>,
    }

    fn node(name: &str) -> NodeRef {
        NodeRef { bus_name: ":1".to_owned(), path: name.to_owned() }
    }

    fn name_of(node: Option<&NodeRef>) -> String {
        node.map_or_else(|| "desktop".to_owned(), |node| node.path.clone())
    }

    impl AtspiApi for Tree {
        fn child_count(&self, parent: Option<&NodeRef>) -> AtspiResult<usize> {
            Ok(self.all_children.unwrap_or_else(|| self.children.get(&name_of(parent)).map_or(0, Vec::len)))
        }
        fn child_at(&self, parent: Option<&NodeRef>, index: usize) -> AtspiResult<Option<NodeRef>> {
            let name = name_of(parent);
            if self.gone.contains(&name) {
                return Err(AtspiError::Call("gone".to_owned()));
            }
            if self.all_children.is_some() {
                return Ok(Some(node(&format!("{name}.{index}"))));
            }
            Ok(self.children.get(&name).and_then(|children| children.get(index)).map(|child| node(child)))
        }
        fn name(&self, _node: &NodeRef) -> AtspiResult<String> {
            Ok(String::new())
        }
        fn process_id(&self, node: &NodeRef) -> AtspiResult<u32> {
            Ok(self.pids.get(&node.path).copied().unwrap_or(0))
        }
        fn states(&self, node: &NodeRef) -> AtspiResult<u64> {
            Ok(u64::from(self.focused.contains(&node.path)) << ATSPI_STATE_FOCUSED)
        }
        fn role(&self, _node: &NodeRef) -> AtspiResult<u32> {
            Ok(0)
        }
        fn has_interface(&self, node: &NodeRef, _interface: &str) -> AtspiResult<bool> {
            Ok(self.texts.contains(&node.path))
        }
        fn n_selections(&self, _node: &NodeRef) -> AtspiResult<i32> {
            Ok(0)
        }
        fn caret_offset(&self, _node: &NodeRef) -> AtspiResult<i32> {
            Ok(self.caret)
        }
        fn character_count(&self, _node: &NodeRef) -> AtspiResult<i32> {
            Ok(0)
        }
        fn get_text(&self, _node: &NodeRef, _start: i32, _end: i32) -> AtspiResult<String> {
            Ok(String::new())
        }
        fn get_selection(&self, _node: &NodeRef, _number: i32) -> AtspiResult<(i32, i32)> {
            Ok((0, 0))
        }
        fn set_selection(&self, _node: &NodeRef, _number: i32, _start: i32, _end: i32) -> AtspiResult<bool> {
            Ok(false)
        }
        fn delete_text(&self, _node: &NodeRef, _start: i32, _end: i32) -> AtspiResult<bool> {
            Ok(false)
        }
        fn insert_text(&self, _node: &NodeRef, _position: i32, _text: &str, _length: i32) -> AtspiResult<bool> {
            Ok(false)
        }
        fn character_extents(&self, _node: &NodeRef, _offset: i32) -> AtspiResult<(i32, i32, i32, i32)> {
            Ok(self.extents)
        }
    }

    fn tree() -> Tree {
        let mut tree = Tree::default();
        tree.children.insert("desktop".to_owned(), vec!["other".to_owned(), "app".to_owned()]);
        tree.children.insert("app".to_owned(), vec!["label".to_owned(), "field".to_owned()]);
        tree.focused = vec!["label".to_owned(), "field".to_owned()];
        tree.texts = vec!["field".to_owned()];
        tree.pids.insert("app".to_owned(), 7);
        tree.extents = (10, 20, 3, 4);
        tree
    }

    #[test]
    fn the_caret_stands_after_the_character_before_it() {
        let mut tree = tree();
        assert_eq!(focused_caret_anchor(&tree, 0), Some((10, 24)), "at the start of the text it stands before the first character");
        tree.caret = 2;
        assert_eq!(focused_caret_anchor(&tree, 0), Some((13, 24)));
    }

    #[test]
    fn a_field_that_has_not_laid_its_text_out_has_no_caret() {
        let mut tree = tree();
        for extents in [(0, 0, 0, 0), (1, 0, 0, 0), (0, 1, 0, 0), (0, 0, 1, 0), (0, 0, 0, 1)] {
            tree.extents = extents;
            assert_eq!(focused_caret_anchor(&tree, 0).is_none(), extents == (0, 0, 0, 0));
        }
    }

    #[test]
    fn a_window_process_narrows_the_search_to_its_applications() {
        let tree = tree();
        assert_eq!(focused_caret_anchor(&tree, 7), Some((10, 24)));
        assert_eq!(focused_caret_anchor(&tree, 8), None);
        let mut broken = self::tree();
        broken.gone = vec!["desktop".to_owned()];
        assert_eq!(focused_caret_anchor(&broken, 7), None);
        assert_eq!(focused_caret_anchor(&broken, 0), None);
    }

    #[test]
    fn a_tree_too_big_to_walk_ends_the_search() {
        let tree = Tree { all_children: Some(MAX_CARET_SEARCH_CHILDREN), ..Tree::default() };
        assert_eq!(focused_caret_anchor(&tree, 0), None);
    }

    #[test]
    fn a_session_with_accessibility_turned_off_is_not_asked() {
        const CHILD: &str = "KS_CARET_CHILD";
        if std::env::var_os(CHILD).is_some() {
            assert_eq!(focused_caret_anchor(&tree(), 0), None);
            return;
        }
        let status = std::process::Command::new(std::env::current_exe().expect("this test"))
            .args(["--exact", "atspi_caret::tree_tests::a_session_with_accessibility_turned_off_is_not_asked"])
            .env(CHILD, "1")
            .env("GTK_A11Y", " None ")
            .status()
            .expect("a child");
        assert!(status.success());
    }
}
