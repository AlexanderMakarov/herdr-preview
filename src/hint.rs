//! Hint overlay: snapshot → tokenize → classify → key assignment → overlay pick.
//!
//! CRITICAL: `read_focused_snapshot()` runs in the **action** process before spawning
//! the overlay pane. The overlay must never call `herdr pane read` (deadlock).

use std::fmt;
use std::fs;
use std::io::{self, Read, Write};
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::classify::{
    classify, classify_with_fallbacks, discover_worktree_roots, exists_on_disk, is_worktree_dir,
    Target,
};
use crate::herdr_ipc::{notify, read_focused_snapshot, HerdrIpcError, PaneSnapshot};
use crate::open::{detect_file_viewer, open_browse, open_file_viewer, open_less, open_url};
use crate::tokenize::{find_candidates, has_file_extension, spaced_continuation_ends, Span};

/// Home-row-first keys; excludes `q` (cancel) and visually ambiguous letters per quicklook.
pub const HINT_KEYS: &str = "asdfghjklwertyuiopzxcvbnm";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HintEntry {
    /// Letter to open this target. `None` for missing paths (red highlight only).
    pub key: Option<char>,
    /// Byte offset into the origin snapshot (`visible_text`).
    pub start: usize,
    pub end: usize,
    pub raw: String,
    pub target: Target,
}

#[derive(Debug)]
pub enum HintError {
    HerdrIpc(HerdrIpcError),
    Io(io::Error),
    NoOpenableTargets,
    OverlayEnv(String),
    UnknownKey(char),
}

impl fmt::Display for HintError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::HerdrIpc(err) => write!(f, "{err}"),
            Self::Io(err) => write!(f, "{err}"),
            Self::NoOpenableTargets => write!(f, "no openable targets in visible pane text"),
            Self::OverlayEnv(msg) => write!(f, "overlay env: {msg}"),
            Self::UnknownKey(key) => write!(f, "unknown hint key: {key}"),
        }
    }
}

impl std::error::Error for HintError {}

impl From<HerdrIpcError> for HintError {
    fn from(value: HerdrIpcError) -> Self {
        Self::HerdrIpc(value)
    }
}

impl From<io::Error> for HintError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

pub fn build_entries(text: &str, cwd: &Path) -> Vec<HintEntry> {
    // Keep every on-screen span. Agent panes often repeat the same absolute
    // path (e.g. inside `$ ls -la …` and again as a clean follow-up line);
    // dropping duplicates by `raw` left later copies unhighlighted.
    let spans: Vec<Span> = extend_spans_on_disk(text, find_candidates(text), cwd);

    // Classify once per distinct raw, then reuse — shared letter for repeats.
    let mut raw_order: Vec<String> = Vec::new();
    for span in &spans {
        if !raw_order.iter().any(|existing| existing == &span.raw) {
            raw_order.push(span.raw.clone());
        }
    }

    // Classify everything against the pane cwd first so later-visible worktree
    // dirs can rescue earlier missing relative paths in a second pass.
    let primary_by_raw: Vec<(String, Target)> = raw_order
        .iter()
        .map(|raw| (raw.clone(), classify(raw, cwd)))
        .collect();
    let mut fallbacks: Vec<PathBuf> = Vec::new();
    for (_, target) in &primary_by_raw {
        if let Target::Dir { path, .. } = target {
            if is_worktree_dir(path) && !fallbacks.iter().any(|existing| existing == path) {
                fallbacks.push(path.clone());
            }
        }
    }
    // Also probe on-disk worktrees under this repo — agent panes often keep
    // pane.cwd on main while the spoken path lives only in a worktree, and that
    // worktree line may be scrolled off the visible snapshot.
    for path in discover_worktree_roots(cwd) {
        if !fallbacks.iter().any(|existing| existing == &path) {
            fallbacks.push(path);
        }
    }

    let mut target_by_raw: Vec<(String, Target)> = Vec::new();
    for (raw, primary_target) in primary_by_raw {
        let target = if matches!(primary_target, Target::Missing { .. }) && !fallbacks.is_empty() {
            classify_with_fallbacks(&raw, cwd, &fallbacks)
        } else {
            primary_target
        };
        target_by_raw.push((raw, target));
    }

    // Letters are scarce — assign from the newest snapshot lines downward so the
    // default bottom-aligned overlay puts `a` on fresh output, not scrollback top.
    let mut newest_pos_by_raw: std::collections::HashMap<String, (usize, usize)> =
        std::collections::HashMap::new();
    for span in &spans {
        let line = line_index_at(text, span.start);
        newest_pos_by_raw
            .entry(span.raw.clone())
            .and_modify(|(best_line, best_start)| {
                if line > *best_line || (line == *best_line && span.start > *best_start) {
                    *best_line = line;
                    *best_start = span.start;
                }
            })
            .or_insert((line, span.start));
    }

    let mut key_assign_order: Vec<usize> = (0..target_by_raw.len()).collect();
    key_assign_order.sort_by(|&i, &j| {
        let (raw_i, _) = &target_by_raw[i];
        let (raw_j, _) = &target_by_raw[j];
        let pos_i = newest_pos_by_raw
            .get(raw_i)
            .copied()
            .unwrap_or((0, 0));
        let pos_j = newest_pos_by_raw
            .get(raw_j)
            .copied()
            .unwrap_or((0, 0));
        pos_j
            .0
            .cmp(&pos_i.0)
            .then_with(|| pos_j.1.cmp(&pos_i.1))
            .then_with(|| raw_i.cmp(raw_j))
    });

    let mut key_by_raw: Vec<(String, Option<char>)> = Vec::new();
    let mut key_index = 0usize;
    for idx in key_assign_order {
        let (raw, target) = &target_by_raw[idx];
        // Overlay shows files, dirs, and missing path-like tokens (red). http(s) stays Ctrl+click.
        if matches!(target, Target::Url(_)) {
            continue;
        }
        let key = if matches!(target, Target::Missing { .. }) {
            None
        } else {
            match hint_key_for_index(key_index) {
                Some(k) => {
                    key_index += 1;
                    Some(k)
                }
                None => continue,
            }
        };
        key_by_raw.push((raw.clone(), key));
    }

    let mut entries = Vec::new();
    for span in spans {
        let Some((_, key)) = key_by_raw.iter().find(|(raw, _)| raw == &span.raw) else {
            continue;
        };
        let Some((_, target)) = target_by_raw.iter().find(|(raw, _)| raw == &span.raw) else {
            continue;
        };
        entries.push(HintEntry {
            key: *key,
            start: span.start,
            end: span.end,
            raw: span.raw,
            target: target.clone(),
        });
    }

    entries
}

