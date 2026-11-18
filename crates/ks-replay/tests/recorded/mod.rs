//! The tree a recorded trace replays in: this workspace, with the context pair the traces were recorded with in place of
//! the pair the program ships. A trace replays the engine around that pair's answers, and its start line names it; the
//! program's own tests and batteries check the shipped pair. The recorded pair is kept in the workspace as the frozen base
//! of the pairs fitted onto it.
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// The artifact of the pair the traces were recorded with (`engine_initialized.context_model_status`).
const RECORDED_PAIR: &str = "model/context_v3/frozen-base-48a863e75bb6.json";

/// The tree, made once a test program: links to the workspace's files, and the recorded pair as the installed one. The
/// trace tool finds it by `KS_REPLAY_TREE`.
pub(crate) fn tree() -> PathBuf {
    static TREE: OnceLock<PathBuf> = OnceLock::new();
    TREE.get_or_init(|| {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().expect("the workspace");
        let tree = std::env::temp_dir().join(format!("ks-recorded-tree-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tree);
        std::fs::create_dir_all(tree.join("resources/models")).expect("a folder");
        let link = |from: PathBuf, to: PathBuf| std::os::unix::fs::symlink(from, to).expect("a link");
        for (from, to, kept) in [("", "", "resources"), ("resources", "resources", "models"), ("resources/models", "resources/models", "context_policy_v1.json")] {
            for entry in std::fs::read_dir(workspace.join(from)).expect("a folder of the workspace").flatten() {
                if entry.file_name() != kept {
                    link(entry.path(), tree.join(to).join(entry.file_name()));
                }
            }
        }
        link(workspace.join(RECORDED_PAIR), tree.join("resources/models/context_policy_v1.json"));
        tree
    })
    .clone()
}
