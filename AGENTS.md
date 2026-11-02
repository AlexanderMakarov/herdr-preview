# Agent handoff — herdr-preview

## Before coding

1. Read `docs/superpowers/specs/2026-08-09-herdr-preview-design.md` end-to-end (including **Brainstorm Q&A**).
2. Read `docs/superpowers/plans/2026-08-09-herdr-preview.md`.
3. Implement with **superpowers:subagent-driven-development** (preferred) or **superpowers:executing-plans**, task-by-task. Do not invent scope outside the plan without updating the spec first.

## Hard product constraints (do not “helpfully” violate)

- Never add Herdr `[[link_handlers]]` / git-host-token handlers that hijack Ctrl+click (learned from quicklook + `gh pr view` / Projects-classic GraphQL stderr in the terminal).
- Prefer **herdr-file-viewer** OPEN; fallback **`less`** overlay if FV missing.
- Hint scan = bounded **`recent`** pane tail (`herdr pane read --source recent --lines N`, default 200, max 1000 via `HERDR_PREVIEW_HINT_LINES`), not full scrollback; Linux+macOS MVP.
- README top stays problem-first; quicklook credit stays at bottom.

## No personal data in the repo

Never copy real names, usernames, home paths (`/home/<user>`), mount points or folder names from the author's machine or pasted pane text into code, comments, tests, docs, commit messages or PR text. Reproduce bugs with neutral stand-ins that keep the shape that matters (spaces, dashes, Unicode), e.g. `Project files/Jane Doe - Draft notes.md`, `/home/user/...`.

## Useful local references (on the author’s machine)

- Installed file-viewer plugin tree: `~/.config/herdr/plugins/github/herdr-file-viewer-*` (esp. `docs/usage.md` OPEN / `HERDR_FILE_VIEWER_OPEN`, `docs/summoning.md`, `docs/keys.md`).
- Prior quicklook fork lessons: `~/code/herdr-quicklook` (spaced paths, lesskey, link_handlers — do not reintroduce PR hijack).
- Brainstorm chat transcript (Cursor): agent id folder `91f5a3fc-93e2-44e4-8f18-f5f25fbc7429`.

## Out of scope until profiled

Large Markdown slow opens in file-viewer (glow vs `preview_max_*`). Investigate separately; do not bake product policy into MVP.

## Local Herdr — rebuild after each iteration

On the author machine the plugin is **`herdr plugin link`’d** to this repo. **`link` does not run manifest `[[build]]`**, so agents must finish hint/overlay/manifest/script changes with a fresh release binary and refreshed link:

```bash
./scripts/reinstall-local.sh
```

Equivalent: `cargo build --release` from repo root, then `herdr plugin link` with the repo’s absolute path. Run `herdr server reload-config` only when `~/.config/herdr/config.toml` changed, not for normal code edits. Confirm with `herdr plugin list` (source `local:` → this tree).

The author reloads the plugin by running `./scripts/reinstall-local.sh` locally (agents may run it too, but only that refreshes the binary on the author’s running Herdr session — already-open overlay panes still need a new preview invocation).
