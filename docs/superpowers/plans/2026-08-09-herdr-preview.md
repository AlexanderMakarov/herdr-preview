# herdr-preview Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship a Rust Herdr plugin that shows a hint overlay on visible pane text and opens picked paths in herdr-file-viewer (or `less` overlay if FV is missing), while sending http(s) to the system browser and never registering link handlers.

**Architecture:** One plugin crate: tokenize/classify/resolve in Rust; hint overlay via Herdr popup/overlay pane; `open_preview` prefers FV `HERDR_FILE_VIEWER_OPEN` summon, else spawns `less` in an overlay. No `[[link_handlers]]` in the manifest.

**Tech Stack:** Rust (edition 2021+), Herdr plugin manifest (`herdr-plugin.toml`), Herdr CLI IPC (`herdr pane …` / `plugin pane open` / `plugin action invoke` as available), `less`, system URL opener (`xdg-open` / `open`).

## Global Constraints

- Platforms MVP: Linux + macOS only.
- Scan source: focused pane **visible** text only (not full scrollback).
- Never add `[[link_handlers]]` or git-host / `gh pr view` Ctrl+click hijacks.
- Prefer herdr-file-viewer; fallback `less` overlay with reduced functionality (documented in README).
- Path open shapes for FV: `path`, `path:N`, `path:A-B` only.
- Tokenization must handle spaces in paths and `%20`.
- README problem-first; quicklook credit only at bottom.
- Do not implement large-file / glow performance policy in MVP (open investigation).
- License: MIT.
- Follow the approved design: `docs/superpowers/specs/2026-08-09-herdr-preview-design.md`.

---

## File map (expected)

| Path | Responsibility |
| --- | --- |
| `Cargo.toml` | Crate metadata, deps (serde, regex/unicode path helpers as needed) |
| `src/main.rs` | CLI entry / subcommands dispatched by Herdr actions |
| `src/tokenize.rs` | Find path/URL candidate spans in a text snapshot |
| `src/classify.rs` | Resolve candidates vs cwd → File / Dir / Url / Missing |
| `src/open.rs` | Browser open; FV summon; `less` overlay spawn |
| `src/hint.rs` | Build hint list / assign keys; overlay input loop glue |
| `src/herdr_ipc.rs` | Thin wrappers around `herdr` CLI JSON |
| `herdr-plugin.toml` | panes + actions; **no** link_handlers |
| `scripts/open-hint.sh` (optional) | Idempotent launcher if Herdr expects a shell entry |
| `tests/*.rs` | Unit + mocked-CLI integration |
| `README.md` / `AGENTS.md` | User + agent docs (already stubbed) |

---

### Task 1: Cargo crate skeleton + CI smoke

**Files:**
- Create: `Cargo.toml`, `src/main.rs`, `.gitignore`, `.github/workflows/ci.yml` (optional but recommended)
- Test: `cargo test` empty/`it_compiles`

**Interfaces:**
- Produces: binary crate name `herdr-preview`

- [ ] **Step 1:** Create `Cargo.toml` with package name `herdr-preview`, MIT license, edition 2021.
- [ ] **Step 2:** `src/main.rs` prints usage / version and exits 0 for `--help`.
- [ ] **Step 3:** `cargo test` / `cargo build` succeeds.
- [ ] **Step 4:** Commit: `chore: bootstrap herdr-preview crate`.

---

### Task 2: Tokenizer (TDD)

**Files:**
- Create: `src/tokenize.rs`, `tests/tokenize.rs`
- Modify: `src/main.rs` or `src/lib.rs` to expose modules (`lib.rs` recommended for testing)

**Interfaces:**
- Produces: `pub struct Span { pub start: usize, pub end: usize, pub raw: String }`
- Produces: `pub fn find_candidates(text: &str) -> Vec<Span>`

