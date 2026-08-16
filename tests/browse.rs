use herdr_preview::browse::{
    click_command, map_browse_key, parse_browse_input, render_browse, BrowseCommand, BrowseKey,
    BrowseOutcome, BrowseRow, BrowseState,
};
use std::fs;
use std::os::unix::fs::{self as unix_fs, PermissionsExt};
use std::path::PathBuf;

fn fixture(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "herdr-preview-browse-{}-{}",
        std::process::id(),
        name
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    // macOS temp_dir is `/var/folders/...`; canonicalize is `/private/var/...`.
    dir.canonicalize().unwrap_or(dir)
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
fn lists_symlink_to_file_and_symlink_to_dir() {
    let root = fixture("symlinks");
    fs::create_dir_all(root.join("realdir")).unwrap();
    fs::write(root.join("realfile.txt"), "x\n").unwrap();
    unix_fs::symlink(root.join("realdir"), root.join("linkdir")).unwrap();
    unix_fs::symlink(root.join("realfile.txt"), root.join("linkfile")).unwrap();

    let state = BrowseState::open(&root);
    assert!(
        state
            .rows
            .iter()
            .any(|r| matches!(r, BrowseRow::Dir { name, .. } if name == "linkdir")),
        "symlink-to-dir should list as Dir, got {:?}",
        state.rows
    );
    assert!(
        state
            .rows
            .iter()
            .any(|r| matches!(r, BrowseRow::File { name, .. } if name == "linkfile")),
        "symlink-to-file should list as File, got {:?}",
        state.rows
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
    assert_eq!(
        state.apply(BrowseCommand::Activate, 10),
        BrowseOutcome::Continue
    );
    assert_eq!(state.cwd, root.join("docs"));
    assert!(state
        .rows
        .iter()
        .any(|r| matches!(r, BrowseRow::File { name, .. } if name == "plan.md")));
    assert_eq!(
        state.apply(BrowseCommand::GoParent, 10),
        BrowseOutcome::Continue
    );
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
    assert_eq!(
        state.apply(BrowseCommand::Activate, 10),
        BrowseOutcome::Continue
    );
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

#[test]
fn render_shows_path_parent_dirs_files_and_legend() {
    let root = fixture("render");
    fs::create_dir_all(root.join("docs")).unwrap();
    fs::write(root.join("a.rs"), "x\n").unwrap();
    let state = BrowseState::open(&root);
    let out = render_browse(&state, 8, 80);
    assert!(out.contains(&root.display().to_string()) || out.contains("browse-"));
    assert!(out.contains(".."));
    assert!(out.contains("docs/"));
    assert!(out.contains("a.rs"));
    assert!(out.contains("browse ·"));
}

#[test]
fn render_warns_when_listing_is_outside_origin_cwd() {
    let root = fixture("outside-tab");
    fs::create_dir_all(root.join("here")).unwrap();
    fs::create_dir_all(root.join("elsewhere")).unwrap();
    fs::write(root.join("elsewhere/a.rs"), "x\n").unwrap();
    let mut state = BrowseState::open(&root.join("elsewhere"));
    state.origin_cwd = Some(root.join("here"));
    let out = render_browse(&state, 8, 80);
    assert!(out.contains("outside this tab's cwd"));
    assert!(out.contains("new tab"));
}

#[test]
fn render_keeps_legend_when_listing_is_under_origin_cwd() {
    let root = fixture("inside-tab");
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("src/a.rs"), "x\n").unwrap();
    let mut state = BrowseState::open(&root.join("src"));
    state.origin_cwd = Some(root.clone());
    let out = render_browse(&state, 8, 80);
    assert!(out.contains("browse ·"));
    assert!(!out.contains("outside this tab's cwd"));
}

#[test]
fn map_keys_match_spec() {
    assert_eq!(
        map_browse_key(BrowseKey::Char('k')),
        Some(BrowseCommand::MoveUp)
    );
    assert_eq!(map_browse_key(BrowseKey::Up), Some(BrowseCommand::MoveUp));
    assert_eq!(
        map_browse_key(BrowseKey::Char('j')),
        Some(BrowseCommand::MoveDown)
    );
    assert_eq!(
        map_browse_key(BrowseKey::Down),
        Some(BrowseCommand::MoveDown)
    );
    assert_eq!(
        map_browse_key(BrowseKey::Char('h')),
        Some(BrowseCommand::GoParent)
    );
    assert_eq!(
        map_browse_key(BrowseKey::Left),
        Some(BrowseCommand::GoParent)
    );
    assert_eq!(
        map_browse_key(BrowseKey::Char('l')),
        Some(BrowseCommand::EnterDir)
    );
    assert_eq!(
        map_browse_key(BrowseKey::Right),
        Some(BrowseCommand::EnterDir)
    );
    assert_eq!(
        map_browse_key(BrowseKey::Enter),
        Some(BrowseCommand::Activate)
    );
    assert_eq!(
        map_browse_key(BrowseKey::Char('q')),
        Some(BrowseCommand::Dismiss)
    );
    assert_eq!(map_browse_key(BrowseKey::Esc), Some(BrowseCommand::Dismiss));
    assert_eq!(
        map_browse_key(BrowseKey::MouseWheelUp),
        Some(BrowseCommand::ScrollUp)
    );
    assert_eq!(
        map_browse_key(BrowseKey::MouseWheelDown),
        Some(BrowseCommand::ScrollDown)
    );
}

#[test]
fn parse_arrows_and_sgr_mouse() {
    assert_eq!(parse_browse_input(&[b'j']), Some(BrowseKey::Char('j')));
    assert_eq!(parse_browse_input(&[0x0d]), Some(BrowseKey::Enter));
    assert_eq!(parse_browse_input(&[0x1b]), Some(BrowseKey::Esc));
    assert_eq!(parse_browse_input(b"\x1b[A"), Some(BrowseKey::Up));
    assert_eq!(parse_browse_input(b"\x1b[B"), Some(BrowseKey::Down));
    assert_eq!(parse_browse_input(b"\x1b[C"), Some(BrowseKey::Right));
    assert_eq!(parse_browse_input(b"\x1b[D"), Some(BrowseKey::Left));
    assert_eq!(
        parse_browse_input(b"\x1b[<0;1;3M"),
        Some(BrowseKey::MouseClick { row: 2, col: 0 })
    );
    assert_eq!(
        parse_browse_input(b"\x1b[<64;1;1M"),
        Some(BrowseKey::MouseWheelUp)
    );
    assert_eq!(
        parse_browse_input(b"\x1b[<65;1;1M"),
        Some(BrowseKey::MouseWheelDown)
    );
}

#[test]
fn parse_concatenated_sgr_wheel_burst_yields_first_event() {
    let burst = b"\x1b[<64;1;1M\x1b[<64;1;1M\x1b[<65;1;1M";
    assert_eq!(parse_browse_input(burst), Some(BrowseKey::MouseWheelUp));
}

#[test]
fn click_on_list_row_selects_index() {
    let root = fixture("click");
    fs::write(root.join("a.rs"), "x\n").unwrap();
    let state = BrowseState::open(&root);
    // row 0 header; row 1 is `..` (index 0); next child starts at row 2
    assert_eq!(click_command(&state, 0, 8), None);
    assert_eq!(click_command(&state, 7, 8), None);
    assert_eq!(
        click_command(&state, 1, 8),
        Some(BrowseCommand::SelectIndex(0))
    );
}

#[test]
fn manifest_declares_browse_pane() {
    let manifest = std::fs::read_to_string("herdr-plugin.toml").unwrap();
    assert!(manifest.contains("id = \"browse\""));
    assert!(!manifest.contains("[[link_handlers]]"));
}