/// Text heuristics can't tell a spaced name like `Project files` (a real dir)
/// from prose, so grow each slash-bearing span through the following words to
/// the longest run that exists on disk, then drop the fragments it swallowed.
/// 0-based line index in `text` for a byte offset (before the newline ending that line).
fn line_index_at(text: &str, byte_offset: usize) -> usize {
    text.get(..byte_offset.min(text.len()))
        .map(|prefix| prefix.bytes().filter(|&b| b == b'\n').count())
        .unwrap_or(0)
}

fn extend_spans_on_disk(text: &str, spans: Vec<Span>, cwd: &Path) -> Vec<Span> {
    let mut out: Vec<Span> = Vec::with_capacity(spans.len());
    for mut span in spans {
        if out.last().is_some_and(|prev| span.start < prev.end) {
            continue;
        }
        if span.raw.contains('/') && !span.raw.contains("://") && !has_file_extension(&span.raw) {
            let longest = spaced_continuation_ends(text, span.end)
                .into_iter()
                .rev()
                .find(|&end| exists_on_disk(&text[span.start..end], cwd));
            if let Some(end) = longest {
                span.end = end;
                span.raw = text[span.start..end].to_owned();
            }
        }
        out.push(span);
    }
    out
}

pub fn hint_key_for_index(index: usize) -> Option<char> {
    HINT_KEYS.chars().nth(index)
}

pub fn target_kind_label(target: &Target) -> &'static str {
    match target {
        Target::File {
            ambiguous: true, ..
        } => "file-warn",
        Target::Dir {
            ambiguous: true, ..
        } => "dir-warn",
        Target::File { .. } => "file",
        Target::Dir { .. } => "dir",
        Target::Url(_) => "url",
        Target::Missing { .. } => "missing",
    }
}

fn hint_token_colors<'a>(
    target: &Target,
    unique_key: &'a str,
    unique_tok: &'a str,
    warn_key: &'a str,
    warn_tok: &'a str,
    miss_key: &'a str,
    miss_tok: &'a str,
) -> (&'a str, &'a str) {
    match target {
        Target::Missing { .. } => (miss_key, miss_tok),
        Target::File {
            ambiguous: true, ..
        }
        | Target::Dir {
            ambiguous: true, ..
        } => (warn_key, warn_tok),
        _ => (unique_key, unique_tok),
    }
}

pub fn format_list(entries: &[HintEntry]) -> String {
    serialize_entries(entries)
}

fn open_spec_for_target(target: &Target) -> String {
    match target {
        Target::File { open_spec, .. } | Target::Dir { open_spec, .. } => open_spec.clone(),
        Target::Url(url) => url.clone(),
        Target::Missing { display } => display.clone(),
    }
}

pub fn run_hint_list(text: &str, cwd: &Path) -> Result<String, HintError> {
    let entries = build_entries(text, cwd);
    Ok(format_list(&entries))
}

pub fn run_hint_action() -> Result<(), HintError> {
    let snapshot = read_focused_snapshot()?;
    let entries = build_entries(&snapshot.visible_text, &snapshot.cwd);
    if entries.is_empty() {
        notify(
            "herdr-preview",
            "No openable paths/URLs in the visible pane text",
        );
        return Err(HintError::NoOpenableTargets);
    }
    spawn_hint_overlay(&entries, &snapshot)
}

