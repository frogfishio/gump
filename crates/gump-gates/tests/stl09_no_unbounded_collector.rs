//! STL-09 evidence: server must not construct the unbounded CallbackAdapter.

use std::fs;
use std::path::{Path, PathBuf};

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("crates/gump-gates → workspace root")
        .to_path_buf()
}

fn collect_rs(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_rs(&path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            out.push(path);
        }
    }
}

#[test]
fn gump_server_does_not_construct_unbounded_callback_adapter() {
    let root = workspace_root().join("crates/gump-server");
    let mut files = Vec::new();
    collect_rs(&root, &mut files);
    assert!(!files.is_empty(), "expected gump-server sources");

    let banned = [
        "CallbackAdapter::new",
        "CallbackAdapter::",
        "SharedCallbackAdapter",
        "CollectingCallbackAdapter",
    ];
    let mut hits = Vec::new();
    for path in &files {
        let text = fs::read_to_string(path).unwrap_or_default();
        for needle in banned {
            if text.contains(needle) {
                hits.push(format!("{}: {needle}", path.display()));
            }
        }
    }
    assert!(
        hits.is_empty(),
        "gump-server must not wire the unbounded collecting adapter: {hits:?}"
    );
}
