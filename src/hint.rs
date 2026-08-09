//! Hint overlay: snapshot → tokenize → classify → key assignment → overlay pick.
//!
//! CRITICAL: `read_focused_snapshot()` runs in the **action** process before spawning
//! the overlay pane. The overlay must never call `herdr pane read` (deadlock).

use std::fmt;
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::classify::{classify, Target};
use crate::herdr_ipc::{read_focused_snapshot, HerdrIpcError, PaneSnapshot};
use crate::open::{detect_file_viewer, open_file_viewer, open_less, open_url};
use crate::tokenize::{find_candidates, Span};

/// Home-row-first keys; excludes `q` (cancel) and visually ambiguous letters per quicklook.
pub const HINT_KEYS: &str = "asdfghjklwertyuiopzxcvbnm";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HintEntry {
    pub key: char,
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
    let mut entries = Vec::new();
    let mut seen = Vec::new();

    for Span { raw, .. } in find_candidates(text) {
        if seen.iter().any(|existing| existing == &raw) {
            continue;
        }
        let target = classify(&raw, cwd);
        if matches!(target, Target::Missing { .. }) {
            continue;
        }
        seen.push(raw.clone());
        if let Some(key) = hint_key_for_index(entries.len()) {
            entries.push(HintEntry { key, raw, target });
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
            "{}\t{}\t{kind}\t{open_spec}\t{path}\n",
            entry.key, entry.raw
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
        let parts: Vec<&str> = line.splitn(5, '\t').collect();
        if parts.len() < 5 {
            return Err(HintError::OverlayEnv(format!(
                "bad targets line: {line}"
            )));
        }
        let key = parts[0]
            .chars()
            .next()
            .ok_or_else(|| HintError::OverlayEnv("empty key".into()))?;
        let raw = parts[1].to_string();
        let kind = parts[2];
        let open_spec = parts[3].to_string();
        let path = parts[4];
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
        entries.push(HintEntry { key, raw, target });
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

    let entries = parse_entries_tsv(&fs::read_to_string(&targets_path)?)?;
    let snapshot = fs::read_to_string(&snap_path)?;

    let choice = overlay_pick(&entries, &snapshot)?;
    match choice {
        OverlayChoice::Cancel => Ok(()),
        OverlayChoice::Pick(index) => open_entry(&entries[index], &cwd),
    }
}

enum OverlayChoice {
    Cancel,
    Pick(usize),
}

fn overlay_pick(entries: &[HintEntry], snapshot: &str) -> Result<OverlayChoice, HintError> {
    let mut stdout = io::stdout();
    write!(stdout, "{ANSI_HOME}{ANSI_HIDE}")?;
    render_overlay(&mut stdout, entries, snapshot)?;
    stdout.flush()?;

    let mut tty = open_tty()?;
    loop {
        let key = read_key(&mut tty)?;
        if matches!(key, b'q' | b'Q' | ESC) {
            write!(stdout, "{ANSI_SHOW}")?;
            stdout.flush()?;
            return Ok(OverlayChoice::Cancel);
        }
        if let Some(index) = entries.iter().position(|entry| entry.key == key as char) {
            write!(stdout, "{ANSI_SHOW}")?;
            stdout.flush()?;
            return Ok(OverlayChoice::Pick(index));
        }
    }
}

const ESC: u8 = 0x1b;
const ANSI_HOME: &str = "\x1b[H\x1b[J";
const ANSI_HIDE: &str = "\x1b[?25l";
const ANSI_SHOW: &str = "\x1b[?25h";

fn render_overlay(out: &mut impl Write, entries: &[HintEntry], snapshot: &str) -> io::Result<()> {
    writeln!(out, "\x1b[2;90mherdr-preview hint\x1b[0m  \x1b[90mq/Esc cancel\x1b[0m")?;
    writeln!(out)?;
    for line in snapshot.lines().take(12) {
        writeln!(out, "\x1b[2;90m{line}\x1b[0m")?;
    }
    if snapshot.lines().count() > 12 {
        writeln!(out, "\x1b[2;90m…\x1b[0m")?;
    }
    writeln!(out)?;
    for entry in entries {
        let kind = target_kind_label(&entry.target);
        writeln!(
            out,
            "\x1b[1;30;48;2;255;253;1m {}\x1b[0m  \x1b[38;2;255;253;1m{}\x1b[0m  \x1b[90m({kind})\x1b[0m",
            entry.key, entry.raw
        )?;
    }
    Ok(())
}

fn open_tty() -> io::Result<fs::File> {
    fs::OpenOptions::new().read(true).write(true).open("/dev/tty")
}

fn read_key(tty: &mut fs::File) -> io::Result<u8> {
    let mut buf = [0u8; 1];
    tty.read_exact(&mut buf)?;
    if buf[0] == ESC {
        let mut extra = [0u8; 2];
        if tty.read(&mut extra)? > 0 {
            // swallow simple escape sequences (arrow keys, etc.)
        }
    }
    Ok(buf[0])
}

/// Which opener `open_entry` will use after peer-detect (`herdr plugin list`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenRoute {
    Browser,
    FileViewer,
    Less,
    DirSkip,
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
        Target::Dir { .. } => {
            if detect_file_viewer(herdr_bin) {
                OpenRoute::FileViewer
            } else {
                OpenRoute::DirSkip
            }
        }
        Target::Missing { .. } => OpenRoute::NoOp,
    }
}