fn resolve_herdr_bin() -> PathBuf {
    std::env::var_os("HERDR_BIN_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("herdr"))
}

fn spawn_hint_overlay(entries: &[HintEntry], snapshot: &PaneSnapshot) -> Result<(), HintError> {
    let dir = std::env::temp_dir().join(format!("herdr-preview-hint-{}", std::process::id()));
    fs::create_dir_all(&dir)?;
    let targets_path = dir.join("targets.tsv");
    let snap_path = dir.join("snap.txt");
    fs::write(&targets_path, serialize_entries(entries))?;
    fs::write(&snap_path, &snapshot.visible_text)?;

    let targets_env = format!("HERDR_PREVIEW_HINT_TARGETS={}", targets_path.display());
    let snap_env = format!("HERDR_PREVIEW_HINT_SNAP={}", snap_path.display());
    let cwd_env = format!("HERDR_PREVIEW_HINT_CWD={}", snapshot.cwd.display());
    let origin_env = format!("HERDR_PREVIEW_HINT_ORIGIN={}", snapshot.pane_id);

    let herdr_bin = resolve_herdr_bin();
    let status = Command::new(&herdr_bin)
        .args([
            "plugin",
            "pane",
            "open",
            "--plugin",
            "herdr-preview",
            "--entrypoint",
            "hint-overlay",
            "--placement",
            "overlay",
            "--focus",
            "--env",
            &targets_env,
            "--env",
            &snap_env,
            "--env",
            &cwd_env,
            "--env",
            &origin_env,
        ])
        .status()
        .map_err(|err| {
            if err.kind() == io::ErrorKind::NotFound {
                io::Error::new(io::ErrorKind::NotFound, "herdr not found on PATH")
            } else {
                err
            }
        })?;

    if status.success() {
        Ok(())
    } else {
        Err(HintError::Io(io::Error::other(format!(
            "herdr plugin pane open exited with {status}"
        ))))
    }
}

pub fn serialize_entries(entries: &[HintEntry]) -> String {
    let mut out = String::new();
    for entry in entries {
        let kind = target_kind_label(&entry.target);
        let open_spec = open_spec_for_target(&entry.target);
        let path = path_for_target(&entry.target);
        out.push_str(&format!(
            "{}\t{}\t{}\t{}\t{kind}\t{open_spec}\t{path}\n",
            entry
                .key
                .map(|c| c.to_string())
                .unwrap_or_else(|| "-".into()),
            entry.start,
            entry.end,
            entry.raw
        ));
    }
    out
}

fn path_for_target(target: &Target) -> String {
    match target {
        Target::File { path, .. } | Target::Dir { path, .. } => path.display().to_string(),
        Target::Url(url) => url.clone(),
        Target::Missing { display } => display.clone(),
    }
}

pub fn parse_entries_tsv(data: &str) -> Result<Vec<HintEntry>, HintError> {
    let mut entries = Vec::new();
    for line in data.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let parts: Vec<&str> = line.splitn(7, '\t').collect();
        if parts.len() < 7 {
            return Err(HintError::OverlayEnv(format!("bad targets line: {line}")));
        }
        let key = if parts[0] == "-" || parts[0].is_empty() {
            None
        } else {
            Some(
                parts[0]
                    .chars()
                    .next()
                    .ok_or_else(|| HintError::OverlayEnv("empty key".into()))?,
            )
        };
        let start: usize = parts[1]
            .parse()
            .map_err(|_| HintError::OverlayEnv(format!("bad start: {}", parts[1])))?;
        let end: usize = parts[2]
            .parse()
            .map_err(|_| HintError::OverlayEnv(format!("bad end: {}", parts[2])))?;
        let raw = parts[3].to_string();
        let kind = parts[4];
        let open_spec = parts[5].to_string();
        let path = parts[6];
        let target = match kind {
            "file" | "file-warn" => Target::File {
                path: PathBuf::from(path),
                open_spec,
                ambiguous: kind == "file-warn",
            },
            "dir" | "dir-warn" => Target::Dir {
                path: PathBuf::from(path),
                open_spec,
                ambiguous: kind == "dir-warn",
            },
            "url" => Target::Url(open_spec),
            "missing" => Target::Missing { display: open_spec },
            other => {
                return Err(HintError::OverlayEnv(format!("unknown kind: {other}")));
            }
        };
        entries.push(HintEntry {
            key,
            start,
            end,
            raw,
            target,
        });
    }
    Ok(entries)
}

pub fn run_hint_overlay() -> Result<(), HintError> {
    let targets_path = std::env::var("HERDR_PREVIEW_HINT_TARGETS")
        .map_err(|_| HintError::OverlayEnv("HERDR_PREVIEW_HINT_TARGETS missing".into()))?;
    let snap_path = std::env::var("HERDR_PREVIEW_HINT_SNAP")
        .map_err(|_| HintError::OverlayEnv("HERDR_PREVIEW_HINT_SNAP missing".into()))?;
    let cwd = std::env::var("HERDR_PREVIEW_HINT_CWD")
        .map(PathBuf::from)
        .map_err(|_| HintError::OverlayEnv("HERDR_PREVIEW_HINT_CWD missing".into()))?;
    let origin_pane = std::env::var("HERDR_PREVIEW_HINT_ORIGIN").ok();

    let entries = parse_entries_tsv(&fs::read_to_string(&targets_path)?)?;
    let snapshot = fs::read_to_string(&snap_path)?;

    let choice = overlay_pick(&entries, &snapshot)?;
    match choice {
        OverlayChoice::Cancel => Ok(()),
        OverlayChoice::Pick(index) => open_entry(&entries[index], &cwd, origin_pane.as_deref()),
    }
}

enum OverlayChoice {
    Cancel,
    Pick(usize),
}

