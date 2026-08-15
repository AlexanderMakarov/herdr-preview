use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenBackend {
    FileViewer,
    Less,
}

pub fn detect_file_viewer(herdr_bin: &Path) -> bool {
    let output = match Command::new(herdr_bin).args(["plugin", "list"]).output() {
        Ok(output) => output,
        Err(_) => return false,
    };

    if !output.status.success() {
        return false;
    }

    let text = String::from_utf8_lossy(&output.stdout);
    text.lines().any(plugin_list_line_is_file_viewer)
}

fn plugin_list_line_is_file_viewer(line: &str) -> bool {
    let trimmed = line.trim_start();
    // Plain `herdr plugin list` lines look like:
    //   - herdr-file-viewer (herdr-file-viewer) enabled
    let body = trimmed
        .strip_prefix('-')
        .map(str::trim_start)
        .unwrap_or(trimmed);
    body.split_whitespace()
        .next()
        .is_some_and(|name| name == "herdr-file-viewer")
}

fn resolve_herdr_bin() -> PathBuf {
    std::env::var_os("HERDR_BIN_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("herdr"))
}

fn browser_opener() -> &'static str {
    if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    }
}

fn less_available() -> bool {
    Command::new("less")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

pub fn open_url(url: &str) -> io::Result<()> {
    let status = Command::new(browser_opener())
        .arg(url)
        .status()
        .map_err(|err| {
            if err.kind() == io::ErrorKind::NotFound {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("{} not found on PATH", browser_opener()),
                )
            } else {
                err
            }
        })?;

    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "{} exited with {}",
            browser_opener(),
            status
        )))
    }
}

/// Split `path`, `path:N`, or `path:A-B` into (path, optional `:suffix`).
fn split_open_spec(open_spec: &str) -> (&str, &str) {
    if let Some((path, suffix)) = open_spec.rsplit_once(':') {
        let ok = if let Some((a, b)) = suffix.split_once('-') {
            !a.is_empty()
                && !b.is_empty()
                && a.chars().all(|c| c.is_ascii_digit())
                && b.chars().all(|c| c.is_ascii_digit())
        } else {
            !suffix.is_empty() && suffix.chars().all(|c| c.is_ascii_digit())
        };
        if ok {
            return (path, &open_spec[path.len()..]);
        }
    }
    (open_spec, "")
}

fn git_toplevel(start: &Path) -> Option<PathBuf> {
    let dir = if start.is_dir() {
        start
    } else {
        start.parent()?
    };
    let output = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(dir)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if text.is_empty() {
        None
    } else {
        Some(PathBuf::from(text))
    }
}

fn file_viewer_plugin_root(herdr_bin: &Path) -> io::Result<PathBuf> {
    let output = Command::new(herdr_bin)
        .args(["plugin", "list", "--json"])
        .output()
        .map_err(|err| {
            if err.kind() == io::ErrorKind::NotFound {
                io::Error::new(io::ErrorKind::NotFound, "herdr not found on PATH")
            } else {
                err
            }
        })?;
    if !output.status.success() {
        return Err(io::Error::other(format!(
            "herdr plugin list --json exited with {}",
            output.status
        )));
    }
    let value: Value = serde_json::from_slice(&output.stdout)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    let plugins = value
        .pointer("/result/plugins")
        .and_then(|v| v.as_array())
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "herdr plugin list --json missing result.plugins",
            )
        })?;
    for plugin in plugins {
        let id = plugin.get("plugin_id").and_then(|v| v.as_str()).unwrap_or("");
        if id == "herdr-file-viewer" {
            if let Some(root) = plugin.get("plugin_root").and_then(|v| v.as_str()) {
                if !root.is_empty() {
                    return Ok(PathBuf::from(root));
                }
            }
        }
    }
    Err(io::Error::new(
        io::ErrorKind::NotFound,
        "herdr-file-viewer plugin_root not found",
    ))
}

fn file_viewer_bin(herdr_bin: &Path) -> io::Result<PathBuf> {
    let root = file_viewer_plugin_root(herdr_bin)?;
    let bin = root.join("target/release/herdr-file-viewer");
    if bin.is_file() {
        Ok(bin)
    } else {
        Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("file-viewer binary missing at {}", bin.display()),
        ))
    }
}

fn parse_tab_create_pane_id(stdout: &[u8]) -> io::Result<String> {
    let value: Value = serde_json::from_slice(stdout)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    value
        .pointer("/result/root_pane/pane_id")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "tab create response missing result.root_pane.pane_id",
            )
        })
}

