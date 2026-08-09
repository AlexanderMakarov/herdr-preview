# herdr-preview

Opening a path (often Markdown) that an agent printed in a Herdr pane — plans, logs, error output — takes too many steps: copy the path, leave the flow, summon a viewer, find the file, jump to the line. **herdr-preview** is a Herdr plugin that cuts that to two keystrokes: `prefix+/`, then one letter over the path you want.

## How to use

1. Install the plugin and bind `prefix+/` (see [Install](#install)). Note: Herdr’s default `prefix+v` is vertical split — do not reuse it.
2. Letter hints appear over path-like spans and URLs in the **visible** pane text only.
3. Press a letter to open the pick:
   - **File path** → [herdr-file-viewer](https://github.com/smarzban/herdr-file-viewer) when that plugin is installed (focused split; `q` closes it).
   - **File path, no file-viewer** → **`less`** in an overlay above the current pane ([reduced mode](#preview-file-viewer-vs-less-fallback)).
   - **`http(s)` URL** → your system browser only.

This plugin does **not** register `[[link_handlers]]` and must not steal Ctrl+click for GitHub PRs (`gh` / GraphQL noise in the terminal).

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
description = "hint-pick paths and URLs on screen"
```

Reload with `herdr server reload-config`.

Native `plugin_action` bindings preserve Herdr's plugin context and avoid an extra shell hop (same pattern as [herdr-quicklook](https://github.com/dwarvesf/herdr-quicklook) and [herdr-file-viewer](https://github.com/smarzban/herdr-file-viewer)).

## Preview: file-viewer vs `less` fallback

herdr-preview does not render file content itself. It routes picks to a peer opener:

| Peer | When | What you get |
| --- | --- | --- |
| **herdr-file-viewer** | Plugin installed and listed by `herdr plugin list` | Opens via `HERDR_FILE_VIEWER_OPEN` — rendered Markdown, syntax highlighting, git-aware tree, diff view, and the viewer's normal keys. |
| **`less` overlay** | File-viewer not installed | A `less` pane above your current app — **reduced functionality** by design. |

**Reduced `less` mode** (when file-viewer is absent):

- Plain paging only — no glow/bat rendering, no git tree, no diff view.
- Line jumps when the token includes `:N` or `:A-B`.
- Directories are skipped with a notice (`less` is file-oriented).
- `q` closes the overlay and returns to the pane underneath.

Install [herdr-file-viewer](https://github.com/smarzban/herdr-file-viewer) for the full preview experience; the `less` fallback exists so hint-pick still works without it.

## Platforms

Linux and macOS for MVP. Windows later.

## Docs for implementers / agents

| Doc | Purpose |
| --- | --- |
| [AGENTS.md](AGENTS.md) | How to start an agent session on this repo |
| [docs/superpowers/specs/2026-08-09-herdr-preview-design.md](docs/superpowers/specs/2026-08-09-herdr-preview-design.md) | Approved design + full brainstorm Q&A |
| [docs/superpowers/plans/2026-08-09-herdr-preview.md](docs/superpowers/plans/2026-08-09-herdr-preview.md) | Implementation plan (task-by-task) |

## License

MIT.

---

## Credits

Hint-overlay UX inspiration: [dwarvesf/herdr-quicklook](https://github.com/dwarvesf/herdr-quicklook). This is a separate MIT project, not a fork.