fn overlay_pick(entries: &[HintEntry], snapshot: &str) -> Result<OverlayChoice, HintError> {
    let mut stdout = io::stdout();
    let mut tty = open_tty()?;
    let _raw = RawMode::enter(tty.as_raw_fd())?;
    let _term = TerminalGuard::enter(&mut stdout)?;
    let mut scroll_up = 0usize;
    let mut leftover = Vec::new();

    loop {
        let (rows, cols) = tty_size(tty.as_raw_fd()).unwrap_or((24, 80));
        render_overlay(
            &mut stdout,
            entries,
            snapshot,
            rows,
            cols,
            scroll_up,
        )?;
        stdout.flush()?;

        let Some(key) = read_hint_key(&mut tty, &mut leftover)? else {
            continue;
        };
        let layout = overlay_layout(rows as usize);
        let total = trim_trailing_empty_lines_for_count(snapshot);
        let max_scroll = max_snapshot_scroll_up(total, layout.content_slots);

        match key {
            HintKey::Cancel => return Ok(OverlayChoice::Cancel),
            HintKey::ScrollUp { lines } => {
                scroll_up = (scroll_up + lines).min(max_scroll);
            }
            HintKey::ScrollDown { lines } => {
                scroll_up = scroll_up.saturating_sub(lines);
            }
            HintKey::Byte(key) => {
                if matches!(key, b'q' | b'Q') {
                    return Ok(OverlayChoice::Cancel);
                }
                if let Some(index) = entries
                    .iter()
                    .position(|entry| entry.key == Some(key as char))
                {
                    return Ok(OverlayChoice::Pick(index));
                }
            }
        }
    }
}

const ESC: u8 = 0x1b;
const ANSI_HOME: &str = "\x1b[H";
const ANSI_CLEAR: &str = "\x1b[J";
const ANSI_HIDE: &str = "\x1b[?25l";
const ANSI_SHOW: &str = "\x1b[?25h";
const ANSI_WRAP_OFF: &str = "\x1b[?7l";
const ANSI_WRAP_ON: &str = "\x1b[?7h";
const ANSI_MOUSE_ON: &str = "\x1b[?1000h\x1b[?1006h";
const ANSI_MOUSE_OFF: &str = "\x1b[?1000l\x1b[?1006l";
const ANSI_CLR_EOL: &str = "\x1b[K";

/// RAII: restore cursor + wrap when the overlay exits for any reason.
struct TerminalGuard;

impl TerminalGuard {
    fn enter(out: &mut impl Write) -> io::Result<Self> {
        write!(out, "{ANSI_HIDE}{ANSI_WRAP_OFF}{ANSI_MOUSE_ON}")?;
        out.flush()?;
        Ok(Self)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let mut stdout = io::stdout();
        let _ = write!(stdout, "{ANSI_MOUSE_OFF}{ANSI_SHOW}{ANSI_WRAP_ON}");
        let _ = stdout.flush();
    }
}

/// Put the pane TTY into non-canonical, no-echo mode so single keypresses
/// arrive immediately (same contract as quicklook's `read -rsn1`).
struct RawMode {
    fd: libc::c_int,
    original: libc::termios,
}

impl RawMode {
    fn enter(fd: libc::c_int) -> io::Result<Self> {
        let mut original = unsafe { std::mem::zeroed::<libc::termios>() };
        if unsafe { libc::tcgetattr(fd, &mut original) } != 0 {
            return Err(io::Error::last_os_error());
        }
        let mut raw = original;
        // True cbreak: byte-at-a-time, no echo; keep ISIG so Ctrl+C can still kill
        // a wedged overlay during development.
        // Do NOT clear OPOST — paint uses writeln! (`\n`). With OPOST off, `\n` is
        // bare LF (cursor down, same column) → blank screen + “ladder” of tokens.
        // That regressed when paint moved after RawMode::enter (geometry paint).
        raw.c_lflag &= !(libc::ECHO | libc::ICANON | libc::IEXTEN);
        raw.c_iflag &= !(libc::IXON | libc::ICRNL);
        raw.c_cc[libc::VMIN] = 1;
        raw.c_cc[libc::VTIME] = 0;
        if unsafe { libc::tcsetattr(fd, libc::TCSANOW, &raw) } != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self { fd, original })
    }
}

impl Drop for RawMode {
    fn drop(&mut self) {
        unsafe {
            libc::tcsetattr(self.fd, libc::TCSANOW, &self.original);
        }
    }
}

enum HintKey {
    Byte(u8),
    Cancel,
    ScrollUp { lines: usize },
    ScrollDown { lines: usize },
}

/// Line window into the snapshot tail: `scroll_up` 0 = bottom-aligned; larger = older lines.
fn snapshot_line_window(total: usize, slots: usize, scroll_up: usize) -> (usize, usize) {
    let max_up = max_snapshot_scroll_up(total, slots);
    let scroll_up = scroll_up.min(max_up);
    let end = total.saturating_sub(scroll_up);
    let start = end.saturating_sub(slots);
    (start, end)
}

fn max_snapshot_scroll_up(total: usize, slots: usize) -> usize {
    total.saturating_sub(slots)
}

fn trim_trailing_empty_lines_for_count(snapshot: &str) -> usize {
    let mut lines = split_snapshot_lines(snapshot);
    trim_trailing_empty_lines(&mut lines);
    lines.len()
}

/// Herdr overlay panes draw left/right border columns (no manifest flag to disable).
/// We paint at the same column indices as the origin pane snapshot (`stty` cols);
/// the frame clips ~[`OVERLAY_PANE_BORDER_COLS`] cells on the right. Do not reflow
/// lines to inner width — that would diverge from pure-shell tab line breaks.
#[allow(dead_code)]
const OVERLAY_PANE_BORDER_COLS: usize = 2;

fn overlay_paint_cols(cols: u16) -> usize {
    cols.max(20) as usize
}

/// 1-indexed terminal rows for overlay height (see `overlay_layout`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct OverlayLayout {
    top_border: usize,
    content_top: usize,
    content_bottom: usize,
    content_slots: usize,
    status_row: usize,
}

