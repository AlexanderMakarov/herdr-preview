# herdr-preview — design

Status: approved for handoff (2026-08-09)  
Repo: https://github.com/AlexanderMakarov/herdr-preview

## Problem

Opening a path (often Markdown) that an agent printed in a Herdr pane — plans, logs, compile errors — takes too many steps: copy the path, leave the flow, summon a viewer, navigate to the file, jump to the line. **herdr-preview** cuts that to a hint keypress and a letter: pick the path under the cursor region and land in a useful preview.

## Goals (MVP)

- Hint overlay on the focused pane’s **visible** text (`prefix+v` or equivalent).
- Path pick → open content with line/range when available.
- Prefer **herdr-file-viewer** when installed (`HERDR_FILE_VIEWER_OPEN` / equivalent summon).
- If file-viewer is missing → **`less`** in an overlay/popup above the current pane (reduced functionality; documented in README).
- `http(s)` picks → system browser only.
- Never register Herdr `[[link_handlers]]` / git-host tokens that steal Ctrl+click (avoids opening PRs via `gh` and surfacing GraphQL Projects-classic noise in the terminal).
- Platforms: Linux and macOS. Windows later.
- New MIT Rust Herdr plugin (own repo, not a GitHub fork).

## Non-goals (MVP)

- Own full file-browser / tree UI (a thin directory browse overlay is specified in `docs/superpowers/specs/2026-08-12-directory-browse-design.md`).
- Image or media rendering.
- Ctrl-hold underline or OSC-8 linkify inside scrollback.
- Full-scrollback scan (visible only).
- Tuning or documenting file-viewer `preview_max_*` as part of this product.
- Large-Markdown open performance — **open investigation** (may be glow cost vs caps); not specified here until profiled.
- Escalation hotkeys to `$EDITOR` / “open in file-viewer” from the less fallback (can revisit later).

## Architecture

```
prefix+v
    → herdr-preview hint action
        → read focused pane visible text + cwd (Herdr IPC)
        → Rust tokenize / classify / resolve
        → hint overlay (letter targets)
            → path → open_preview(path[:line…])
            → http(s) → open browser
            → else → toast / no-op

open_preview:
    if herdr-file-viewer installed and summonable:
        plugin pane / action with HERDR_FILE_VIEWER_OPEN=…
        (split or host-default placement; focus Files; q closes pane)
    else:
        less overlay/popup above current pane (+N when line known)
```

- Single Rust plugin binary + `herdr-plugin.toml`.
- Hint UI is the only surface herdr-preview owns; content rendering is delegated (FV preferred, `less` fallback).
- Pure Rust classify/resolve in-process (no bash scan pipeline required for MVP correctness).

## UX

1. User focuses a shell/agent pane and presses the bound key.
2. Overlay shows letter (or short) targets on path-like and URL spans in the **visible** snapshot.
3. Choosing a filesystem target:
   - Resolves relative segments against `focused_pane_cwd`.
   - Handles spaced paths and `%20` (known quicklook footgun).
   - Passes `path`, `path:N`, or `path:A-B` through to file-viewer OPEN when using FV (same shapes FV documents; no separate column hop).
   - Directories: open the herdr-preview browse overlay (see 2026-08-12 directory-browse spec); file chosen there uses FV or `less`.
4. Choosing `http(s)`: system browser only.
5. File-viewer path: focus lands in Files; `q` / `Esc` quits the viewer process and tears down the split/tab (same as normal FV summon). Note: `q` exits the whole Files app, not a stacked “preview layer.”
6. `less` fallback: overlay above the existing app; fewer features (no glow/bat/git/tree); document clearly in README.

Placement for hints and for `less`: overlay/popup above the current application is acceptable; FV uses its normal split/tab summon. Either placement is fine as long as focus and `q` behavior stay predictable.

## Detection rules (MVP)

