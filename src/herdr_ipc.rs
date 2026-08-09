//! Thin wrappers around the Herdr CLI for focused-pane snapshots.
//!
//! Pinned commands (from Herdr CLI + herdr-quicklook):
//!
//! 1. Prefer `HERDR_PLUGIN_CONTEXT_JSON` `.focused_pane_id` / `.focused_pane_cwd`
//!    when the action is launched via a Herdr keybinding (authoritative for the
//!    pane that owned focus when the key fired). Fall back to:
//!    `herdr pane current` → JSON `.result.pane.pane_id` and `.result.pane.cwd`
//! 2. Visible text only (MVP — never full scrollback / recent):
//!    `herdr pane read <PANE_ID> --source visible --format text`

use std::fmt;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneSnapshot {
    pub cwd: PathBuf,
    pub visible_text: String,
    pub pane_id: String,
}

#[derive(Debug, PartialEq, Eq)]
pub enum HerdrIpcError {
    HerdrNotFound,
    PaneCurrentFailed(String),
    InvalidPaneCurrentJson(String),
    MissingPaneId,
    MissingCwd,
    PaneReadFailed { pane_id: String, detail: String },
}

impl fmt::Display for HerdrIpcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::HerdrNotFound => write!(f, "herdr not found on PATH"),
            Self::PaneCurrentFailed(detail) => write!(f, "herdr pane current failed: {detail}"),
            Self::InvalidPaneCurrentJson(detail) => {
                write!(f, "invalid pane current JSON: {detail}")
            }
            Self::MissingPaneId => write!(f, "pane current JSON missing pane_id"),
            Self::MissingCwd => write!(f, "pane current JSON missing cwd"),
            Self::PaneReadFailed { pane_id, detail } => {
                write!(f, "herdr pane read {pane_id} failed: {detail}")
            }
        }
    }
}

impl std::error::Error for HerdrIpcError {}

fn resolve_herdr_bin() -> PathBuf {
    std::env::var_os("HERDR_BIN_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("herdr"))
}

pub fn read_focused_snapshot() -> Result<PaneSnapshot, HerdrIpcError> {
    let herdr_bin = resolve_herdr_bin();
    read_focused_snapshot_with_bin(&herdr_bin)
}

fn read_focused_snapshot_with_bin(herdr_bin: &Path) -> Result<PaneSnapshot, HerdrIpcError> {
    let (pane_id, cwd) = focused_pane_meta(herdr_bin)?;
    let visible_text = fetch_visible_text(herdr_bin, &pane_id)?;
    Ok(PaneSnapshot {
        cwd,
        visible_text,
        pane_id,
    })
}

fn focused_pane_meta(herdr_bin: &Path) -> Result<(String, PathBuf), HerdrIpcError> {
    if let Some(meta) = parse_plugin_context_meta(std::env::var_os("HERDR_PLUGIN_CONTEXT_JSON")) {
        return Ok(meta);
    }
    fetch_pane_current_meta(herdr_bin)
}

fn parse_plugin_context_meta(raw: Option<std::ffi::OsString>) -> Option<(String, PathBuf)> {
    let raw = raw?;
    let text = raw.to_str()?;
    let value: serde_json::Value = serde_json::from_str(text.trim()).ok()?;
    let pane_id = value
        .get("focused_pane_id")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())?
        .to_string();
    let cwd = value
        .get("focused_pane_cwd")
        .or_else(|| value.get("workspace_cwd"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())?;
    Some((pane_id, PathBuf::from(cwd)))
}

fn fetch_pane_current_meta(herdr_bin: &Path) -> Result<(String, PathBuf), HerdrIpcError> {
    let output = Command::new(herdr_bin)
        .args(["pane", "current"])
        .output()
        .map_err(|err| {
            if err.kind() == std::io::ErrorKind::NotFound {
                HerdrIpcError::HerdrNotFound
            } else {
                HerdrIpcError::PaneCurrentFailed(err.to_string())
            }
        })?;

    if !output.status.success() {
        return Err(HerdrIpcError::PaneCurrentFailed(format!(
            "exited with {}",
            output.status
        )));
    }

    let json = String::from_utf8_lossy(&output.stdout);
    parse_pane_current(&json)
}

fn parse_pane_current(json: &str) -> Result<(String, PathBuf), HerdrIpcError> {
    let value: serde_json::Value = serde_json::from_str(json.trim())
        .map_err(|err| HerdrIpcError::InvalidPaneCurrentJson(format!("{err}: {}", json.trim())))?;

    let pane = value
        .pointer("/result/pane")
        .ok_or(HerdrIpcError::InvalidPaneCurrentJson(
            "missing .result.pane".to_string(),
        ))?;

    let pane_id = pane
        .get("pane_id")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .ok_or(HerdrIpcError::MissingPaneId)?
        .to_string();

    let cwd = pane
        .get("cwd")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .ok_or(HerdrIpcError::MissingCwd)?;

    Ok((pane_id, PathBuf::from(cwd)))
}

fn fetch_visible_text(herdr_bin: &Path, pane_id: &str) -> Result<String, HerdrIpcError> {
    let output = Command::new(herdr_bin)
        .args(["pane", "read", pane_id, "--source", "visible", "--format", "text"])
        .output()
        .map_err(|err| {
            if err.kind() == std::io::ErrorKind::NotFound {
                HerdrIpcError::HerdrNotFound
            } else {
                HerdrIpcError::PaneReadFailed {
                    pane_id: pane_id.to_string(),
                    detail: err.to_string(),
                }
            }
        })?;

    if !output.status.success() {
        return Err(HerdrIpcError::PaneReadFailed {
            pane_id: pane_id.to_string(),
            detail: format!("exited with {}", output.status),
        });
    }

    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Best-effort toast; never fails the caller.
pub fn notify(title: &str, body: &str) {
    let herdr_bin = resolve_herdr_bin();
    let _ = Command::new(herdr_bin)
        .args([
            "notification",
            "show",
            title,
            "--body",
            body,
            "--sound",
            "none",
        ])
        .status();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_plugin_context_prefers_focused_pane_fields() {
        let raw = r#"{
          "focused_pane_id": "w1:p2",
          "focused_pane_cwd": "/tmp/agent-repo",
          "workspace_cwd": "/tmp/workspace"
        }"#;
        let (id, cwd) =
            parse_plugin_context_meta(Some(std::ffi::OsString::from(raw))).expect("parse");
        assert_eq!(id, "w1:p2");
        assert_eq!(cwd, PathBuf::from("/tmp/agent-repo"));
    }
}
