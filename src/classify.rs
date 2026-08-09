use std::path::{Component, Path, PathBuf};

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
    let decoded_path = percent_decode(path_part);
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
