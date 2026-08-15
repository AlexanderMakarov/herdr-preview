use herdr_preview::browse::{BrowseCommand, BrowseOutcome, BrowseRow, BrowseState};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

fn fixture(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "herdr-preview-browse-{}-{}",
        std::process::id(),
        name
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn lists_parent_dirs_then_files_case_insensitive_including_dots() {
    let root = fixture("sort");
    fs::create_dir_all(root.join("docs")).unwrap();
    fs::create_dir_all(root.join(".git")).unwrap();
    fs::write(root.join("README.md"), "x\n").unwrap();
    fs::write(root.join(".hidden"), "x\n").unwrap();
    fs::write(root.join("Zed.txt"), "x\n").unwrap();
    fs::write(root.join("alpha.txt"), "x\n").unwrap();

    let state = BrowseState::open(&root);
    let names: Vec<String> = state.rows.iter().map(|r| r.display_name()).collect();
    assert_eq!(names[0], "..");
    let children = &names[1..];
    assert_eq!(
        children,
        &[
            ".git/".to_string(),
            "docs/".to_string(),
            ".hidden".to_string(),
            "alpha.txt".to_string(),
            "README.md".to_string(),
            "Zed.txt".to_string(),
        ]
    );
}

#[test]
fn empty_directory_shows_parent_and_placeholder() {
    let root = fixture("empty-parent");
    let empty = root.join("empty");
    fs::create_dir_all(&empty).unwrap();
    let state = BrowseState::open(&empty);
    assert!(matches!(state.rows[0], BrowseRow::Parent { .. }));
    assert!(matches!(state.rows[1], BrowseRow::EmptyPlaceholder));
    assert_eq!(state.rows[1].display_name(), "(empty)");
    assert!(!state.rows[1].is_activatable());
}

#[test]
fn drill_in_and_parent_walk() {
    let root = fixture("walk");
    fs::create_dir_all(root.join("docs/sub")).unwrap();
    fs::write(root.join("docs/plan.md"), "#\n").unwrap();
    let mut state = BrowseState::open(&root);
    let docs = state
        .rows
        .iter()
        .position(|r| matches!(r, BrowseRow::Dir { name, .. } if name == "docs"))
        .unwrap();
    state.selected = docs;
    assert_eq!(state.apply(BrowseCommand::Activate, 10), BrowseOutcome::Continue);
    assert_eq!(state.cwd, root.join("docs"));
    assert!(state.rows.iter().any(|r| matches!(r, BrowseRow::File { name, .. } if name == "plan.md")));
    assert_eq!(state.apply(BrowseCommand::GoParent, 10), BrowseOutcome::Continue);
    assert_eq!(state.cwd, root);
}

#[test]
fn activate_file_returns_open_file() {
    let root = fixture("file");
    fs::write(root.join("a.rs"), "fn main() {}\n").unwrap();
    let mut state = BrowseState::open(&root);
    let idx = state
        .rows
        .iter()
        .position(|r| matches!(r, BrowseRow::File { name, .. } if name == "a.rs"))
        .unwrap();
    state.selected = idx;
    match state.apply(BrowseCommand::Activate, 10) {
        BrowseOutcome::OpenFile { path } => assert_eq!(path, root.join("a.rs")),
        other => panic!("expected OpenFile, got {other:?}"),
    }
}

#[test]
fn unreadable_dir_keeps_previous_listing() {
    let root = fixture("unreadable");
    fs::create_dir_all(root.join("ok")).unwrap();
    let blocked = root.join("secret");
    fs::create_dir_all(&blocked).unwrap();
    fs::set_permissions(&blocked, fs::Permissions::from_mode(0o000)).unwrap();
    let mut state = BrowseState::open(&root);
    let before = state.rows.clone();
    let idx = state
        .rows
        .iter()
        .position(|r| matches!(r, BrowseRow::Dir { name, .. } if name == "secret"))
        .unwrap();
    state.selected = idx;
    assert_eq!(state.apply(BrowseCommand::Activate, 10), BrowseOutcome::Continue);
    assert_eq!(state.rows, before);
    assert_eq!(state.notice.as_deref(), Some("cannot read directory"));
    let _ = fs::set_permissions(&blocked, fs::Permissions::from_mode(0o755));
}

#[test]
fn move_and_scroll_keep_selection_visible() {
    let root = fixture("scroll");
    for i in 0..20 {
        fs::write(root.join(format!("f{i:02}.txt")), "x\n").unwrap();
    }
    let mut state = BrowseState::open(&root);
    for _ in 0..10 {
        state.apply(BrowseCommand::MoveDown, 5);
    }
    assert!(state.selected >= state.scroll);
    assert!(state.selected < state.scroll + 5);
}
