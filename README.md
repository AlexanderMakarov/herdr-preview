# herdr-preview

Opening a path (often Markdown) that an agent printed in a Herdr pane — plans, logs, error output — takes too many steps: copy the path, leave the flow, summon a viewer, find the file, jump to the line. **herdr-preview** is a Herdr plugin that cuts that to a hint keypress and a letter: pick the path and land in a useful preview.

**Status:** design + implementation plan only. Binary/plugin not shipped yet.

## Intended UX (MVP)

1. Bind `prefix+v` (or similar) to the hint action.
2. Letter targets appear over path-like spans and URLs in the **visible** pane text.
3. Path → open **herdr-file-viewer** with `HERDR_FILE_VIEWER_OPEN` when that plugin is installed (focused split; `q` closes it).
4. If file-viewer is **not** installed → open **`less`** in an overlay above the current pane (reduced functionality: no glow/bat/git/tree).
5. `http(s)` → system browser only. This plugin does **not** register `[[link_handlers]]` and must not steal Ctrl+click for GitHub PRs (`gh` / GraphQL noise).

## Docs for implementers / agents

| Doc | Purpose |
| --- | --- |
| [AGENTS.md](AGENTS.md) | How to start an agent session on this repo |
| [docs/superpowers/specs/2026-08-09-herdr-preview-design.md](docs/superpowers/specs/2026-08-09-herdr-preview-design.md) | Approved design + full brainstorm Q&A |
| [docs/superpowers/plans/2026-08-09-herdr-preview.md](docs/superpowers/plans/2026-08-09-herdr-preview.md) | Implementation plan (task-by-task) |

## Platforms

Linux and macOS for MVP. Windows later.

## License

MIT.

---

## Credits

Hint-overlay UX inspiration: [dwarvesf/herdr-quicklook](https://github.com/dwarvesf/herdr-quicklook). This is a separate MIT project, not a fork.