/// Map `stty` height to rows we paint. Herdr's overlay pane draws its own bottom
/// edge; we do not paint a bottom rule (that extra `CUP` row looked like a blank
/// band above the yellow status). Title/chrome may consume the top row — leave
/// one row at the bottom undrawn so status + content stay adjacent.
fn overlay_layout(stty_rows: usize) -> OverlayLayout {
    let h = stty_rows.max(5).saturating_sub(1);
    OverlayLayout {
        top_border: 1,
        content_top: 2,
        content_bottom: h.saturating_sub(1),
        content_slots: h.saturating_sub(2).max(1),
        status_row: h,
    }
}

fn horizontal_rule(cols: usize) -> String {
    let n = cols.min(512);
    "─".repeat(n)
}

fn write_terminal_row(out: &mut impl Write, row: usize, text: &str) -> io::Result<()> {
    write!(out, "\x1b[{row};1H{text}{ANSI_CLR_EOL}")?;
    Ok(())
}

/// Soft-wrap for display: CSI sequences pass through and do not consume width.
#[allow(dead_code)]
fn wrap_line_preserve_ansi(text: &str, cols: usize) -> Vec<String> {
    let cols = cols.max(1);
    if text.is_empty() {
        return vec![String::new()];
    }
    let mut rows: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut cells = 0usize;
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' {
            current.push(ch);
            if chars.peek() == Some(&'[') {
                current.push(chars.next().unwrap());
                for c in chars.by_ref() {
                    current.push(c);
                    if c.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            continue;
        }
        if cells >= cols {
            rows.push(current);
            current = String::new();
            cells = 0;
        }
        current.push(ch);
        cells += 1;
    }
    rows.push(current);
    rows
}

fn paint_logical_line(
    line: &mut String,
    line_origin: usize,
    entries_here: &[&HintEntry],
    h_key: &str,
    h_tok: &str,
    h_key_warn: &str,
    h_tok_warn: &str,
    h_key_miss: &str,
    h_tok_miss: &str,
    reset: &str,
    dim: &str,
) {
    let mut ordered = entries_here.to_vec();
    ordered.sort_by_key(|e| std::cmp::Reverse(e.start));
    for entry in ordered {
        let rel = entry.start.saturating_sub(line_origin);
        if rel + entry.raw.len() <= line.len() && line[rel..].starts_with(&entry.raw) {
            paint_token_at(
                line,
                rel,
                entry,
                h_key,
                h_tok,
                h_key_warn,
                h_tok_warn,
                h_key_miss,
                h_tok_miss,
                reset,
                dim,
            );
        }
    }
}

/// Bottom-aligned display rows: one snapshot line per row, truncate (no reflow).
fn build_logical_display_rows(
    snap_lines: &[String],
    line_starts: &[usize],
    entries: &[HintEntry],
    content_slots: usize,
    scroll_up: usize,
    cols: usize,
    h_key: &str,
    h_tok: &str,
    h_key_warn: &str,
    h_tok_warn: &str,
    h_key_miss: &str,
    h_tok_miss: &str,
    reset: &str,
    dim: &str,
) -> Vec<String> {
    let total = snap_lines.len();
    let mut by_abs: Vec<Vec<&HintEntry>> = vec![Vec::new(); total];
    for entry in entries {
        let Some(abs) = line_starts
            .windows(2)
            .position(|w| entry.start >= w[0] && entry.start < w[1])
        else {
            continue;
        };
        by_abs[abs].push(entry);
    }

    let (start, end) = snapshot_line_window(total, content_slots, scroll_up);
    let mut display: Vec<String> = Vec::with_capacity(content_slots);
    for abs in start..end {
        let mut line = snap_lines[abs].clone();
        let origin = line_starts.get(abs).copied().unwrap_or(0);
        paint_logical_line(
            &mut line,
            origin,
            &by_abs[abs],
            h_key,
            h_tok,
            h_key_warn,
            h_tok_warn,
            h_key_miss,
            h_tok_miss,
            reset,
            dim,
        );
        display.push(truncate_cells(&line, cols));
    }
    while display.len() < content_slots {
        display.insert(0, String::new());
    }
    display
}

