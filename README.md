# herdr-preview

Opening a path that appears in a Herdr pane — plans, logs, error output — takes too many steps: copy the path, leave the flow, summon a viewer, find the file, jump to the line. **herdr-preview** is a Herdr plugin that cuts that to two keystrokes: `prefix+/`, then one letter over the path you want.

## How to use

1. Install the plugin and bind `prefix+/` (see [Install](#install)).
2. All file paths in the visible pane text are highlighted with letter hints so you can preview them. If nothing openable is on screen, you get a short notice.
3. Press a letter to open the pick:
   - **File path** → [herdr-file-viewer](https://github.com/smarzban/herdr-file-viewer) when that plugin is installed (right split; `q` closes it).
   - **File path, no file-viewer** → **`less`** in an overlay above the current pane ([reduced mode](#preview-file-viewer-vs-less-fallback)).
   - **Directory path** → browse overlay listing that folder. Arrow keys / `j` `k` move; `Enter` or click opens a file (file-viewer or `less`) or enters a subfolder; `q` / Esc dismisses browse only.

## Install

**Released install** (once version tags are published):

```sh
herdr plugin install github:AlexanderMakarov/herdr-preview@v0.1.0
```

**Local development** (`herdr plugin link` does not run the manifest `[[build]]` step — build first):

```sh
cargo build --release
herdr plugin link /path/to/herdr-preview
```

Confirm registration with `herdr plugin list`.

### Keybinding

Add to `~/.config/herdr/config.toml`:

```toml
[[keys.command]]
key = "prefix+/"
type = "plugin_action"
command = "herdr-preview.hint"
description = "hint-pick file paths on screen"
```

Reload with `herdr server reload-config`.

## Preview: file-viewer vs `less` fallback

herdr-preview does not render file content itself. It routes picks to a peer opener:

| Peer | When | What you get |
| --- | --- | --- |
| **herdr-file-viewer** | Plugin installed and listed by `herdr plugin list` | Opens via `HERDR_FILE_VIEWER_OPEN` — rendered Markdown, syntax highlighting, git-aware tree, diff view, and the viewer's normal keys. |
| **`less` overlay** | File-viewer not installed | A `less` pane above your current app — **reduced functionality** by design. |

**Reduced `less` mode** (when file-viewer is absent):

- Plain paging only — no glow/bat rendering, no git tree, no diff view.
- Line jumps when the token includes `:N` or `:A-B`.
- Directories open the same browse overlay as when file-viewer is installed; choosing a file then uses `less`.
- `q` closes the overlay and returns to the pane underneath.

Install [herdr-file-viewer](https://github.com/smarzban/herdr-file-viewer) for the full preview experience; the `less` fallback exists so hint-pick still works without it.

## Platforms

Linux and macOS for MVP. Windows later.

## License

MIT.

---

## Credits

Hint-overlay UX inspiration: [dwarvesf/herdr-quicklook](https://github.com/dwarvesf/herdr-quicklook). This is a separate MIT project, not a fork.
