# README demo GIF — design

Status: approved (2026-11-23); demo GIF shipped in README

## Intent

Ship a short looping GIF in `README.md` that shows herdr-preview’s two-keystroke flow, for promotion after the v0.1.1 release. Viewers should understand: paths on screen → hint overlay → letter → file opens in herdr-file-viewer.

## Goals

- Autoplaying GIF on GitHub README (no click-to-play).
- Visible keystrokes (`Ctrl+A`, `/`, letter) via screenkey baked into the recording.
- Visible overlay scroll (content moves); mouse buttons via screenkey `-M` when clicked; scroll-wheel proof is the overlay itself moving.
- Keep README **problem-first**: GIF sits under the opening problem paragraph, before Install.
- Neutral demo content only (no real names, usernames, or `/home/<user>` paths in the captured pane).

## Non-goals

- Voiceover / audio (GIF is silent).
- Showing `less` fallback or directory browse in this clip.
- Post-processed fake key badges (Approach B).
- showmethekey (Approach C; unnecessary on this X11 machine).

## Approaches considered

| Approach | Summary | Trade-off |
| --- | --- | --- |
| **A (chosen)** | screenkey during Kazam → trim MP4 → palette GIF → commit under `docs/assets/` | Best fidelity; one apt package; scroll wheel may only show as Button4/5 |
| B | Bare Kazam + ffmpeg timed labels | No new app; timing fiddly, looks fake |
| C | showmethekey | Wayland-oriented; polkit overhead with no gain on X11 |

## Design

### Storyboard (~15–25 s wall time before any speed-up)

1. Herdr pane shows agent-style output with several file paths (neutral stand-ins).
2. Press `Ctrl+A` then `/` (bound hint key) — screenkey shows the chord.
3. Preview overlay appears with letter hints; scroll up/down so viewers see more of the tail.
4. Press one letter for a specific file.
5. herdr-file-viewer opens as a pane (right split / normal FV open); hold ~2 s so the content is readable.
6. End. No credits, no install steps in the clip.

### Capture

- Recorder: **Kazam** → MP4.
- Overlay: `screenkey -p bottom -s large -M --mods-mode normal` (mouse buttons on; position away from Herdr’s yellow status band if it clashes — move to `top` if needed).
- Region: crop to the Herdr window (or a tight desktop region) so the GIF stays readable at README width.
- Privacy: stage pane text with stand-ins like `docs/plans/demo-notes.md`, never real home paths.

### Convert / ship

- Trim dead head/tail from the MP4 (ffmpeg / screen-recording-editor `cut.py`).
- Optional light speed-up only on idle holds; keep keypress and scroll at 1×.
- Encode GIF with palette (`palettegen` + `paletteuse`), target roughly **2–5 MB**, ≤ ~800 px wide if needed for GitHub.
- Asset path: `docs/assets/herdr-preview-demo.gif`.
- README: one image under the problem paragraph, alt text describing the flow; do not move Credits or change Install copy beyond the media insert.

### Builder calls (unless corrected)

- No voiceover.
- Prefer committing the GIF in-repo over external hosting.
- Do not add a second GIF for `less` fallback in this pass.

## Success

Someone skimming the README sees the GIF, understands hint → letter → file-viewer, and the problem paragraph still leads.