fn render_overlay(
    out: &mut impl Write,
    entries: &[HintEntry],
    snapshot: &str,
    rows: u16,
    cols: u16,
    scroll_up: usize,
) -> io::Result<()> {
    const DIM: &str = "\x1b[2;90m";
    const RESET: &str = "\x1b[0m";
    const H_KEY: &str = "\x1b[0;1;30;48;2;80;200;80m";
    const H_TOK: &str = "\x1b[0;38;2;80;200;80m";
    const H_KEY_WARN: &str = "\x1b[0;1;30;48;2;255;253;1m";
    const H_TOK_WARN: &str = "\x1b[0;38;2;255;253;1m";
    const H_KEY_MISS: &str = "\x1b[0;1;37;48;2;200;60;60m";
    const H_TOK_MISS: &str = "\x1b[0;38;2;220;80;80m";

    let layout = overlay_layout(rows as usize);
    let cols = overlay_paint_cols(cols);

    let mut snap_lines = split_snapshot_lines(snapshot);
    trim_trailing_empty_lines(&mut snap_lines);
    let total = snap_lines.len();
    let tail_lines = total;
    let mut line_starts = Vec::with_capacity(total + 1);
    let mut cursor = 0usize;
    for line in &snap_lines {
        line_starts.push(cursor);
        cursor += line.len();
        if snapshot.as_bytes().get(cursor) == Some(&b'\n') {
            cursor += 1;
        }
    }
    line_starts.push(cursor);

    let (win_start, win_end) =
        snapshot_line_window(total, layout.content_slots, scroll_up);
    let display_rows = build_logical_display_rows(
        &snap_lines,
        &line_starts,
        entries,
        layout.content_slots,
        scroll_up,
        cols,
        H_KEY,
        H_TOK,
        H_KEY_WARN,
        H_TOK_WARN,
        H_KEY_MISS,
        H_TOK_MISS,
        RESET,
        DIM,
    );

    let distinct = {
        let mut raws: Vec<&str> = entries.iter().map(|e| e.raw.as_str()).collect();
        raws.sort_unstable();
        raws.dedup();
        raws.len()
    };
    let _shown = win_end.saturating_sub(win_start);
    let scroll_note = if scroll_up > 0 {
        format!(" · ↑{scroll_up}")
    } else {
        String::new()
    };
    let line_from = win_start + 1;
    let status = format!(
        " Preview · {distinct} path(s) · L{line_from}-{win_end}/{tail_lines}{scroll_note} · ↑↓ scroll · letter · q/Esc"
    );
    let status = truncate_cells(&status, cols);
    let rule = horizontal_rule(cols);

    write!(out, "{ANSI_HOME}{ANSI_CLEAR}")?;
    write_terminal_row(
        out,
        layout.top_border,
        &format!("{DIM}{rule}{RESET}"),
    )?;
    for (index, line) in display_rows.iter().enumerate() {
        let row = layout.content_top + index;
        if row > layout.content_bottom {
            break;
        }
        write_terminal_row(out, row, &format!("{DIM}{line}{RESET}"))?;
    }
    write_terminal_row(
        out,
        layout.status_row,
        &format!("\x1b[1;30;48;2;255;253;1m{status}{RESET}"),
    )?;
    Ok(())
}

fn trim_trailing_empty_lines(lines: &mut Vec<String>) {
    while lines.last().is_some_and(|line| line.is_empty()) {
        lines.pop();
    }
}

/// Split snapshot text into lines, keeping middle empty rows (bash `read` style).
fn split_snapshot_lines(snapshot: &str) -> Vec<String> {
    let mut lines: Vec<String> = snapshot.split('\n').map(str::to_string).collect();
    if snapshot.ends_with('\n') {
        let _ = lines.pop();
    }
    lines
}

fn style_token(key: char, raw: &str, h_key: &str, h_tok: &str, reset: &str, dim: &str) -> String {
    let rest: String = raw.chars().skip(1).collect();
    format!("{h_key}{key}{reset}{h_tok}{rest}{reset}{dim}")
}

fn paint_token_at(
    line: &mut String,
    at: usize,
    entry: &HintEntry,
    h_key: &str,
    h_tok: &str,
    h_key_warn: &str,
    h_tok_warn: &str,
    h_key_miss: &str,
    h_tok_miss: &str,
    reset: &str,
    dim: &str,
) {
    let (key_s, tok_s) = hint_token_colors(
        &entry.target,
        h_key,
        h_tok,
        h_key_warn,
        h_tok_warn,
        h_key_miss,
        h_tok_miss,
    );
    let styled = match entry.key {
        Some(key) => style_token(key, &entry.raw, key_s, tok_s, reset, dim),
        None => format!("{tok_s}{}{reset}{dim}", entry.raw),
    };
    line.replace_range(at..at + entry.raw.len(), &styled);
}

/// Truncate to about `cols` terminal cells, ignoring CSI sequences for counting.
fn truncate_cells(text: &str, cols: usize) -> String {
    let mut out = String::new();
    let mut cells = 0usize;
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' {
            out.push(ch);
            if chars.peek() == Some(&'[') {
                out.push(chars.next().unwrap());
                for c in chars.by_ref() {
                    out.push(c);
                    if c.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            continue;
        }
        if cells >= cols {
            break;
        }
        out.push(ch);
        cells += 1;
    }
    out
}

fn tty_size(fd: libc::c_int) -> io::Result<(u16, u16)> {
    unsafe {
        let mut ws: libc::winsize = std::mem::zeroed();
        if libc::ioctl(fd, libc::TIOCGWINSZ, &mut ws) != 0 {
            return Err(io::Error::last_os_error());
        }
        if ws.ws_row == 0 || ws.ws_col == 0 {
            return Err(io::Error::other("empty winsize"));
        }
        Ok((ws.ws_row, ws.ws_col))
    }
}

fn open_tty() -> io::Result<fs::File> {
    fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/tty")
}

fn read_hint_key(tty: &mut fs::File, leftover: &mut Vec<u8>) -> io::Result<Option<HintKey>> {
    use crate::browse::parse_browse_input;

    loop {
        if leftover.is_empty() {
            let mut buf = [0u8; 1];
            tty.read_exact(&mut buf)?;
            if buf[0] != ESC {
                leftover.push(buf[0]);
            } else {
                let fd = tty.as_raw_fd();
                let mut pollfd = libc::pollfd {
                    fd,
                    events: libc::POLLIN,
                    revents: 0,
                };
                let ready = unsafe { libc::poll(&mut pollfd, 1, 50) };
                if ready == 0 {
                    return Ok(Some(HintKey::Cancel));
                }
                if ready < 0 {
                    return Err(io::Error::last_os_error());
                }
                leftover.push(ESC);
                let mut extra = [0u8; 32];
                let n = tty.read(&mut extra)?;
                leftover.extend_from_slice(&extra[..n]);
            }
        }

        if let Some(n) = hint_first_input_len(leftover) {
            let seq: Vec<u8> = leftover.drain(..n).collect();
            return Ok(map_browse_to_hint(parse_browse_input(&seq)));
        }

        let mut extra = [0u8; 32];
        let n = tty.read(&mut extra)?;
        if n == 0 {
            leftover.clear();
            return Ok(None);
        }
        leftover.extend_from_slice(&extra[..n]);
    }
}

