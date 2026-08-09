use std::path::{Component, Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    File { path: PathBuf, open_spec: String },
    Dir { path: PathBuf, open_spec: String },
    Url(String),
    Missing { display: String },
}

pub fn classify(raw: &str, cwd: &Path) -> Target {
    let raw = raw.trim();

    if raw.starts_with("http://") || raw.starts_with("https://") {
        return Target::Url(raw.to_string());
    }

    let path_raw = strip_file_url(raw);
    let (path_part, line_suffix) = split_line_suffix(path_raw);
    let decoded_path = expand_tilde(&percent_decode(path_part));
    let open_spec = build_open_spec(&decoded_path, line_suffix.as_deref());
    let resolved = resolve_path(&decoded_path, cwd);

    if !resolved.exists() {
        return Target::Missing {
            display: raw.to_string(),
        };
    }

    if resolved.is_dir() {
        Target::Dir {
            path: resolved,
            open_spec,
        }
    } else {
        Target::File {
            path: resolved,
            open_spec,
        }
    }
}

/// Like [`classify`], but if `cwd` misses, try each fallback root (e.g. a
/// `.claude/worktrees/…` directory visible in the same pane).
///
/// When a fallback hits, `open_spec` is rewritten relative to `cwd` when the
/// resolved path is under that tree so FV rooted at the agent pane still opens
/// it (e.g. `.claude/worktrees/feat/…/file.md`).
pub fn classify_with_fallbacks(raw: &str, cwd: &Path, fallbacks: &[PathBuf]) -> Target {
    let primary = classify(raw, cwd);
    if !matches!(primary, Target::Missing { .. }) {
        return primary;
    }

    for root in fallbacks {
        if root.as_path() == cwd {
            continue;
        }
        match classify(raw, root) {
            Target::File { path, .. } => {
                return Target::File {
                    open_spec: open_spec_under_base(raw, &path, cwd),
                    path,
                };
            }
            Target::Dir { path, .. } => {
                return Target::Dir {
                    open_spec: open_spec_under_base(raw, &path, cwd),
                    path,
                };
            }
            Target::Url(_) | Target::Missing { .. } => continue,
        }
    }

    primary
}

/// True when `path` looks like a git/claude worktree directory (`…/worktrees/…`).
pub fn is_worktree_dir(path: &Path) -> bool {
    path.components()
        .any(|component| component.as_os_str() == "worktrees")
}

/// Extra roots to try when a relative path misses under `cwd`.
///
/// Includes on-disk git worktrees for this repo (via `git worktree list`) and
/// any immediate children of `.claude/worktrees/`. Callers should prepend any
/// worktree dirs observed in the visible snapshot so those win when several
/// worktrees contain the same relative path.
pub fn discover_worktree_roots(cwd: &Path) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    let cwd_canon = cwd.canonicalize().unwrap_or_else(|_| cwd.to_path_buf());

    if let Some(from_git) = git_worktree_paths(cwd) {
        for path in from_git {
            let canon = path.canonicalize().unwrap_or(path);
            if canon != cwd_canon && canon.is_dir() && !roots.iter().any(|r| r == &canon) {
                roots.push(canon);
            }
        }
    }

    let claude_root = cwd.join(".claude/worktrees");
    if let Ok(entries) = std::fs::read_dir(&claude_root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let canon = path.canonicalize().unwrap_or(path);
            if canon != cwd_canon && !roots.iter().any(|r| r == &canon) {
                roots.push(canon);
            }
        }
    }

    roots
}

fn git_worktree_paths(cwd: &Path) -> Option<Vec<PathBuf>> {
    let output = Command::new("git")
        .args(["worktree", "list", "--porcelain"])
        .current_dir(cwd)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let mut paths = Vec::new();
    for line in text.lines() {
        if let Some(path) = line.strip_prefix("worktree ") {
            if !path.is_empty() {
                paths.push(PathBuf::from(path));
            }
        }
    }
    Some(paths)
}

fn open_spec_under_base(raw: &str, resolved: &Path, base: &Path) -> String {
    let path_raw = strip_file_url(raw.trim());
    let (_, line_suffix) = split_line_suffix(path_raw);
    if let Ok(rel) = resolved.strip_prefix(base) {
        return build_open_spec(&rel.to_string_lossy(), line_suffix.as_deref());
    }
    build_open_spec(&resolved.to_string_lossy(), line_suffix.as_deref())
}

fn strip_file_url(raw: &str) -> &str {
    raw.strip_prefix("file://").unwrap_or(raw)
}

fn split_line_suffix(raw: &str) -> (&str, Option<String>) {
    let Some((path, suffix)) = raw.rsplit_once(':') else {
        return (raw, None);
    };

    if path.is_empty() {
        return (raw, None);
    }

    if parse_line_suffix(suffix).is_some() {
        (path, Some(format!(":{suffix}")))
    } else {
        (raw, None)
    }
}

fn parse_line_suffix(suffix: &str) -> Option<(usize, Option<usize>)> {
    if let Ok(number) = suffix.parse::<usize>() {
        return (number >= 1).then_some((number, None));
    }

    let (start, end) = suffix.split_once('-')?;
    let start = start.parse::<usize>().ok()?;
    let end = end.parse::<usize>().ok()?;
    if start >= 1 && end >= 1 {
        if start == end {
            Some((start, None))
        } else {
            Some((start, Some(end)))
        }
    } else {
        None
    }
}

fn build_open_spec(path: &str, line_suffix: Option<&str>) -> String {
    match line_suffix {
        Some(suffix) => format!("{path}{suffix}"),
        None => path.to_string(),
    }
}

fn percent_decode(input: &str) -> String {
    let mut out = Vec::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut index = 0;

    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            if let Ok(byte) = u8::from_str_radix(&input[index + 1..index + 3], 16) {
                out.push(byte);
                index += 3;
                continue;
            }
        }
        out.push(bytes[index]);
        index += 1;
    }

    String::from_utf8_lossy(&out).into_owned()
}

fn expand_tilde(path: &str) -> String {
    if path == "~" {
        return home_dir().unwrap_or_else(|| path.to_string());
    }
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = home_dir() {
            return format!(
                "{home}{}{rest}",
                if home.ends_with('/') { "" } else { "/" }
            );
        }
    }
    path.to_string()
}

fn home_dir() -> Option<String> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(|value| value.to_string_lossy().into_owned())
        .filter(|value| !value.is_empty())
}

fn resolve_path(path: &str, cwd: &Path) -> PathBuf {
    let candidate = Path::new(path);
    let joined = if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        cwd.join(candidate)
    };
    normalize_path(&joined)
}

fn normalize_path(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();

    for component in path.components() {
        match component {
            Component::Prefix(_) | Component::RootDir | Component::Normal(_) => {
                out.push(component.as_os_str());
            }
            Component::CurDir => {}
            Component::ParentDir => match out.components().next_back() {
                Some(Component::Normal(_)) => {
                    out.pop();
                }
                _ => {
                    out.push(component.as_os_str());
                }
            },
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_line_suffix_keeps_non_numeric_colon_on_path() {
        assert_eq!(
            split_line_suffix("C:\\dev\\note.txt"),
            ("C:\\dev\\note.txt", None)
        );
    }

    #[test]
    fn percent_decode_replaces_encoded_space() {
        assert_eq!(percent_decode("Tray%20status"), "Tray status");
    }
}