| Kind | Action |
| --- | --- |
| Existing regular file | `open_preview` (FV or `less`; binary handling left to the opener) |
| Directory | browse overlay (always); file chosen → `open_preview` |
| `http://` / `https://` | System browser |
| Missing path after resolve | Skip + notice |

Tokenization must not truncate on the first whitespace inside a path; decode `%20` for filesystem candidates.

File-viewer OPEN paths must live under the viewer’s tree root (usually the repo/worktree of the summon cwd). Out-of-root absolute paths may fail; surface FV’s failure rather than silently rewriting.

## Error handling

- Missing FV: silent fallback to `less` (no hard error); mention reduced mode once in overlay banner or README, not every open.
- `less` missing from PATH: clear toast; do not fall through to `gh` or URL handlers.
- Open/summon failure: show stderr snippet or short Herdr notice; leave the origin pane focused if the preview pane never started.
- Hint cancel (`q` / Esc on overlay): dismiss overlay only.

## Testing (MVP plan outline)

- Unit: tokenizer (spaces, `%20`, line/range suffixes), classifier (file/dir/url/missing), OPEN string builder.
- Integration (mocked `herdr` CLI): hint action chooses FV summon vs `less` spawn based on peer detection; never writes `link_handlers` into manifest fixtures.
- Manual: agent Markdown path → FV; PR URL Ctrl+click still browser with plugin installed; `less` fallback with FV uninstalled.

## Configuration

- Herdr key → `herdr-preview` hint action (suggested `prefix+v`).
- Plugin manifest: hint + overlay pane entries as needed; **no** `[[link_handlers]]`.
- Peer dependency: herdr-file-viewer strongly preferred; absence triggers `less` fallback (not a hard install failure).

## Packaging / README

- Repo: `herdr-preview`, MIT license, GitHub install once published.
- README **top**: the problem (too many actions to open agent-printed paths, especially Markdown) and how to use the plugin.
- README: document FV vs `less` fallback and reduced `less` capabilities.
- README **bottom** (short, optional): credit dwarvesf/herdr-quicklook as inspiration — do not lead with a feature comparison.
- Do not front-load “differences from quicklook” or FV `preview_max_*` tuning notes.

## Open investigations (out of MVP scope)

- Why large Markdown opens can take multiple seconds in file-viewer with default caps (glow vs truncation vs config). Decide any product response after profiling.

## Success criteria

- From an agent plan in a pane, `prefix+v` + one key opens the referenced Markdown (or other text path) in FV when installed, or `less` overlay otherwise.
- Ctrl+click on a GitHub PR URL still goes to the browser (plugin does not register competing link handlers).
- Spaced path tokens and `%20` resolve to the real file when it exists on disk.

---

## Brainstorm Q&A (verbatim answers)

Context notes from the session are in italics. Multiple-choice option labels are shortened to the chosen option id plus the label text the user selected. Free-text answers are copied as given.

### Background incident (motivating constraint)

**User (as-is):** Now Ctrl+click works in a wrong way - if do it on URL then preview tries to open it right in the terminal with `  GraphQL: Projects (classic) is being deprecated in favor of the new Projects experience, see: https://github.blog/changelog/2024-05-23-sunset-notice-projects-classic/. (repository.pullRequest.projectCards)` message

*Interpretation carried into the design: do not ship quicklook-style `git-host-token` / `[[link_handlers]]` that run `gh pr view` on Ctrl+click. At handoff time on the author’s machine, `herdr-quicklook` was already uninstalled; only `herdr-file-viewer` and `reviewr` remained.*

### Pre-summary decisions (from prior turn / summary)

| Topic | Decision |
| --- | --- |
| Name | `herdr-preview` |
| MVP entry | `prefix+v` hint overlay (not Ctrl-hold, not OSC-8-first) |
| Inspiration | quicklook-like UX; new MIT repo, **not** a fork |
| Platforms ambition | more portable than bash-heavy quicklook long-term |

### Q: How should herdr-preview treat URLs in the hint overlay?