fn hint_first_input_len(input: &[u8]) -> Option<usize> {
    match input {
        [] => None,
        [ESC, b'[', rest @ ..] => rest
            .iter()
            .position(u8::is_ascii_alphabetic)
            .map(|i| 2 + i + 1)
            .or_else(|| {
                rest.iter()
                    .position(|&b| b == b'M' || b == b'm')
                    .map(|i| 2 + i + 1)
            }),
        [ESC] => Some(1),
        [ESC, ..] => Some(1),
        _ => Some(1),
    }
}

fn map_browse_to_hint(key: Option<crate::browse::BrowseKey>) -> Option<HintKey> {
    use crate::browse::BrowseKey;
    match key? {
        BrowseKey::Esc | BrowseKey::Char('q') | BrowseKey::Char('Q') => Some(HintKey::Cancel),
        BrowseKey::Up | BrowseKey::MouseWheelUp => Some(HintKey::ScrollUp { lines: 1 }),
        BrowseKey::Down | BrowseKey::MouseWheelDown => Some(HintKey::ScrollDown { lines: 1 }),
        BrowseKey::Char(c) => c.is_ascii().then_some(HintKey::Byte(c as u8)),
        BrowseKey::Enter | BrowseKey::Left | BrowseKey::Right | BrowseKey::MouseClick { .. } => {
            None
        }
    }
}

/// Which opener `open_entry` will use after peer-detect (`herdr plugin list`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenRoute {
    Browser,
    FileViewer,
    Less,
    Browse,
    NoOp,
}

pub fn route_entry(entry: &HintEntry, herdr_bin: &Path) -> OpenRoute {
    match &entry.target {
        Target::Url(_) => OpenRoute::Browser,
        Target::File { .. } => {
            if detect_file_viewer(herdr_bin) {
                OpenRoute::FileViewer
            } else {
                OpenRoute::Less
            }
        }
        Target::Dir { .. } => OpenRoute::Browse,
        Target::Missing { .. } => OpenRoute::NoOp,
    }
}

pub fn open_preview_file(
    path: &Path,
    cwd: &Path,
    origin_pane_id: Option<&str>,
) -> Result<(), HintError> {
    let herdr_bin = resolve_herdr_bin();
    match classify(&path.to_string_lossy(), cwd) {
        Target::File {
            path, open_spec, ..
        } => {
            if detect_file_viewer(&herdr_bin) {
                open_file_viewer(&open_spec, cwd, origin_pane_id)?;
            } else {
                let line = line_from_open_spec(&open_spec);
                open_less(&path, line, &herdr_bin)?;
            }
        }
        _ => {
            return Err(HintError::Io(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("open_preview_file: not a file: {}", path.display()),
            )));
        }
    }
    Ok(())
}

pub fn open_entry(
    entry: &HintEntry,
    cwd: &Path,
    origin_pane_id: Option<&str>,
) -> Result<(), HintError> {
    let herdr_bin = resolve_herdr_bin();
    match route_entry(entry, &herdr_bin) {
        OpenRoute::Browser => {
            if let Target::Url(url) = &entry.target {
                open_url(url)?;
            }
        }
        OpenRoute::FileViewer => {
            let open_spec = open_spec_for_target(&entry.target);
            open_file_viewer(&open_spec, cwd, origin_pane_id)?;
        }
        OpenRoute::Less => {
            if let Target::File {
                path, open_spec, ..
            } = &entry.target
            {
                let line = line_from_open_spec(open_spec);
                open_less(path, line, &herdr_bin)?;
            }
        }
        OpenRoute::Browse => {
            if let Target::Dir { path, .. } = &entry.target {
                open_browse(path, origin_pane_id, cwd)?;
            }
        }
        OpenRoute::NoOp => {}
    }
    Ok(())
}

