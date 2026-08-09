use herdr_preview::hint::{build_entries, format_list, run_hint_list, serialize_entries, HINT_KEYS};
use std::fs;
use std::path::PathBuf;

fn temp_fixture(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "herdr-preview-hint-it-{}-{}",
        std::process::id(),
        name
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create fixture dir");
    dir
}

#[test]
fn headless_list_mode_prints_path_targets_not_urls() {
    let root = temp_fixture("list");
    let cwd = root.join("repo");
    fs::create_dir_all(cwd.join("docs")).unwrap();
    fs::write(cwd.join("docs/plan.md"), "# plan\n").unwrap();

    let text = "see docs/plan.md and docs/missing.md\nhttps://github.com/org/repo/pull/1\n";
    let output = run_hint_list(text, &cwd).expect("list");

    assert!(output.contains("docs/plan.md"));
    assert!(!output.contains("https://github.com/org/repo/pull/1"));
    assert!(!output.contains("missing.md"));
}

#[test]
fn manifest_has_no_link_handlers() {
    let manifest = fs::read_to_string("herdr-plugin.toml").expect("read manifest");
    assert!(
        !manifest.contains("[[link_handlers]]"),
        "manifest must not register link_handlers"
    );
}

#[test]
fn manifest_declares_hint_action_and_panes() {
    let manifest = fs::read_to_string("herdr-plugin.toml").expect("read manifest");
    assert!(manifest.contains("id = \"hint\""));
    assert!(manifest.contains("id = \"hint-overlay\""));
    assert!(manifest.contains("id = \"less\""));
}

#[test]
fn hint_keys_are_unique_and_exclude_cancel() {
    assert_eq!(HINT_KEYS.len(), 25);
    assert!(!HINT_KEYS.contains('q'));
}

#[test]
fn serialize_matches_format_list() {
    let root = temp_fixture("serialize");
    let cwd = root.join("repo");
    fs::create_dir_all(cwd.join("src")).unwrap();
    fs::write(cwd.join("src/a.rs"), "x\n").unwrap();
    let text = "src/a.rs\n";
    let entries = build_entries(text, &cwd);
    assert!(!entries.is_empty());
    assert_eq!(serialize_entries(&entries), format_list(&entries));
}

#[test]
fn builds_hint_for_path_only_in_visible_worktree() {
    let root = temp_fixture("wt-hint");
    let cwd = root.join("repo");
    let wt = cwd.join(".claude/worktrees/feat-109-explain-the-product");
    fs::create_dir_all(wt.join("context/spec/008-make-product-explain-itself")).unwrap();
    let rel = "context/spec/008-make-product-explain-itself/technical-considerations.md";
    fs::write(wt.join(rel), "# tech\n").unwrap();

    // Relative path appears BEFORE the worktree dir on screen (common agent layout).
    let text = format!(
        "Approval gate ready.\n\n  {rel}\n\n  worktree: {wt_disp}\n",
        wt_disp = wt.display()
    );
    let entries = build_entries(&text, &cwd);
    let hit = entries
        .iter()
        .find(|e| e.raw == rel)
        .expect("relative path in worktree should be hinted");
    match &hit.target {
        herdr_preview::classify::Target::File { open_spec, path } => {
            assert_eq!(
                open_spec,
                &format!(
                    ".claude/worktrees/feat-109-explain-the-product/{rel}"
                )
            );
            assert_eq!(path, &wt.join(rel));
        }
        other => panic!("expected file target, got {other:?}"),
    }
}
