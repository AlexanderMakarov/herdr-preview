# Directory Browse Overlay Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Hint-picking a directory opens a herdr-preview-owned browse overlay so the user can drill to a file with arrows/Enter or a mouse click, then open that file in herdr-file-viewer or `less`.

**Architecture:** Add `src/browse.rs` for listing, sort, navigation, and TUI. Directory hint picks spawn a new `browse` overlay pane (`herdr-preview browse`) with start-path + origin pane env. Choosing a file dismisses browse and reuses existing `open_file_viewer` / `open_less`. Remove `OpenRoute::DirSkip`.

**Tech Stack:** Rust 2021, existing ANSI/raw-TTY overlay pattern from `src/hint.rs` (no new TUI crate), Herdr plugin manifest panes, `less` / herdr-file-viewer peer openers.

**Spec:** `docs/superpowers/specs/2026-08-12-directory-browse-design.md` (amends `docs/superpowers/specs/2026-08-09-herdr-preview-design.md`; implements [issue #2](https://github.com/AlexanderMakarov/herdr-preview/issues/2) via browse overlay, **not** FV directory OPEN).

## Global Constraints

- Platforms MVP: Linux + macOS only.
- Scan source: focused pane **visible** text only (not full scrollback).
- Never add `[[link_handlers]]` or git-host / `gh pr view` Ctrl+click hijacks.
- Prefer herdr-file-viewer for **file** opens; fallback `less` overlay with reduced functionality (documented in README).
- Directory picks always open the browse overlay (with or without FV). Do **not** summon FV on a directory. Do **not** DirSkip.
- Path open shapes for FV: `path`, `path:N`, `path:A-B` only.
- Tokenization must handle spaces in paths and `%20`.
- README problem-first; quicklook credit only at bottom.
- Do not implement large-file / glow performance policy.
- Do not add search/filter, git status coloring, icons, hide-dot toggle, recursive flat lists, or an in-plugin file manager.
- License: MIT.
- Follow the approved directory-browse spec: `docs/superpowers/specs/2026-08-12-directory-browse-design.md`.

---

## File map

| Path | Responsibility |
| --- | --- |
| `src/browse.rs` | `BrowseState` list/sort/nav; render to a string; key/mouse mapping; overlay loop |
| `tests/browse.rs` | Unit tests for listing, sort, nav, empty/unreadable, render, input map (no real TTY) |
| `src/open.rs` | `open_browse` — `herdr plugin pane open` browse overlay with env |
| `src/hint.rs` | `OpenRoute::Browse`; `open_entry` Dir → `open_browse`; `open_preview_file` for browse file picks |
| `src/lib.rs` | `pub mod browse` |
| `src/main.rs` | `herdr-preview browse` subcommand |
| `herdr-plugin.toml` | `[[panes]]` id `browse`; still **no** `[[link_handlers]]` |
| `tests/routing.rs` | Dir pick opens browse pane; file pick from browse → FV/`less`; file/URL unchanged |
| `README.md` | Directory pick → browse → file → FV/`less` |
| `docs/superpowers/specs/2026-08-09-herdr-preview-design.md` | Directory rows now browse overlay, not FV OPEN / DirSkip |

---

### Task 1: BrowseState list, sort, navigate (TDD)

**Files:**
- Create: `src/browse.rs`, `tests/browse.rs`
- Modify: `src/lib.rs` (add `pub mod browse;`)

**Interfaces:**
- Consumes: filesystem paths only
- Produces:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BrowseRow {
    Parent { path: PathBuf },
    Dir { name: String, path: PathBuf },
    File { name: String, path: PathBuf },
    EmptyPlaceholder,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowseState {
    pub cwd: PathBuf,
    pub rows: Vec<BrowseRow>,
    pub selected: usize,
    pub scroll: usize,
    pub notice: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BrowseCommand {
    MoveUp,
    MoveDown,
    GoParent,
    EnterDir,
    Activate,
    ScrollUp,
    ScrollDown,
    SelectIndex(usize),
    Dismiss,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BrowseOutcome {
    Continue,
    OpenFile { path: PathBuf },
    Dismiss,
}

impl BrowseState {
    pub fn open(path: &Path) -> Self;
    pub fn apply(&mut self, cmd: BrowseCommand, visible_rows: usize) -> BrowseOutcome;
}

impl BrowseRow {
    pub fn display_name(&self) -> String; // `..`, `name/`, `name`, `(empty)`
    pub fn is_activatable(&self) -> bool; // false only for EmptyPlaceholder
}
```

- Listing rules: `read_dir` immediate children only; include dot entries; never include `.` / `..` from `read_dir` (Unix `read_dir` already omits them). Prepend `BrowseRow::Parent` when `path.parent()` is `Some`. Sort children: directories first, then files, case-insensitive by `name`. If there are no children, append `EmptyPlaceholder`.
- `open`: canonicalize when possible; `selected = 0`, `scroll = 0`, `notice = None`. If the start path is unreadable, `rows` is only `Parent` (if any) plus `EmptyPlaceholder`, and `notice` is `cannot read directory`.
- `apply` navigation: `MoveUp`/`MoveDown` change `selected` (clamp); then `ensure_visible(visible_rows)` so `scroll <= selected < scroll + visible_rows`. `GoParent` / `Activate` on `Parent` / `EnterDir` on a dir: if `read_dir` succeeds, replace listing (clear notice); if it fails, **keep previous listing** and set `notice = Some("cannot read directory".into())`. `Activate` on file → `OpenFile`. `Activate` on placeholder → `Continue`. `EnterDir` on a file or placeholder → `Continue`. `Dismiss` → `Dismiss`. `SelectIndex` clamps then continues (Activate is a separate command). `ScrollUp`/`ScrollDown` change `scroll` only, clamped to the list.

- [ ] **Step 1: Write failing tests** in `tests/browse.rs`:

```rust
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
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --test browse`
Expected: FAIL compiling (`no browse module` / `BrowseState not found`).

- [ ] **Step 3: Implement `src/browse.rs`** with the types above and:

```rust
impl BrowseState {
    pub fn open(path: &Path) -> Self {
        let cwd = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        match list_rows(&cwd) {
            Ok(rows) => Self { cwd, rows, selected: 0, scroll: 0, notice: None },
            Err(_) => Self {
                cwd: cwd.clone(),
                rows: fallback_rows(&cwd),
                selected: 0,
                scroll: 0,
                notice: Some("cannot read directory".into()),
            },
        }
    }

    pub fn apply(&mut self, cmd: BrowseCommand, visible_rows: usize) -> BrowseOutcome { /* as Interfaces */ }

    fn ensure_visible(&mut self, visible_rows: usize) { /* scroll so selected is in view; visible_rows max(1) */ }
}

fn list_rows(path: &Path) -> io::Result<Vec<BrowseRow>> { /* parent + sorted children + optional placeholder */ }

fn try_enter(&mut self, path: &Path) {
    match list_rows(path) {
        Ok(rows) => {
            self.cwd = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
            self.rows = rows;
            self.selected = 0;
            self.scroll = 0;
            self.notice = None;
        }
        Err(_) => self.notice = Some("cannot read directory".into()),
    }
}
```

Do not add TUI/render in this task.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --test browse`
Expected: PASS (all tests in this file). Then `cargo test` — existing suite still passes.

- [ ] **Step 5: Commit**

```bash
git add src/browse.rs src/lib.rs tests/browse.rs
git commit -m "$(cat <<'EOF'
feat: add directory browse listing and navigation state

EOF
)"
```

---

### Task 2: Route directory picks to browse overlay (TDD)

**Files:**
- Modify: `src/hint.rs` (`OpenRoute`, `route_entry`, `open_entry`, remove `DirSkip` / `DIR_SKIP_NOTICE`)
- Modify: `src/open.rs` (add `open_browse`)
- Modify: `tests/routing.rs` (replace dir-skip test; add browse spawn + file-from-browse tests)
- Modify: `src/hint.rs` unit test `route_entry_picks_backend_from_target_and_peer_detect` (dir → `Browse`, not `DirSkip`)

**Interfaces:**
- Consumes: `Target::Dir { path, .. }`, origin pane id, origin cwd
- Produces:

```rust
// src/hint.rs
pub enum OpenRoute {
    Browser,
    FileViewer,
    Less,
    Browse, // replaces DirSkip
    NoOp,
}

pub fn route_entry(entry: &HintEntry, herdr_bin: &Path) -> OpenRoute;
// Target::Dir => OpenRoute::Browse always (ignore FV detect)

pub fn open_preview_file(
    path: &Path,
    cwd: &Path,
    origin_pane_id: Option<&str>,
) -> Result<(), HintError>;
// classify path against cwd; File => existing FileViewer/Less routing; never Browse/DirSkip

// src/open.rs
pub fn open_browse(
    start: &Path,
    origin_pane_id: Option<&str>,
    origin_cwd: &Path,
) -> io::Result<()>;
```

`open_browse` must invoke (same overlay style as `open_less`):

```text
herdr plugin pane open
  --plugin herdr-preview
  --entrypoint browse
  --placement overlay
  --focus
  --env HERDR_PREVIEW_BROWSE_START=<start>
  --env HERDR_PREVIEW_BROWSE_CWD=<origin_cwd>
  [--env HERDR_PREVIEW_BROWSE_ORIGIN=<origin_pane_id>]  # when Some and non-empty
```

`open_entry` for `OpenRoute::Browse`: call `open_browse(&dir_path, origin_pane_id, cwd)` using `Target::Dir { path, .. }`. Delete `DIR_SKIP_NOTICE` and the notify/eprintln skip path.

`open_preview_file`: `classify(&path.to_string_lossy(), cwd)` then the same FileViewer/Less arms as today's file `open_entry` (reuse `detect_file_viewer` / `open_file_viewer` / `open_less` / `line_from_open_spec`). If classify is not File, return `io::Error` / `HintError` — do not open browse recursively from here.

- [ ] **Step 1: Write failing tests** — replace `fv_absent_skips_directory_with_notice` in `tests/routing.rs` with:

```rust
#[test]
fn directory_pick_opens_browse_overlay_even_without_fv() {
    let root = temp_fixture("browse-dir");
    let list = "1 plugins installed:\n- other (other) enabled\n";
    let herdr = herdr_with_plugin_list(&root, list);
    let cwd = root.join("repo");
    let dir = cwd.join("docs");
    fs::create_dir_all(&dir).unwrap();
    let entry = dir_entry(&dir, "docs/");

    with_env(&root, &herdr, || {
        assert_eq!(route_entry(&entry, &herdr), OpenRoute::Browse);
        open_entry(&entry, &cwd, Some("w1:origin")).expect("open_entry");
    });

    let invocations = read_invocations(&stub_log_path(&root));
    assert_eq!(invocations.len(), 1, "expected browse overlay summon: {invocations:?}");
    let args = &invocations[0];
    assert!(args.windows(2).any(|w| w == ["--plugin", "herdr-preview"]));
    assert!(args.windows(2).any(|w| w == ["--entrypoint", "browse"]));
    assert!(args.contains(&"overlay".to_string()));
    let envs: Vec<_> = args.windows(2).filter(|w| w[0] == "--env").map(|w| w[1].as_str()).collect();
    assert!(envs.iter().any(|e| e.starts_with("HERDR_PREVIEW_BROWSE_START=") && e.contains("docs")));
    assert!(envs.iter().any(|e| *e == "HERDR_PREVIEW_BROWSE_ORIGIN=w1:origin"));
    assert!(envs.iter().any(|e| e.starts_with("HERDR_PREVIEW_BROWSE_CWD=")));
    assert!(!args.contains(&"herdr-file-viewer".to_string()));
    assert!(!args.contains(&"less".to_string()));
}

#[test]
fn directory_pick_opens_browse_not_fv_when_fv_installed() {
    let root = temp_fixture("browse-dir-fv");
    let list = "2 plugins installed:\n- herdr-file-viewer (herdr-file-viewer) enabled\n";
    let herdr = herdr_with_plugin_list(&root, list);
    let cwd = root.join("repo");
    let dir = cwd.join("docs");
    fs::create_dir_all(&dir).unwrap();
    let entry = dir_entry(&dir, "docs/");

    with_env(&root, &herdr, || {
        assert!(detect_file_viewer(&herdr));
        assert_eq!(route_entry(&entry, &herdr), OpenRoute::Browse);
        open_entry(&entry, &cwd, None).expect("open_entry");
    });

    let invocations = read_invocations(&stub_log_path(&root));
    assert!(
        invocations.iter().any(|args| args.windows(2).any(|w| w == ["--entrypoint", "browse"])),
        "expected browse spawn, got {invocations:?}"
    );
    assert!(
        !invocations.iter().any(|args| args.iter().any(|a| a.contains("HERDR_FILE_VIEWER_OPEN"))),
        "must not OPEN a directory in file-viewer: {invocations:?}"
    );
}

#[test]
fn browse_file_pick_routes_to_fv_when_installed() {
    let root = temp_fixture("browse-file-fv");
    let list = "2 plugins installed:\n- herdr-file-viewer (herdr-file-viewer) enabled\n";
    let herdr = herdr_with_plugin_list(&root, list);
    let _gh = fake_gh(&root);
    let cwd = root.join("repo");
    fs::create_dir_all(&cwd).unwrap();
    let file = cwd.join("src/app.rs");
    fs::create_dir_all(file.parent().unwrap()).unwrap();
    fs::write(&file, "fn main() {}\n").unwrap();

    with_env(&root, &herdr, || {
        herdr_preview::hint::open_preview_file(&file, &cwd, None).expect("open_preview_file");
    });

    let invocations = read_invocations(&stub_log_path(&root));
    assert!(
        invocations.iter().any(|args| args.iter().any(|a| a.contains("HERDR_FILE_VIEWER_OPEN="))),
        "expected FV OPEN, got {invocations:?}"
    );
    assert!(!root.join("gh.log").exists());
}

#[test]
fn browse_file_pick_routes_to_less_when_fv_absent() {
    let root = temp_fixture("browse-file-less");
    let list = "1 plugins installed:\n- other (other) enabled\n";
    let herdr = herdr_with_plugin_list(&root, list);
    let _less = fake_less(&root);
    let cwd = root.join("repo");
    fs::create_dir_all(&cwd).unwrap();
    let file = cwd.join("doc.md");
    fs::write(&file, "# hi\n").unwrap();

    with_env(&root, &herdr, || {
        herdr_preview::hint::open_preview_file(&file, &cwd, None).expect("open_preview_file");
    });

    let invocations = read_invocations(&stub_log_path(&root));
    let args = &invocations[0];
    assert!(args.contains(&"less".to_string()));
    assert!(args.contains(&"overlay".to_string()));
    assert!(!args.contains(&"browse".to_string()));
}
```

Keep existing file and URL routing tests unchanged. Update `tests/hint.rs` only if it mentions `DirSkip` / `DIR_SKIP_NOTICE` (it currently does not). In `src/hint.rs` unit test, change `assert_eq!(route_entry(&dir, &herdr), OpenRoute::DirSkip)` to `OpenRoute::Browse`.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --test routing --test hint`
Expected: FAIL (`DirSkip` still used / `open_browse` missing / `OpenRoute::Browse` missing).

- [ ] **Step 3: Implement routing + `open_browse` + `open_preview_file`**

`open_browse` can copy `open_less`'s `Command` construction. Do **not** add the `browse` pane or `main.rs` subcommand in this task — tests only assert the `herdr` argv the stub logs.

- [ ] **Step 4: Run tests**

Run: `cargo test`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/hint.rs src/open.rs tests/routing.rs
git commit -m "$(cat <<'EOF'
feat: route directory hint picks to browse overlay

EOF
)"
```

---

### Task 3: Browse overlay TUI, pane, and `browse` subcommand (TDD)

**Files:**
- Modify: `src/browse.rs` (render, input map, `run_browse_overlay`)
- Modify: `tests/browse.rs` (render + key/mouse mapping tests)
- Modify: `src/main.rs` (`browse` subcommand + usage)
- Modify: `herdr-plugin.toml` (`[[panes]]` id `browse`)
- Modify: `tests/hint.rs` (`manifest_declares_hint_action_and_panes` must also assert `id = "browse"`)

**Interfaces:**
- Consumes: `BrowseState`, env `HERDR_PREVIEW_BROWSE_START`, `HERDR_PREVIEW_BROWSE_CWD`, optional `HERDR_PREVIEW_BROWSE_ORIGIN`
- Produces:

```rust
pub fn render_browse(state: &BrowseState, rows: u16, cols: u16) -> String;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BrowseKey {
    Char(char),
    Enter,
    Esc,
    Up,
    Down,
    Left,
    Right,
    MouseClick { row: u16, col: u16 }, // 0-based screen row/col
    MouseWheelUp,
    MouseWheelDown,
}

pub fn map_browse_key(key: BrowseKey) -> Option<BrowseCommand>;
// ↑/k → MoveUp; ↓/j → MoveDown; ←/h → GoParent; →/l → EnterDir;
// Enter → Activate; q/Q/Esc → Dismiss;
// MouseWheelUp/Down → ScrollUp/Down;
// MouseClick { row, .. } → if row == 0 (header) or last row (footer): None;
//   else SelectIndex(state is applied by caller: index = scroll + row - 1) — map returns
//   a dedicated command. Use BrowseCommand::SelectIndex(usize) only from run loop after
//   computing index. map_browse_key(MouseClick) should return None; provide:
pub fn click_command(state: &BrowseState, screen_row: u16, total_rows: u16) -> Option<BrowseCommand>;
// header row 0 and footer row total_rows-1 ignored; list_row = screen_row - 1;
// index = state.scroll + list_row; if index < rows.len() { Some(SelectIndex) } else None
// The run loop on SelectIndex: apply SelectIndex, then apply Activate.

pub fn parse_browse_input(prefix: &[u8]) -> Option<BrowseKey>;
// b'j' → Char('j'); 0x0d/0x0a → Enter; 0x1b alone → Esc;
// CSI A/B/C/D → Up/Down/Right/Left;
// SGR mouse `\x1b[<0;COL;ROWM` (1-based) → MouseClick { row: ROW-1, col: COL-1 } (button 0 press);
// `\x1b[<64;...M` → MouseWheelUp; `\x1b[<65;...M` → MouseWheelDown;
// other / release `m` → None

pub fn run_browse_overlay() -> Result<(), io::Error>;
```

Render layout (must match click mapping):

- Row 0: current `state.cwd` display, truncated to `cols` (no ANSI in the path itself; dim OK).
- Rows 1..rows-2: visible slice `state.rows[scroll..]` with `>` (or reverse video) on `selected`. Directory names already include trailing `/` via `display_name`. Placeholder `(empty)`.
- Last row: if `state.notice` is Some, show that text; else `browse · j/k move · enter open · h parent · l enter · q cancel`.
- No extra trailing newline on the last row (same paint rule as hint overlay).

TUI loop in `run_browse_overlay` (this part is not unit-tested against a real TTY):

1. Read `HERDR_PREVIEW_BROWSE_START` (required), `HERDR_PREVIEW_BROWSE_CWD` (required), `HERDR_PREVIEW_BROWSE_ORIGIN` (optional). Missing start/cwd → error like hint overlay env errors.
2. `BrowseState::open(start)`.
3. Copy hint overlay TTY helpers **into browse.rs** (RawMode, TerminalGuard, `open_tty`, `tty_size`, `read` loop). Do **not** refactor `hint.rs` in this task. Enable SGR mouse: write `\x1b[?1000h\x1b[?1006h` on enter; disable `\x1b[?1000l\x1b[?1006l` in TerminalGuard Drop (in addition to cursor show / wrap on).
4. Loop: render, read bytes, parse, map, `apply`. `visible_rows = rows.saturating_sub(2).max(1)`.
5. `BrowseOutcome::OpenFile` → `open_preview_file(&path, &origin_cwd, origin.as_deref())` then return Ok (process exit dismisses overlay).
6. `Dismiss` → return Ok without opening.

`herdr-plugin.toml` add after the `less` pane:

```toml
[[panes]]
id = "browse"
title = "Browse"
placement = "overlay"
command = ["./target/release/herdr-preview", "browse"]
```

`main.rs`: dispatch `browse` like `hint-overlay`. Usage line: `herdr-preview browse`.

- [ ] **Step 1: Write failing tests** in `tests/browse.rs`:

```rust
use herdr_preview::browse::{
    click_command, map_browse_key, parse_browse_input, render_browse, BrowseCommand, BrowseKey,
    BrowseState,
};

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
fn map_keys_match_spec() {
    assert_eq!(map_browse_key(BrowseKey::Char('k')), Some(BrowseCommand::MoveUp));
    assert_eq!(map_browse_key(BrowseKey::Up), Some(BrowseCommand::MoveUp));
    assert_eq!(map_browse_key(BrowseKey::Char('j')), Some(BrowseCommand::MoveDown));
    assert_eq!(map_browse_key(BrowseKey::Down), Some(BrowseCommand::MoveDown));
    assert_eq!(map_browse_key(BrowseKey::Char('h')), Some(BrowseCommand::GoParent));
    assert_eq!(map_browse_key(BrowseKey::Left), Some(BrowseCommand::GoParent));
    assert_eq!(map_browse_key(BrowseKey::Char('l')), Some(BrowseCommand::EnterDir));
    assert_eq!(map_browse_key(BrowseKey::Right), Some(BrowseCommand::EnterDir));
    assert_eq!(map_browse_key(BrowseKey::Enter), Some(BrowseCommand::Activate));
    assert_eq!(map_browse_key(BrowseKey::Char('q')), Some(BrowseCommand::Dismiss));
    assert_eq!(map_browse_key(BrowseKey::Esc), Some(BrowseCommand::Dismiss));
    assert_eq!(map_browse_key(BrowseKey::MouseWheelUp), Some(BrowseCommand::ScrollUp));
    assert_eq!(map_browse_key(BrowseKey::MouseWheelDown), Some(BrowseCommand::ScrollDown));
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
    assert_eq!(parse_browse_input(b"\x1b[<64;1;1M"), Some(BrowseKey::MouseWheelUp));
    assert_eq!(parse_browse_input(b"\x1b[<65;1;1M"), Some(BrowseKey::MouseWheelDown));
}

#[test]
fn click_on_list_row_selects_index() {
    let root = fixture("click");
    fs::write(root.join("a.rs"), "x\n").unwrap();
    let state = BrowseState::open(&root);
    // row 0 header; row 1 is `..` (index 0); next child starts at row 2
    assert_eq!(click_command(&state, 0, 8), None);
    assert_eq!(click_command(&state, 7, 8), None);
    assert_eq!(click_command(&state, 1, 8), Some(BrowseCommand::SelectIndex(0)));
}

#[test]
fn manifest_declares_browse_pane() {
    let manifest = std::fs::read_to_string("herdr-plugin.toml").unwrap();
    assert!(manifest.contains("id = \"browse\""));
    assert!(!manifest.contains("[[link_handlers]]"));
}
```

Also add `assert!(manifest.contains("id = \"browse\""));` to `manifest_declares_hint_action_and_panes` in `tests/hint.rs`.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --test browse --test hint`
Expected: FAIL on missing render/map/parse/pane.

- [ ] **Step 3: Implement render, input, overlay loop, manifest, main**

Reuse hint overlay paint constants where they help (dim, reset, home, clear). `read` loop: first byte; if ESC, poll 50ms like hint; if more bytes, parse CSI/SGR; bare ESC → Esc.

On `SelectIndex`, the loop should `apply(SelectIndex)` then `apply(Activate)` so a click both highlights and opens/enters.

- [ ] **Step 4: Run tests**

Run: `cargo test`
Expected: PASS. Confirm `herdr-plugin.toml` has zero `[[link_handlers]]`.

- [ ] **Step 5: Commit**

```bash
git add src/browse.rs src/main.rs herdr-plugin.toml tests/browse.rs tests/hint.rs
git commit -m "$(cat <<'EOF'
feat: add browse overlay TUI for directory hint picks

EOF
)"
```

---

### Task 4: README and MVP spec amendments

**Files:**
- Modify: `README.md`
- Modify: `docs/superpowers/specs/2026-08-09-herdr-preview-design.md`

**Interfaces:** none (docs only). Do not change browse/open behavior.

- [ ] **Step 1: Update README**

In **How to use** step 3, add a directory bullet after the file bullets:

```markdown
   - **Directory path** → browse overlay listing that folder. Arrow keys / `j` `k` move; `Enter` or click opens a file (file-viewer or `less`) or enters a subfolder; `q` / Esc dismisses browse only.
```

In **Preview: file-viewer vs `less` fallback**, replace the bullet `- Directories are skipped with a notice (`less` is file-oriented).` with:

```markdown
- Directories open the same browse overlay as when file-viewer is installed; choosing a file then uses `less`.
```

Do not move Credits. Do not add a quicklook comparison at the top.

- [ ] **Step 2: Amend MVP design doc**

In `docs/superpowers/specs/2026-08-09-herdr-preview-design.md`:

- Non-goals: change “Own full file-browser / tree UI.” to “Own full file-browser / tree UI (a thin directory browse overlay is specified in `docs/superpowers/specs/2026-08-12-directory-browse-design.md`).”
- UX item 3 directories bullet: replace “Directories: with FV, summon and select/open that directory if OPEN accepts it; with `less` fallback, skip + notice (less is file-oriented).” with “Directories: open the herdr-preview browse overlay (see 2026-08-12 directory-browse spec); file chosen there uses FV or `less`.”
- Detection table Directory row: replace “FV summon/OPEN if accepted; `less` path → skip + notice” with “browse overlay (always); file chosen → `open_preview`.”

Keep Brainstorm Q&A verbatim. Do not rewrite the whole MVP spec.

- [ ] **Step 3: Commit**

```bash
git add README.md docs/superpowers/specs/2026-08-09-herdr-preview-design.md
git commit -m "$(cat <<'EOF'
docs: document directory browse overlay and amend MVP spec

EOF
)"
```

---

## Execution handoff prompt (paste into a new agent)

```
Implement directory browse overlay from the approved plan in this repo.

1. Read AGENTS.md, docs/superpowers/specs/2026-08-12-directory-browse-design.md, and docs/superpowers/plans/2026-08-15-directory-browse.md.
2. Use superpowers:subagent-driven-development (or executing-plans) on that plan.
3. Respect Global Constraints: no [[link_handlers]], directory pick → browse overlay (not FV dir OPEN, not DirSkip), file opens still FV preferred / less fallback, visible-scan only, Linux+macOS.
4. Do not expand scope into search, icons, hide-dot, or large-Markdown performance.
```
