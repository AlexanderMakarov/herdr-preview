# Agent handoff — herdr-preview

## Before coding

1. Read `docs/superpowers/specs/2026-08-09-herdr-preview-design.md` end-to-end (including **Brainstorm Q&A**).
2. Read `docs/superpowers/plans/2026-08-09-herdr-preview.md`.
3. Implement with **superpowers:subagent-driven-development** (preferred) or **superpowers:executing-plans**, task-by-task. Do not invent scope outside the plan without updating the spec first.

## Hard product constraints (do not “helpfully” violate)

- Never add Herdr `[[link_handlers]]` / git-host-token handlers that hijack Ctrl+click (learned from quicklook + `gh pr view` / Projects-classic GraphQL stderr in the terminal).
- Prefer **herdr-file-viewer** OPEN; fallback **`less`** overlay if FV missing.
- Hint scan = **visible** pane text only; Linux+macOS MVP.
- README top stays problem-first; quicklook credit stays at bottom.

## Useful local references (on the author’s machine)

- Installed file-viewer plugin tree: `~/.config/herdr/plugins/github/herdr-file-viewer-*` (esp. `docs/usage.md` OPEN / `HERDR_FILE_VIEWER_OPEN`, `docs/summoning.md`, `docs/keys.md`).
- Prior quicklook fork lessons: `~/code/herdr-quicklook` (spaced paths, lesskey, link_handlers — do not reintroduce PR hijack).
- Brainstorm chat transcript (Cursor): agent id folder `91f5a3fc-93e2-44e4-8f18-f5f25fbc7429`.

## Out of scope until profiled

Large Markdown slow opens in file-viewer (glow vs `preview_max_*`). Investigate separately; do not bake product policy into MVP.
