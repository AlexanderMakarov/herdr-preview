use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    File {
        path: PathBuf,
        open_spec: String,
        ambiguous: bool,
    },
    Dir {
        path: PathBuf,
        open_spec: String,
        ambiguous: bool,
    },
    Url(String),
    Missing {
        display: String,
    },
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

    if resolved.exists() {
        return existing_target(resolved, open_spec);
    }

    if let Some(collapsed) = resolve_collapsed(&decoded_path, line_suffix.as_deref(), cwd) {
        return collapsed;
    }

    Target::Missing {
        display: raw.to_string(),
    }
}

fn existing_target(resolved: PathBuf, open_spec: String) -> Target {
    if resolved.is_dir() {
        Target::Dir {
            path: resolved,
            open_spec,
            ambiguous: false,
        }
    } else {
        Target::File {
            path: resolved,
            open_spec,
            ambiguous: false,
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

    let mut hits: Vec<Target> = Vec::new();
    for root in fallbacks {
        if root.as_path() == cwd {
            continue;
        }
        match classify(raw, root) {
            Target::File {
                path, ambiguous, ..
            } => {
                hits.push(Target::File {
                    open_spec: open_spec_under_base(raw, &path, cwd),
                    path,
                    ambiguous,
                });
            }
            Target::Dir {
                path, ambiguous, ..
            } => {
                hits.push(Target::Dir {
                    open_spec: open_spec_under_base(raw, &path, cwd),
                    path,
                    ambiguous,
                });
            }
            Target::Url(_) | Target::Missing { .. } => continue,
        }
    }

    match hits.len() {
        0 => primary,
        1 => hits.pop().expect("len 1"),
        _ => mark_ambiguous(hits.into_iter().next().expect("len >= 2")),
    }
}

fn mark_ambiguous(target: Target) -> Target {
    match target {
        Target::File {
            path, open_spec, ..
        } => Target::File {
            path,
            open_spec,
            ambiguous: true,
        },
        Target::Dir {
            path, open_spec, ..
        } => Target::Dir {
            path,
            open_spec,
            ambiguous: true,
        },
        other => other,
    }
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
    // macOS often hands non-canonical cwd (`/var/...`) while canonicalize()
    // yields `/private/var/...`; strip_prefix must use the same form.
    let base = base.canonicalize().unwrap_or_else(|_| base.to_path_buf());
    let resolved = resolved
        .canonicalize()
        .unwrap_or_else(|_| resolved.to_path_buf());
    if let Ok(rel) = resolved.strip_prefix(&base) {
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
            return format!("{home}{}{rest}", if home.ends_with('/') { "" } else { "/" });
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

fn split_collapse_parts(path: &str) -> Option<Vec<String>> {
    let mut parts = Vec::new();
    let mut rest = path;
    let mut found = false;
    loop {
        let ascii = rest.find("...");
        let uni = rest.find('…');
        let hit = match (ascii, uni) {
            (None, None) => None,
            (Some(a), Some(u)) if a <= u => Some((a, 3)),
            (Some(_), Some(u)) => Some((u, '…'.len_utf8())),
            (Some(a), None) => Some((a, 3)),
            (None, Some(u)) => Some((u, '…'.len_utf8())),
        };
        let Some((idx, len)) = hit else {
            break;
        };
        found = true;
        parts.push(rest[..idx].to_string());
        rest = &rest[idx + len..];
    }
    if !found {
        return None;
    }
    parts.push(rest.to_string());
    if !parts.iter().any(|part| part.contains('/')) {
        return None;
    }
    Some(parts)
}

fn rel_matches_collapse(rel: &str, parts: &[String]) -> bool {
    let leading_wild = parts.first().is_some_and(String::is_empty);
    let trailing_wild = parts.last().is_some_and(String::is_empty);
    let literals: Vec<&str> = parts
        .iter()
        .map(String::as_str)
        .filter(|part| !part.is_empty())
        .collect();
    if literals.is_empty() {
        return false;
    }

    let mut pos = 0usize;
    for (index, lit) in literals.iter().enumerate() {
        let last = index + 1 == literals.len();
        if last && !trailing_wild {
            return rel.get(pos..).is_some_and(|tail| tail.ends_with(lit));
        }
        if index == 0 && !leading_wild {
            if !rel.starts_with(lit) {
                return false;
            }
            pos = lit.len();
            continue;
        }
        match rel.get(pos..).and_then(|tail| tail.find(lit)) {
            Some(offset) => pos += offset + lit.len(),
            None => return false,
        }
    }
    trailing_wild
}

fn rel_slash(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn skip_walk_dir(name: &str) -> bool {
    // `worktrees` is skipped on the cwd walk so nested git/Claude worktrees
    // do not lex-steal the hit; `classify_with_fallbacks` searches those roots.
    matches!(name, ".git" | "node_modules" | "target" | "worktrees")
}

fn collect_suffix_matches(root: &Path, parts: &[String], out: &mut Vec<PathBuf>) {
    fn rec(dir: &Path, root: &Path, parts: &[String], out: &mut Vec<PathBuf>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if file_type.is_symlink() {
                continue;
            }
            if file_type.is_dir() {
                let name = entry.file_name();
                if skip_walk_dir(&name.to_string_lossy()) {
                    continue;
                }
                if rel_matches_collapse(&rel_slash(root, &path), parts) {
                    out.push(path.clone());
                }
                rec(&path, root, parts, out);
            } else if file_type.is_file() && rel_matches_collapse(&rel_slash(root, &path), parts) {
                out.push(path);
            }
        }
    }
    rec(root, root, parts, out);
}

fn resolve_collapsed(decoded_path: &str, line_suffix: Option<&str>, cwd: &Path) -> Option<Target> {
    let parts = split_collapse_parts(decoded_path)?;
    let mut matches = Vec::new();
    collect_suffix_matches(cwd, &parts, &mut matches);
    matches.sort_by_key(|path| rel_slash(cwd, path));
    let ambiguous = matches.len() > 1;
    let path = matches.into_iter().next()?;
    let rel = rel_slash(cwd, &path);
    let open_spec = build_open_spec(&rel, line_suffix);
    Some(if path.is_dir() {
        Target::Dir {
            path,
            open_spec,
            ambiguous,
        }
    } else {
        Target::File {
            path,
            open_spec,
            ambiguous,
        }
    })
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

    #[test]
    fn collapse_parts_match_internal_and_leading() {
        let middle = split_collapse_parts("context/spec/…/review.md").unwrap();
        assert!(rel_matches_collapse(
            "context/spec/009-one-call-per-source-synth/review.md",
            &middle
        ));
        assert!(!rel_matches_collapse("context/spec/review.md", &middle));

        let leading = split_collapse_parts("...xt/spec/foo.md").unwrap();
        assert!(rel_matches_collapse("context/spec/foo.md", &leading));

        let both = split_collapse_parts("...xt/spec/…/review.md").unwrap();
        assert!(rel_matches_collapse(
            "context/spec/009-one-call-per-source-synth/review.md",
            &both
        ));
    }
}