pub const DIR_SKIP_NOTICE: &str =
    "herdr-preview: directories need herdr-file-viewer (less is file-only)";

pub fn open_entry(entry: &HintEntry, cwd: &Path) -> Result<(), HintError> {
    let herdr_bin = resolve_herdr_bin();
    match route_entry(entry, &herdr_bin) {
        OpenRoute::Browser => {
            if let Target::Url(url) = &entry.target {
                open_url(url)?;
            }
        }
        OpenRoute::FileViewer => {
            let open_spec = open_spec_for_target(&entry.target);
            open_file_viewer(&open_spec, cwd)?;
        }
        OpenRoute::Less => {
            if let Target::File { path, open_spec } = &entry.target {
                let line = line_from_open_spec(open_spec);
                open_less(path, line, &herdr_bin)?;
            }
        }
        OpenRoute::DirSkip => {
            eprintln!("{DIR_SKIP_NOTICE}");
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
    fn build_entries_skips_missing_and_assigns_keys() {
        let root = temp_fixture("build");
        let cwd = root.join("repo");
        fs::create_dir_all(cwd.join("src")).unwrap();
        fs::write(cwd.join("src/app.rs"), "fn main() {}\n").unwrap();

        let text = "see src/app.rs and src/missing.rs\nhttps://example.com\n";
        let entries = build_entries(text, &cwd);

        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].key, 'a');
        assert_eq!(entries[0].raw, "src/app.rs");
        assert!(matches!(entries[0].target, Target::File { .. }));
        assert_eq!(entries[1].key, 's');
        assert_eq!(entries[1].raw, "https://example.com");
        assert!(matches!(entries[1].target, Target::Url(_)));
    }

    #[test]
    fn format_list_emits_tsv_rows() {
        let root = temp_fixture("format");
        let cwd = root.join("repo");
        fs::create_dir_all(&cwd).unwrap();
        let entries = build_entries("https://example.com\n", &cwd);
        let list = format_list(&entries);
        assert!(list.starts_with(
            "a\thttps://example.com\turl\thttps://example.com\thttps://example.com\n"
        ));
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
            raw: "a.rs".into(),
            target: Target::File {
                path: PathBuf::from("a.rs"),
                open_spec: "a.rs".into(),
            },
        };
        let dir = HintEntry {
            key: 's',
            raw: "docs/".into(),
            target: Target::Dir {
                path: PathBuf::from("docs"),
                open_spec: "docs/".into(),
            },
        };
        let url = HintEntry {
            key: 'd',
            raw: "https://x".into(),
            target: Target::Url("https://x".into()),
        };

        assert_eq!(route_entry(&url, &herdr), OpenRoute::Browser);
        // herdr stub exits 0 on plugin list → no FV line → less / dir skip
        assert_eq!(route_entry(&file, &herdr), OpenRoute::Less);
        assert_eq!(route_entry(&dir, &herdr), OpenRoute::DirSkip);
    }

    #[test]
    fn line_from_open_spec_parses_suffix() {
        assert_eq!(line_from_open_spec("src/a.rs:42"), Some(42));
        assert_eq!(line_from_open_spec("src/a.rs:10-20"), Some(10));
        assert_eq!(line_from_open_spec("src/a.rs"), None);
    }
}