fn line_from_open_spec(open_spec: &str) -> Option<u32> {
    let (_, suffix) = open_spec.rsplit_once(':')?;
    suffix
        .split('-')
        .next()
        .and_then(|part| part.parse::<u32>().ok())
        .filter(|line| *line >= 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;

    fn temp_fixture(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "herdr-preview-hint-{}-{}",
            std::process::id(),
            name
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create fixture dir");
        dir
    }

    #[test]
    fn hint_keys_exclude_q_and_are_unique() {
        assert!(!HINT_KEYS.contains('q'));
        let mut chars: Vec<char> = HINT_KEYS.chars().collect();
        chars.sort_unstable();
        chars.dedup();
        assert_eq!(chars.len(), HINT_KEYS.len());
    }

    #[test]
    fn build_entries_assigns_keys_newest_line_first() {
        let root = temp_fixture("newest-keys");
        let cwd = root.join("repo");
        fs::create_dir_all(cwd.join("src")).unwrap();
        fs::write(cwd.join("src/old.rs"), "x\n").unwrap();
        fs::write(cwd.join("src/new.rs"), "y\n").unwrap();

        let text = "src/old.rs\nsrc/new.rs\n";
        let entries = build_entries(text, &cwd);
        let old = entries.iter().find(|e| e.raw == "src/old.rs").unwrap();
        let new = entries.iter().find(|e| e.raw == "src/new.rs").unwrap();
        assert_eq!(new.key, Some('a'));
        assert_eq!(old.key, hint_key_for_index(1));
    }

    #[test]
    fn build_entries_skips_urls_and_assigns_keys() {
        let root = temp_fixture("build");
        let cwd = root.join("repo");
        fs::create_dir_all(cwd.join("src")).unwrap();
        fs::write(cwd.join("src/app.rs"), "fn main() {}\n").unwrap();

        let text = "see src/app.rs and src/missing.rs\nhttps://example.com\n";
        let entries = build_entries(text, &cwd);

        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].key, Some('a'));
        assert_eq!(entries[0].raw, "src/app.rs");
        assert!(matches!(entries[0].target, Target::File { .. }));
        assert_eq!(entries[1].raw, "src/missing.rs");
        assert_eq!(entries[1].key, None);
        assert!(matches!(entries[1].target, Target::Missing { .. }));
        assert!(entries[0].end > entries[0].start);
    }

    #[test]
    fn format_list_emits_tsv_rows() {
        let root = temp_fixture("format");
        let cwd = root.join("repo");
        fs::create_dir_all(cwd.join("src")).unwrap();
        fs::write(cwd.join("src/app.rs"), "fn main() {}\n").unwrap();
        let entries = build_entries("src/app.rs\n", &cwd);
        let list = format_list(&entries);
        assert!(list.contains("\tsrc/app.rs\tfile\t"));
        assert!(list.starts_with("a\t"));
    }

    #[test]
    fn entries_tsv_roundtrip() {
        let root = temp_fixture("roundtrip");
        let cwd = root.join("repo");
        fs::create_dir_all(cwd.join("docs")).unwrap();
        let entries = build_entries("docs/\n", &cwd);
        let tsv = serialize_entries(&entries);
        let parsed = parse_entries_tsv(&tsv).expect("parse");
        assert_eq!(parsed.len(), entries.len());
        assert_eq!(parsed[0].key, entries[0].key);
        assert_eq!(parsed[0].raw, entries[0].raw);
    }

    #[test]
    fn route_entry_picks_backend_from_target_and_peer_detect() {
        let root = temp_fixture("route");
        let herdr = root.join("herdr");
        fs::write(&herdr, "#!/bin/bash\nexit 0\n").unwrap();
        fs::set_permissions(&herdr, fs::Permissions::from_mode(0o755)).unwrap();

        let file = HintEntry {
            key: Some('a'),
            start: 0,
            end: 4,
            raw: "a.rs".into(),
            target: Target::File {
                path: PathBuf::from("a.rs"),
                open_spec: "a.rs".into(),
                ambiguous: false,
            },
        };
        let dir = HintEntry {
            key: Some('s'),
            start: 0,
            end: 5,
            raw: "docs/".into(),
            target: Target::Dir {
                path: PathBuf::from("docs"),
                open_spec: "docs/".into(),
                ambiguous: false,
            },
        };
        let url = HintEntry {
            key: Some('d'),
            start: 0,
            end: 9,
            raw: "https://x".into(),
            target: Target::Url("https://x".into()),
        };

        assert_eq!(route_entry(&url, &herdr), OpenRoute::Browser);
        // herdr stub exits 0 on plugin list → no FV line → less / browse
        assert_eq!(route_entry(&file, &herdr), OpenRoute::Less);
        assert_eq!(route_entry(&dir, &herdr), OpenRoute::Browse);
    }

    #[test]
    fn line_from_open_spec_parses_suffix() {
        assert_eq!(line_from_open_spec("src/a.rs:42"), Some(42));
        assert_eq!(line_from_open_spec("src/a.rs:10-20"), Some(10));
        assert_eq!(line_from_open_spec("src/a.rs"), None);
    }

    #[test]
    fn overlay_paint_cols_matches_stty_width() {
        assert_eq!(overlay_paint_cols(120), 120);
        assert_eq!(overlay_paint_cols(1), 20);
    }

    #[test]
    fn overlay_layout_reserves_border_status_and_content() {
        let l = overlay_layout(24);
        assert_eq!(l.top_border, 1);
        assert_eq!(l.content_top, 2);
        assert_eq!(l.content_bottom, 22);
        assert_eq!(l.content_slots, 21);
        assert_eq!(l.status_row, 23);
    }

    #[test]
    fn trim_trailing_empty_lines_drops_tail_blanks() {
        let mut lines = vec!["a".into(), "".into(), "".into()];
        trim_trailing_empty_lines(&mut lines);
        assert_eq!(lines, vec!["a".to_string()]);
    }

    #[test]
    fn snapshot_line_window_bottom_and_scroll_up() {
        assert_eq!(snapshot_line_window(200, 40, 0), (160, 200));
        assert_eq!(snapshot_line_window(200, 40, 160), (0, 40));
        assert_eq!(snapshot_line_window(10, 40, 0), (0, 10));
    }

    #[test]
    fn wrap_line_preserves_ansi_and_splits_on_cell_width() {
        let line = format!("\x1b[31m{}\x1b[0m", "a".repeat(10));
        let wrapped = wrap_line_preserve_ansi(&line, 4);
        assert_eq!(wrapped.len(), 3);
        assert!(wrapped[0].contains("\x1b[31m"));
        let visible_a: usize = wrapped
            .iter()
            .map(|r| r.chars().filter(|c| *c == 'a').count())
            .sum();
        assert_eq!(visible_a, 10);
    }
}