fn focus_pane(herdr_bin: &Path, pane_id: &str) -> io::Result<()> {
    // Herdr has no focus-by-id; FV's launcher uses zoom --on/--off to focus
    // without leaving the pane maximized.
    for arg in ["--on", "--off"] {
        let status = Command::new(herdr_bin)
            .args(["pane", "zoom", pane_id, arg])
            .status()
            .map_err(|err| {
                if err.kind() == io::ErrorKind::NotFound {
                    io::Error::new(io::ErrorKind::NotFound, "herdr not found on PATH")
                } else {
                    err
                }
            })?;
        if !status.success() {
            return Err(io::Error::other(format!(
                "herdr pane zoom {pane_id} {arg} exited with {status}"
            )));
        }
    }
    Ok(())
}

/// True when `path` is inside the origin pane's git worktree (or cwd if not a repo).
pub fn is_under_origin_tree(path: &Path, origin_cwd: &Path) -> bool {
    let abs = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let origin_root = git_toplevel(origin_cwd).unwrap_or_else(|| origin_cwd.to_path_buf());
    let origin_root = origin_root.canonicalize().unwrap_or(origin_root);
    abs.starts_with(&origin_root)
}

/// `(viewer_root, OPEN spec, file is under origin pane tree)`.
fn open_target_for_cwd(open_spec: &str, cwd: &Path) -> (PathBuf, String, bool) {
    let (path_part, suffix) = split_open_spec(open_spec);
    let candidate = if Path::new(path_part).is_absolute() {
        PathBuf::from(path_part)
    } else {
        cwd.join(path_part)
    };
    let abs = candidate.canonicalize().unwrap_or(candidate);

    let origin_root = git_toplevel(cwd).unwrap_or_else(|| cwd.to_path_buf());
    let origin_root = origin_root.canonicalize().unwrap_or(origin_root);
    let in_origin_tree = is_under_origin_tree(&abs, cwd);

    // In-tree: OPEN is relative to the origin pane's worktree/cwd so a split
    // viewer rooted there can reveal it. Out-of-tree: root at the file's git
    // toplevel (or its parent) — never send an above-root absolute OPEN.
    let root = if in_origin_tree {
        origin_root
    } else {
        let parent = abs.parent().unwrap_or(abs.as_path());
        git_toplevel(parent)
            .unwrap_or_else(|| parent.to_path_buf())
            .canonicalize()
            .unwrap_or_else(|_| parent.to_path_buf())
    };

    let rel = abs
        .strip_prefix(&root)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| abs.to_string_lossy().into_owned());
    (root, format!("{rel}{suffix}"), in_origin_tree)
}

/// Open a path in herdr-file-viewer.
///
/// When the file is under the origin pane's worktree/cwd and an origin pane id
/// is known:
/// 1. Focus the origin pane via zoom --on/--off (same as FV's own launcher)
/// 2. `plugin pane open --target-pane <origin> --placement split --direction right`
///    with OPEN env (target-pane binds the spawn to that pane's cwd)
///
/// Otherwise (no origin, or the file is outside that tree) use
/// `tab create --cwd <file's tree>` + `pane run <abs-viewer-bin>` so OPEN is
/// under the viewer root. Passing an above-root path to a split on the origin
/// pane makes FV reveal fail and land on the pane cwd tree.
pub fn open_file_viewer(
    open_spec: &str,
    cwd: &Path,
    origin_pane_id: Option<&str>,
) -> io::Result<()> {
    let herdr_bin = resolve_herdr_bin();
    let (root, open_target, in_origin_tree) = open_target_for_cwd(open_spec, cwd);
    let env = format!("HERDR_FILE_VIEWER_OPEN={open_target}");

    if in_origin_tree {
        if let Some(origin) = origin_pane_id.filter(|id| !id.is_empty()) {
            focus_pane(&herdr_bin, origin)?;
            let status = Command::new(&herdr_bin)
                .args([
                    "plugin",
                    "pane",
                    "open",
                    "--plugin",
                    "herdr-file-viewer",
                    "--entrypoint",
                    "file-viewer",
                    "--placement",
                    "split",
                    "--direction",
                    "right",
                    "--target-pane",
                    origin,
                    "--focus",
                    "--env",
                    &env,
                ])
                .status()
                .map_err(|err| {
                    if err.kind() == io::ErrorKind::NotFound {
                        io::Error::new(io::ErrorKind::NotFound, "herdr not found on PATH")
                    } else {
                        err
                    }
                })?;
            return if status.success() {
                Ok(())
            } else {
                Err(io::Error::other(format!(
                    "herdr plugin pane open exited with {status}"
                )))
            };
        }
    }

    open_file_viewer_via_tab(&herdr_bin, &root, &env)
}

