# herdr-preview — directory browse overlay

Status: approved for implementation (2026-08-12)  
Related: [issue #2](https://github.com/AlexanderMakarov/herdr-preview/issues/2), amends `2026-08-09-herdr-preview-design.md`

## Problem

Hint-picking a **directory** should get the user to a file with few actions. herdr-file-viewer’s `HERDR_FILE_VIEWER_OPEN` / `--open` only reveals **regular files** (`TreeModel::reveal` rejects non-files). The old “FV dir OPEN / DirSkip + notice” path therefore fails the goal when FV is present and dead-ends when it is not.

## Goals

- On directory hint-pick, open a **herdr-preview-owned** browse overlay (not FV’s tree).
- List immediate children; drill into subfolders; `..` and free parent navigation (hint-picked path is start location, not a jail).
- Selecting a **file** closes browse and opens via existing preview routing: FV if installed, else `less`.
- Keyboard and mouse both work for move/activate.
- Same platforms as MVP: Linux + macOS.
- No `[[link_handlers]]`.

## Non-goals

- Search/filter, git status coloring, icons, hide-dot toggle.
- Relying on FV directory OPEN or any FV UI beyond “open this file focused.”
- Recursive flat file lists.
- Image/media preview.
- Windows.

## Architecture

```
hint pick → Target::Dir
    → plugin pane open browse overlay
        (env: start path, origin pane id)
    → list / navigate
    → file chosen
        → dismiss browse
        → open_preview(file)  // FV preferred, else less
    → q / Esc
        → dismiss browse only
```

- New `[[panes]]` entry `browse` in `herdr-plugin.toml` → binary subcommand (e.g. `herdr-preview browse`).
- New module `src/browse.rs`: directory listing, sort, navigation state, TUI render/input; list/nav logic unit-tested without a real TTY.
- Reuse `open_file_viewer` / `open_less` / `detect_file_viewer` for the hand-off after a file is chosen.
- `route_entry` / `open_entry`: `Target::Dir` → always browse; remove `DirSkip` as the no-FV directory path.

## UX

1. User hint-picks an existing directory.
2. Overlay opens listing that directory’s immediate children.
3. Status/header shows current path (truncate if needed).
4. Rows: `..` when a parent exists; directories (trailing `/`); files. Sort: directories first, then files, case-insensitive by name. Dot entries included (no hide toggle).
5. Inputs:

| Input | Action |
| --- | --- |
| `↑`/`↓` or `k`/`j` | Move selection |
| `Enter` or mouse click on row | Directory → enter; file → close browse + open FV/`less` |
| `←` or `h` | Go up (`..`) when parent exists |
| `→` or `l` | Enter selected directory |
| `q` / `Esc` | Dismiss overlay only |
| Mouse wheel | Scroll when list taller than pane |

6. Empty directory: show `..` (if any) plus an “(empty)” placeholder; stay in browse.
7. Unreadable directory: short notice; keep previous listing if possible.
8. Regular file and URL hint picks unchanged.

## Detection / routing (amended)

| Kind | Action |
| --- | --- |
| Existing regular file | `open_preview` (FV or `less`) |
| Directory | **browse overlay** (always) |
| `http(s)` | System browser |
| Missing path | Skip + notice |

## Error handling

- Browse spawn failure: Herdr notice / stderr; leave origin pane focused.
- File open after pick fails: surface existing opener errors; browse already dismissed (same as today’s file-open failure after hint).
- `less` missing: existing toast; do not invent alternate directory UI.

## Testing

- Unit: sort order; `..` / parent walk; drill-in; empty and unreadable dirs.
- Integration (mocked `herdr`): directory pick opens browse pane with start-path env; choosing a file routes to FV vs `less` stubs; file and URL routes unchanged; no `link_handlers` in manifest.
- Manual: hint a folder → browse → Enter file → FV or `less`; `q` returns to origin; `..` leaves the start folder.

## Packaging / README

- Document directory pick → browse → file → FV or `less`.
- Update MVP design doc note: directories no longer “FV OPEN if accepted / DirSkip.”
- Issue #2 satisfied by this browse path (not by FV directory OPEN).

## Success criteria

- Hint-pick directory → browse overlay starts in that folder.
- Fewest actions to a file: arrows/`Enter` or mouse click; then FV or `less` opens the file.
- Without FV, directory picks still work (browse + `less`).
- No Ctrl+click hijack via `[[link_handlers]]`.
- File and URL behavior unchanged.

## Amendments to 2026-08-09 MVP design

- Non-goal “Own full file-browser / tree UI” → allow this **thin** directory browse overlay only.
- Directory row in detection/UX tables → browse overlay as above (not FV dir OPEN / DirSkip).