**Selected:** `browser-always` — All http(s) → system browser (never hijack PRs to gh) — Recommended

### Q: From the preview pane, should MVP expose “open elsewhere” actions?

**Selected:** `preview-only` — Preview pane only for MVP (escalate later)

*Later superseded: MVP does not own a renderer; content opens in FV or `less`. No escalate-to-editor in MVP remains true.*

### Q: Path detection scope for MVP?

**Selected:** `paths-linux-mac` — MVP: local/relative paths + line:col; Linux+macOS; Windows later — Recommended

*Line:col narrowed in self-review to FV-documented `path:N` and `path:A-B` only (no column hop).*

### Q: Architecture approaches?

**Selected:** `a-rust-core` — Pure Rust plugin — scan visible pane in-process, hint overlay via Herdr, preview pane renders text/images itself; never registers `[[link_handlers]]` / git-host-token

*Later superseded for rendering: companion to file-viewer (+ `less` fallback), not an own preview renderer. Rust-in-process scan + no link_handlers retained.*

### Q: Design §1 Architecture — Approve?

**User (as-is):** Let's make click (or button press before path) to open above existing application in the same pane. The same as herdr-quicklook do mostly. Also it make sense to preview only text files - we can't render images/media in terminal. From features: Markdown syntax highlighting at least (maybe find good library which do it already for Multiple languages, or add More languages later), Warning if file is bigger than 20kB, Ability to jump on line, lines wrapping. Hotkeys inside preview very close to herdr-file-viewer. While stop - maybe really reuse herdr-file-viewer for file rendering? But I want: in the side (right) pane and only file preview - without files tree, immediate focus switch on preview, q hotkey to close pane. Is it possible?

*Answer given in session: FV can OPEN + focus + `q` closes split; zoom (`z` / auto-zoom) hides tree but there is no stock tree-less preview-only mode.*

### Q: Placement — overlay vs right split?

**Selected:** `either-ok` — Either is fine if q closes and focus lands in the preview immediately

### Q: Given file-viewer can’t be tree-less, how should herdr-preview render files?

**User (as-is):** if use `open via file-viewer OPEN` way - on "q" press I will get file-viewer closed together with side pane, right?

*Confirmed: yes — `q`/`Esc` quits the viewer process; Herdr tears down that split/tab; caveat that `q` exits the whole Files app, not one stacked preview layer.*

### Q: With that, which path?

**Selected:** `companion-fv` — Got it — use file-viewer OPEN companion; q closing the split is fine

### Q: Design §2 UX — Approve?

**User (as-is):** approve except large files limitation - let's reuse existing FV's truncation but reduce it - right now I am getting big Markdown files opening multiple seconds so `preview_max_*` is not working properly

### Q: Large-file / Markdown performance?

**Selected:** `investigate-first` — Don't design around this yet — first profile why a big .md is slow (cap vs glow), then decide

### Q: Next: profile or continue design?

**Selected:** `continue-design` — Park large-file performance as open investigation; continue design §3 (detection / non-goals / config)

### Q: Design §3 Detection / non-goals — Approve?

**Selected:** `approve` — Approve §3 — continue to packaging/install

### Q: Design §4 Packaging — Approve?

**User (as-is):** Don't need to explain differences with dwarvesf/herdr-quicklook At the top of a readme.md file - It could be provided at the bottom. At top of README.md we need to explain the problem we are solving - too many actions to open (mostly Markdown) file content with agent plan (or alike) from herdr. Also skip note on lowering FV `preview_max_*`. If FV is not installed then open `less` above existing pane but with reduced functionality (need to state it in REDME.md)

### Q: Design §4 revised — Approve?

**Selected:** `approve` — Approve revised §4 — write the design spec

### Q: Spec user review gate?

**User (as-is):** I want to pass implementation to another agentic session. So let's bootstrap new repository, Enrich spec/plan with All the details and all our Question and the answer turns (With all my answers as is), Provide to me steps to run new Agent from this new repository.