- [ ] **Step 1:** Write failing tests:
  - bare relative `docs/foo.md`
  - absolute `/tmp/x`
  - spaced path `…/Tray status icon-….plan.md` must be one token when clearly path-shaped (match the behavior fixed in quicklook PR #74 — do not whitespace-split mid-path when the token continues with path chars)
  - `%20` in path kept in `raw` for later decode
  - `https://github.com/org/repo/pull/1` as URL candidate
  - `src/app.rs:42` and `src/app.rs:10-20` keep suffix in `raw`
- [ ] **Step 2:** Implement minimal `find_candidates`.
- [ ] **Step 3:** Tests pass.
- [ ] **Step 4:** Commit: `feat: tokenize path and URL candidates`.

---

### Task 3: Classify + resolve (TDD)

**Files:**
- Create: `src/classify.rs`, `tests/classify.rs`

**Interfaces:**
- Consumes: `Span` / raw strings + `cwd: &Path`
- Produces:
  ```rust
  pub enum Target {
      File { path: PathBuf, open_spec: String }, // open_spec for FV / less (+line)
      Dir { path: PathBuf, open_spec: String },
      Url(String),
      Missing { display: String },
  }
  pub fn classify(raw: &str, cwd: &Path) -> Target
  ```
- `open_spec` rules: strip `file://` if present; percent-decode for FS; keep `:N` / `:A-B`; resolve relative against `cwd`.

- [ ] **Step 1:** Failing tests for file/dir/url/missing, `%20` decode, `path:N`, `path:A-B`, relative resolve.
- [ ] **Step 2:** Implement `classify`.
- [ ] **Step 3:** Tests pass.
- [ ] **Step 4:** Commit: `feat: classify and resolve preview targets`.

---

### Task 4: Open backends (TDD with mocks)

**Files:**
- Create: `src/open.rs`, `tests/open.rs`

**Interfaces:**
- Produces:
  ```rust
  pub enum OpenBackend { FileViewer, Less }
  pub fn detect_file_viewer(herdr_bin: &Path) -> bool; // e.g. `herdr plugin list` parse
  pub fn open_url(url: &str) -> io::Result<()>;
  pub fn open_file_viewer(open_spec: &str, cwd: &Path) -> io::Result<()>;
  pub fn open_less(path: &Path, line: Option<u32>, /* pane spawn args */) -> io::Result<()>;
  ```
- FV: set `HERDR_FILE_VIEWER_OPEN` and invoke the same summon pattern FV documents (`herdr plugin action invoke herdr-file-viewer.open-file-viewer` and/or `plugin pane open` with env). Inspect installed FV scripts under `~/.config/herdr/plugins/github/herdr-file-viewer-*` for the canonical summon; prefer reuse over inventing a new env protocol.
- Less: Herdr overlay/popup placement above current pane; `less +N` when line known; directory → error (caller shows notice).
- Browser: `xdg-open` (Linux) / `open` (macOS).

- [ ] **Step 1:** Unit tests with a fake `herdr` stub on PATH (script writing argv to a log).
- [ ] **Step 2:** Implement detectors + openers behind the stub.
- [ ] **Step 3:** Assert FV path never calls `gh`; URL path never calls FV/`less`.
- [ ] **Step 4:** Commit: `feat: open via file-viewer or less and browser`.

---

### Task 5: Herdr IPC — read visible pane + cwd

**Files:**
- Create: `src/herdr_ipc.rs`, `tests/herdr_ipc.rs`

**Interfaces:**
- Produces: `pub struct PaneSnapshot { pub cwd: PathBuf, pub visible_text: String, pub pane_id: String }`
- Produces: `pub fn read_focused_snapshot() -> Result<PaneSnapshot, …>`

- [ ] **Step 1:** Discover exact Herdr CLI for visible buffer + focused cwd (from Herdr docs / `herdr help` / patterns in `~/code/herdr-quicklook`). Pin the commands in comments.
- [ ] **Step 2:** Mocked tests parse fixture JSON/text.
- [ ] **Step 3:** Implement live wrappers.
- [ ] **Step 4:** Commit: `feat: read focused pane snapshot via herdr CLI`.

---

### Task 6: Hint overlay action

**Files:**
- Create: `src/hint.rs`, `herdr-plugin.toml`, possibly `scripts/run-hint.sh`
- Modify: `src/main.rs` subcommand `hint`

**Interfaces:**
- Consumes: snapshot + classify + open
- Behavior: assign easy keys (a–z excluding ambiguous); draw/list targets in overlay; on key → open; `q`/Esc → cancel.
- Manifest: `[[actions]]` hint; `[[panes]]` for overlay if required. **Confirm manifest has zero `[[link_handlers]]`.**

- [ ] **Step 1:** Wire `herdr-preview hint` that: snapshot → candidates → print targets (headless mode for tests).
- [ ] **Step 2:** Add overlay pane placement matching Herdr popup/overlay (study quicklook `QUICKLOOK_OPEN_PLACEMENT` / manifest for the supported placement name in current Herdr).
- [ ] **Step 3:** Manual smoke in Herdr: `prefix+v` binding documented in README.
- [ ] **Step 4:** Commit: `feat: hint overlay action and plugin manifest`.

---

### Task 7: Peer detect routing end-to-end

**Files:**
- Modify: `src/open.rs`, `src/hint.rs`
- Test: `tests/routing.rs`

- [ ] **Step 1:** Test: FV listed in stub `plugin list` → OPEN summon with env.
- [ ] **Step 2:** Test: FV absent → `less` overlay spawn; dirs skip with notice.
- [ ] **Step 3:** Test: http(s) → browser helper only.
- [ ] **Step 4:** Commit: `feat: route picks to FV, less, or browser`.

---

### Task 8: README install + keybinding + fallback copy

**Files:**
- Modify: `README.md`

- [ ] **Step 1:** Add install (`herdr plugin install github:AlexanderMakarov/herdr-preview@…` once tags exist, or path install for dev).
- [ ] **Step 2:** Document `prefix+v` binding snippet for `~/.config/herdr/config.toml`.
- [ ] **Step 3:** Document FV vs `less` reduced mode (required by design).
- [ ] **Step 4:** Keep Credits at bottom.
- [ ] **Step 5:** Commit: `docs: install, keybinding, and less fallback`.

---

### Task 9: Manual acceptance checklist

- [ ] Agent-printed Markdown relative path in a git repo pane → hint → opens in FV at file (and `:N` if present).
- [ ] Spaced path file on disk → opens correctly.
- [ ] GitHub PR URL via hint → browser; Ctrl+click same URL with plugin installed → browser (not `gh` / GraphQL Projects message in terminal).
- [ ] Uninstall/disable FV (or stub absence) → `less` overlay opens file; `q` in less returns to prior app.
- [ ] `q` on hint overlay cancels without opening anything.
- [ ] Commit any bugfixes; tag `v0.1.0` when ready for install.

---

## Execution handoff prompt (paste into a new agent)

```
Implement herdr-preview from the approved plan in this repo.

1. Read AGENTS.md and docs/superpowers/specs/2026-08-09-herdr-preview-design.md (including Brainstorm Q&A).
2. Use superpowers:subagent-driven-development (or executing-plans) on docs/superpowers/plans/2026-08-09-herdr-preview.md.
3. Respect Global Constraints: no [[link_handlers]], FV preferred / less fallback, visible-scan only, Linux+macOS.
4. Do not expand scope into large-Markdown performance policy without a new design decision.
```
