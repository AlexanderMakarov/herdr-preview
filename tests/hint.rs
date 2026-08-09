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
fn headless_list_mode_prints_targets() {
    let root = temp_fixture("list");
    let cwd = root.join("repo");
    fs::create_dir_all(cwd.join("docs")).unwrap();
    fs::write(cwd.join("docs/plan.md"), "# plan\n").unwrap();

    let text = "see docs/plan.md and docs/missing.md\nhttps://github.com/org/repo/pull/1\n";
    let output = run_hint_list(text, &cwd).expect("list");

    assert!(output.contains("docs/plan.md"));
    assert!(output.contains("https://github.com/org/repo/pull/1"));
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
    fs::create_dir_all(&cwd).unwrap();
    let text = "https://example.com\n";
    let entries = build_entries(text, &cwd);
    assert_eq!(serialize_entries(&entries), format_list(&entries));
}