fn open_file_viewer_via_tab(herdr_bin: &Path, root: &Path, env: &str) -> io::Result<()> {
    let viewer = file_viewer_bin(herdr_bin)?;
    let create = Command::new(herdr_bin)
        .args([
            "tab",
            "create",
            "--cwd",
            &root.to_string_lossy(),
            "--focus",
            "--env",
            env,
        ])
        .output()
        .map_err(|err| {
            if err.kind() == io::ErrorKind::NotFound {
                io::Error::new(io::ErrorKind::NotFound, "herdr not found on PATH")
            } else {
                err
            }
        })?;
    if !create.status.success() {
        return Err(io::Error::other(format!(
            "herdr tab create exited with {}",
            create.status
        )));
    }
    let pane_id = parse_tab_create_pane_id(&create.stdout)?;

    let status = Command::new(herdr_bin)
        .args(["pane", "run", &pane_id])
        .arg(&viewer)
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
        Err(io::Error::other(format!(
            "herdr pane run exited with {status}"
        )))
    }
}

pub fn open_less(path: &Path, line: Option<u32>, herdr_bin: &Path) -> io::Result<()> {
    if path.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "less cannot open a directory",
        ));
    }

    if !less_available() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "less not found on PATH",
        ));
    }

    let file_env = format!("HERDR_PREVIEW_LESS_FILE={}", path.display());
    let mut cmd = Command::new(herdr_bin);
    cmd.args([
        "plugin",
        "pane",
        "open",
        "--plugin",
        "herdr-preview",
        "--entrypoint",
        "less",
        "--placement",
        "overlay",
        "--focus",
        "--env",
        &file_env,
    ]);

    if let Some(line) = line {
        let line_env = format!("HERDR_PREVIEW_LESS_LINE={line}");
        cmd.args(["--env", &line_env]);
    }

    let status = cmd.status().map_err(|err| {
        if err.kind() == io::ErrorKind::NotFound {
            io::Error::new(io::ErrorKind::NotFound, "herdr not found on PATH")
        } else {
            err
        }
    })?;

    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "herdr plugin pane open exited with {status}"
        )))
    }
}

pub fn open_browse(
    start: &Path,
    origin_pane_id: Option<&str>,
    origin_cwd: &Path,
) -> io::Result<()> {
    let herdr_bin = resolve_herdr_bin();
    let start_env = format!("HERDR_PREVIEW_BROWSE_START={}", start.display());
    let cwd_env = format!("HERDR_PREVIEW_BROWSE_CWD={}", origin_cwd.display());

    let mut cmd = Command::new(&herdr_bin);
    cmd.args([
        "plugin",
        "pane",
        "open",
        "--plugin",
        "herdr-preview",
        "--entrypoint",
        "browse",
        "--placement",
        "overlay",
        "--focus",
        "--env",
        &start_env,
        "--env",
        &cwd_env,
    ]);

    if let Some(origin) = origin_pane_id.filter(|id| !id.is_empty()) {
        let origin_env = format!("HERDR_PREVIEW_BROWSE_ORIGIN={origin}");
        cmd.args(["--env", &origin_env]);
    }

    let status = cmd.status().map_err(|err| {
        if err.kind() == io::ErrorKind::NotFound {
            io::Error::new(io::ErrorKind::NotFound, "herdr not found on PATH")
        } else {
            err
        }
    })?;

    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "herdr plugin pane open exited with {status}"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::{plugin_list_line_is_file_viewer, split_open_spec};

    #[test]
    fn split_open_spec_keeps_line_and_range() {
        assert_eq!(split_open_spec("src/a.rs:42"), ("src/a.rs", ":42"));
        assert_eq!(split_open_spec("src/a.rs:10-20"), ("src/a.rs", ":10-20"));
        assert_eq!(split_open_spec("src/a.rs"), ("src/a.rs", ""));
        assert_eq!(split_open_spec("/tmp/x:note"), ("/tmp/x:note", ""));
    }

    #[test]
    fn plugin_list_detection_accepts_common_list_shapes() {
        assert!(plugin_list_line_is_file_viewer(
            "- herdr-file-viewer (herdr-file-viewer) enabled"
        ));
        assert!(plugin_list_line_is_file_viewer(
            "  - herdr-file-viewer (herdr-file-viewer) enabled"
        ));
        assert!(!plugin_list_line_is_file_viewer(
            "- other-plugin (other) enabled"
        ));
        assert!(!plugin_list_line_is_file_viewer(
            "note: see herdr-file-viewer docs"
        ));
    }
}
