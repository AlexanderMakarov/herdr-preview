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
    classify, classify_with_fallbacks, discover_worktree_roots, is_worktree_dir, Target,
};
use crate::herdr_ipc::{notify, read_focused_snapshot, HerdrIpcError, PaneSnapshot};
use crate::open::{detect_file_viewer, open_browse, open_file_viewer, open_less, open_url};
use crate::tokenize::{find_candidates, Span};

/// Home-row-first keys; excludes `q` (cancel) and visually ambiguous letters per quicklook.
pub const HINT_KEYS: &str = "asdfghjklwertyuiopzxcvbnm";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HintEntry {
    pub key: char,
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
    let mut unique: Vec<Span> = Vec::new();
    let mut seen_raw = Vec::new();
    for span in find_candidates(text) {
        if seen_raw.iter().any(|existing| existing == &span.raw) {
            continue;
        }
        seen_raw.push(span.raw.clone());
        unique.push(span);
    }

    // Classify everything against the pane cwd first so later-visible worktree
    // dirs can rescue earlier missing relative paths in a second pass.
    let primary: Vec<Target> = unique.iter().map(|span| classify(&span.raw, cwd)).collect();
    let mut fallbacks: Vec<PathBuf> = Vec::new();
    for target in &primary {
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

    let mut entries = Vec::new();
    for (span, primary_target) in unique.into_iter().zip(primary) {
        let target = if matches!(primary_target, Target::Missing { .. }) && !fallbacks.is_empty() {
            classify_with_fallbacks(&span.raw, cwd, &fallbacks)
        } else {
            primary_target
        };
        // MVP overlay is filesystem paths only; http(s) stays Ctrl+click → browser.
        if matches!(target, Target::Missing { .. } | Target::Url(_)) {
            continue;
        }
        if let Some(key) = hint_key_for_index(entries.len()) {
            entries.push(HintEntry {
                key,
                start: span.start,
                end: span.end,
                raw: span.raw,
                target,
            });
        } else {
            break;
        }
    }

    entries
}

pub fn hint_key_for_index(index: usize) -> Option<char> {
    HINT_KEYS.chars().nth(index)
}

pub fn target_kind_label(target: &Target) -> &'static str {
    match target {
        Target::File { .. } => "file",
        Target::Dir { .. } => "dir",
        Target::Url(_) => "url",
        Target::Missing { .. } => "missing",
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
    let dir = std::env::temp_dir().join(format!(
        "herdr-preview-hint-{}",
        std::process::id()
    ));
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
            entry.key, entry.start, entry.end, entry.raw
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
            return Err(HintError::OverlayEnv(format!(
                "bad targets line: {line}"
            )));
        }
        let key = parts[0]
            .chars()
            .next()
            .ok_or_else(|| HintError::OverlayEnv("empty key".into()))?;
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
            "file" => Target::File {
                path: PathBuf::from(path),
                open_spec,
            },
            "dir" => Target::Dir {
                path: PathBuf::from(path),
                open_spec,
            },
            "url" => Target::Url(open_spec),
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
        OverlayChoice::Pick(index) => {
            open_entry(&entries[index], &cwd, origin_pane.as_deref())
        }
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
    let (rows, cols) = tty_size(tty.as_raw_fd()).unwrap_or((24, 80));
    write!(stdout, "{ANSI_HOME}{ANSI_CLEAR}")?;
    render_overlay(&mut stdout, entries, snapshot, rows, cols)?;
    stdout.flush()?;

    loop {
        match read_key(&mut tty)? {
            KeyPress::Cancel => return Ok(OverlayChoice::Cancel),
            KeyPress::Ignore => continue,
            KeyPress::Byte(key) => {
                if matches!(key, b'q' | b'Q') {
                    return Ok(OverlayChoice::Cancel);
                }
                if let Some(index) = entries.iter().position(|entry| entry.key == key as char) {
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
const ANSI_CLR_EOL: &str = "\x1b[K";

/// RAII: restore cursor + wrap when the overlay exits for any reason.
struct TerminalGuard;

impl TerminalGuard {
    fn enter(out: &mut impl Write) -> io::Result<Self> {
        write!(out, "{ANSI_HIDE}{ANSI_WRAP_OFF}")?;
        out.flush()?;
        Ok(Self)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let mut stdout = io::stdout();
        let _ = write!(stdout, "{ANSI_SHOW}{ANSI_WRAP_ON}");
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

enum KeyPress {
    Byte(u8),
    Cancel,
    Ignore,
}

fn render_overlay(
    out: &mut impl Write,
    entries: &[HintEntry],
    snapshot: &str,
    rows: u16,
    cols: u16,
) -> io::Result<()> {
    // Match quicklook hint-pane geometry:
    // - Prefer bottom-align when the snap is shorter than the overlay (origin panes
    //   often report viewport_rows = N but visible text has N-1 lines; top-align
    //   floats content one row high).
    // - When the snap is taller, keep the bottom rows (border/resize mismatch).
    // - Legend overwrites the last physical row (no extra newline that scrolls).
    const DIM: &str = "\x1b[2;90m";
    const RESET: &str = "\x1b[0m";
    const H_KEY: &str = "\x1b[0;1;30;48;2;255;253;1m";
    const H_TOK: &str = "\x1b[0;38;2;255;253;1m";

    let rows = rows.max(2) as usize;
    let cols = cols.max(20) as usize;

    // Preserve empty lines the way bash `while read` does (not str::lines(),
    // which drops a final empty after a trailing newline equally in both, but
    // keep middle blanks explicitly).
    let snap_lines = split_snapshot_lines(snapshot);
    let total = snap_lines.len();
    let offset = total.saturating_sub(rows);
    let visible = total - offset;
    let pad = rows.saturating_sub(visible);

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

    let mut body_lines: Vec<String> = Vec::with_capacity(rows);
    for _ in 0..pad {
        body_lines.push(String::new());
    }
    for line in snap_lines.iter().skip(offset) {
        body_lines.push(line.clone());
    }
    debug_assert_eq!(body_lines.len(), rows);

    let mut by_line: Vec<Vec<&HintEntry>> = vec![Vec::new(); rows];
    for entry in entries {
        let Some(abs) = line_starts
            .windows(2)
            .position(|w| entry.start >= w[0] && entry.start < w[1])
        else {
            continue;
        };
        if abs < offset {
            continue;
        }
        let painted = pad + (abs - offset);
        if painted < rows {
            by_line[painted].push(entry);
        }
    }

    for (row, entries_here) in by_line.iter().enumerate() {
        if entries_here.is_empty() {
            continue;
        }
        let line = &mut body_lines[row];
        let mut ordered = entries_here.clone();
        ordered.sort_by_key(|e| std::cmp::Reverse(e.start));
        for entry in ordered {
            if let Some(found) = line.find(&entry.raw) {
                let styled = style_token(entry.key, &entry.raw, H_KEY, H_TOK, RESET, DIM);
                line.replace_range(found..found + entry.raw.len(), &styled);
            }
        }
    }

    // Paint rows-1 body lines with writeln, then legend without a trailing
    // newline (a final newline alone scrolls a full-height frame up by one).
    write!(out, "{ANSI_HOME}")?;
    for line in body_lines.iter().take(rows.saturating_sub(1)) {
        let truncated = truncate_cells(line, cols);
        writeln!(out, "{DIM}{truncated}{RESET}{ANSI_CLR_EOL}")?;
    }
    let legend = format!(
        " hint · {} path(s) · letter opens · q/Esc cancel",
        entries.len()
    );
    let legend = truncate_cells(&legend, cols);
    write!(
        out,
        "\x1b[1;30;48;2;255;253;1m{legend}{RESET}{ANSI_CLR_EOL}"
    )?;
    Ok(())
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
    fs::OpenOptions::new().read(true).write(true).open("/dev/tty")
}

fn read_key(tty: &mut fs::File) -> io::Result<KeyPress> {
    let mut buf = [0u8; 1];
    tty.read_exact(&mut buf)?;
    if buf[0] != ESC {
        return Ok(KeyPress::Byte(buf[0]));
    }

    // Bare Esc cancels. CSI/SS3 sequences (arrows, mouse) must not block —
    // poll briefly then drain, matching quicklook `read -rsn1 -t 0.05`.
    let fd = tty.as_raw_fd();
    let mut pollfd = libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    };
    let ready = unsafe { libc::poll(&mut pollfd, 1, 50) };
    if ready == 0 {
        return Ok(KeyPress::Cancel);
    }
    if ready < 0 {
        return Err(io::Error::last_os_error());
    }
    let mut extra = [0u8; 32];
    let _ = tty.read(&mut extra)?;
    Ok(KeyPress::Ignore)
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
        Target::File { path, open_spec } => {
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
            if let Target::File { path, open_spec } = &entry.target {
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
    fn build_entries_skips_missing_urls_and_assigns_keys() {
        let root = temp_fixture("build");
        let cwd = root.join("repo");
        fs::create_dir_all(cwd.join("src")).unwrap();
        fs::write(cwd.join("src/app.rs"), "fn main() {}\n").unwrap();

        let text = "see src/app.rs and src/missing.rs\nhttps://example.com\n";
        let entries = build_entries(text, &cwd);

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].key, 'a');
        assert_eq!(entries[0].raw, "src/app.rs");
        assert!(matches!(entries[0].target, Target::File { .. }));
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
            key: 'a',
            start: 0,
            end: 4,
            raw: "a.rs".into(),
            target: Target::File {
                path: PathBuf::from("a.rs"),
                open_spec: "a.rs".into(),
            },
        };
        let dir = HintEntry {
            key: 's',
            start: 0,
            end: 5,
            raw: "docs/".into(),
            target: Target::Dir {
                path: PathBuf::from("docs"),
                open_spec: "docs/".into(),
            },
        };
        let url = HintEntry {
            key: 'd',
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
}
