pub struct Span {
    pub start: usize,
    pub end: usize,
    pub raw: String,
}

pub fn find_candidates(text: &str) -> Vec<Span> {
    let mut spans = Vec::new();
    let mut line_start = 0;

    for line in text.split_inclusive('\n') {
        find_in_line(line.trim_end_matches('\n'), line_start, &mut spans);
        line_start += line.len();
    }

    spans
}

/// End offsets where a span ending at `end` could continue through the
/// space-separated words that follow it on the same line; callers probe the
/// disk to pick one. The run stops at punctuation (`,` `|` `#` `·` …), at a
/// duration like `4.4s`, and right after a word ending in a file extension.
pub fn spaced_continuation_ends(text: &str, end: usize) -> Vec<usize> {
    let line_end = text[end..].find('\n').map_or(text.len(), |rel| end + rel);
    let rest = &text[end..line_end];
    let mut ends = Vec::new();
    if !rest.starts_with(' ') {
        return ends;
    }
    for (word_start, raw_end) in word_bounds(rest) {
        let (_, word_end) = trim_wrappers(rest, (word_start, raw_end));
        let word = &rest[word_start..word_end];
        if is_duration_token(word) || !is_path_continuation(word) {
            break;
        }
        ends.push(end + word_end);
        if word_end != raw_end || has_file_extension(word) {
            break;
        }
    }
    ends
}

/// `name.ext` with a 1–4 char extension (optionally `:line`), like `\.\w{1,4}$`.
pub fn has_file_extension(word: &str) -> bool {
    let final_component = word.rsplit('/').next().unwrap_or(word);
    let name = final_component.split(':').next().unwrap_or(final_component);
    name.rfind('.').is_some_and(|dot| {
        let ext = &name[dot + 1..];
        dot > 0
            && (1..=4).contains(&ext.len())
            && ext.chars().all(|c| c.is_alphanumeric() || c == '_')
    })
}

fn find_in_line(line: &str, line_start: usize, spans: &mut Vec<Span>) {
    // Wrapped one-line JSON (`cat *.json`) breaks into non-`{` rows; word mode
    // would treat `ess":true,"stdout":"…/home/…` as one path-like word. Still run
    // word/markdown passes afterward so relative paths on the same line are found.
    let mut covered: Vec<(usize, usize)> = Vec::new();
    if line_looks_like_json_payload(line) {
        find_embedded_absolute_paths(line, line_start, spans, &mut covered);
    }

    // Shell/Markdown panes often wrap destinations as `[label](path)` — extract the
    // destination before word-splitting so `docs/a.md](docs/a.md` is not one token.
    for (dest_start, dest_end) in markdown_link_destinations(line) {
        let dest = &line[dest_start..dest_end];
        if is_url(dest) || is_path_start(dest) {
            let range = text_range(line_start, dest_start, dest_end);
            if !overlaps_any(range.0, range.1, &covered) {
                spans.push(make_span(range, dest));
                covered.push(range);
            }
        }
    }

    let words = word_bounds(line);
    let mut index = 0;

    while index < words.len() {
        let (word_start, word_end) = trim_wrappers(line, words[index]);
        let (word_start, word_end) =
            env_assignment_path(line, word_start, word_end).unwrap_or((word_start, word_end));
        let word = &line[word_start..word_end];
        let abs_start = line_start + word_start;
        let abs_end = line_start + word_end;
        if overlaps_any(abs_start, abs_end, &covered) {
            index += 1;
            continue;
        }

        if is_json_blob_word(word) {
            index += 1;
            continue;
        }

        if is_url(word) {
            spans.push(make_span(
                text_range(line_start, word_start, word_end),
                word,
            ));
            index += 1;
            continue;
        }

        if is_path_start(word) {
            if let Some((path_start, path_end)) = shell_prompt_path_range(line, word_start, word_end)
            {
                let range = text_range(line_start, path_start, path_end);
                if !overlaps_any(range.0, range.1, &covered) {
                    let raw = &line[path_start..path_end];
                    spans.push(make_span(range, raw));
                    covered.push(range);
                }
                index += 1;
                continue;
            }

            let colon_ranges = path_list_colon_ranges(line, word_start, word_end);
            if colon_ranges.len() >= 2 {
                for (path_start, path_end) in colon_ranges {
                    let range = text_range(line_start, path_start, path_end);
                    if overlaps_any(range.0, range.1, &covered) {
                        continue;
                    }
                    let raw = &line[path_start..path_end];
                    spans.push(make_span(range, raw));
                    covered.push(range);
                }
                index += 1;
                continue;
            }

            let mut candidate_end = word_end;
            let mut consumed_through = index;

            if !has_filename_shape(word) {
                let starts_at_directory_boundary = word.ends_with('/');
                for (next_index, bounds) in words.iter().enumerate().skip(index + 1) {
                    let (next_start, next_end) = trim_wrappers(line, *bounds);
                    let next_word = &line[next_start..next_end];
                    if is_duration_token(next_word) {
                        break;
                    }
                    if !is_path_continuation(next_word) {
                        break;
                    }
                    // A later slash-bearing token starts a new path (e.g. "src/foo and
                    // docs/") unless it is the immediate next word providing
                    // `folder/file.ext` after a spaced directory name.
                    let is_immediate_next_word = next_index == index + 1;
                    if next_word.contains('/') && !is_immediate_next_word {
                        break;
                    }
                    if has_filename_shape(next_word) {
                        let has_nested_separator = next_word.contains('/');
                        let has_distinctive_name = has_distinctive_path_punctuation(next_word);

                        if has_nested_separator
                            || (!starts_at_directory_boundary
                                && (is_immediate_next_word || has_distinctive_name))
                        {
                            candidate_end = next_end;
                            consumed_through = next_index;
                        }
                        break;
                    }
                }
            }

            let range = text_range(line_start, word_start, candidate_end);
            if !overlaps_any(range.0, range.1, &covered) {
                let raw = &line[word_start..candidate_end];
                spans.push(make_span(range, raw));
                covered.push(range);
            }
            index = consumed_through + 1;
            continue;
        }

        index += 1;
    }

    find_embedded_absolute_paths(line, line_start, spans, &mut covered);
}

/// Paths inside JSON strings, `cat` one-liners, etc. — not whitespace-delimited words.
fn find_embedded_absolute_paths(
    line: &str,
    line_start: usize,
    spans: &mut Vec<Span>,
    covered: &mut Vec<(usize, usize)>,
) {
    for (i, ch) in line.char_indices() {
        let at_tilde = ch == '~' && line[i..].starts_with("~/");
        let at_slash = ch == '/'
            && !line[i..].starts_with("//")
            && embedded_slash_starts_absolute_path(line, i);
        if !at_tilde && !at_slash {
            continue;
        }
        if at_slash && embedded_slash_is_url(line, i) {
            continue;
        }
        let len = embedded_path_byte_len(&line[i..]);
        if len == 0 {
            continue;
        }
        let end = i + len;
        let raw = &line[i..end];
        if is_rustc_json_placeholder_path(raw) {
            continue;
        }
        let path_like = is_path_start(raw)
            && (has_filename_shape(raw) || (raw.contains('/') && raw.len() > 1));
        if path_like {
            let range = text_range(line_start, i, end);
            if !overlaps_any(range.0, range.1, covered) {
                spans.push(make_span(range, raw));
                covered.push(range);
            }
        }
    }
}

/// `/` inside `src/app.rs` is not an embedded absolute path; `\n/home` in JSON is.
fn embedded_slash_starts_absolute_path(line: &str, slash: usize) -> bool {
    if slash == 0 {
        return true;
    }
    let bytes = line.as_bytes();
    if slash >= 2 && bytes[slash - 2] == b'\\' {
        return true;
    }
    let prev = bytes[slash - 1];
    if prev.is_ascii_whitespace()
        || matches!(prev, b'"' | b'\'' | b'(' | b':' | b',' | b';' | b'=' | b'[' | b'{')
    {
        return true;
    }
    !(prev.is_ascii_alphanumeric() || prev == b'_' || prev == b'.' || prev == b'-')
}

fn embedded_slash_is_url(line: &str, slash: usize) -> bool {
    if slash > 0 && line.as_bytes()[slash - 1] == b':' {
        return true;
    }
    if slash > 0 && line.as_bytes()[slash - 1] == b'/' {
        return true;
    }
    if let Some(colon) = line[..slash].rfind("://") {
        let host_start = colon + 3;
        if slash >= host_start {
            return true;
        }
    }
    false
}

fn embedded_path_byte_len(s: &str) -> usize {
    if s.starts_with("~/") {
        return 2 + embedded_path_body_len(&s[2..]);
    }
    if s.starts_with('/') {
        return 1 + embedded_path_body_len(&s[1..]);
    }
    0
}

fn embedded_path_body_len(s: &str) -> usize {
    let mut len = 0usize;
    for ch in s.chars() {
        if ch == '/'
            || ch.is_alphanumeric()
            || matches!(ch, '_' | '-' | '.' | '+' | '~' | '%' | '…')
        {
            len += ch.len_utf8();
            continue;
        }
        if ch == '\\' {
            break;
        }
        if matches!(ch, '"' | '\'' | ',' | '}' | ']' | ')' | ' ' | '\t') {
            break;
        }
        break;
    }
    len
}

/// Destinations inside Markdown inline links: `[label](dest)`.
fn markdown_link_destinations(line: &str) -> Vec<(usize, usize)> {
    let bytes = line.as_bytes();
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] != b'[' {
            i += 1;
            continue;
        }
        let Some(close_bracket) = line[i + 1..].find(']').map(|rel| i + 1 + rel) else {
            break;
        };
        let after = close_bracket + 1;
        if after >= bytes.len() || bytes[after] != b'(' {
            i = close_bracket + 1;
            continue;
        }
        let dest_start = after + 1;
        let Some(close_paren) = line[dest_start..].find(')').map(|rel| dest_start + rel) else {
            break;
        };
        let dest = line[dest_start..close_paren].trim();
        // Skip title-bearing destinations (`path "title"`) for MVP.
        if !dest.is_empty() && !dest.contains([' ', '\t']) {
            let trimmed_start = dest_start + (line[dest_start..close_paren].len() - dest.len());
            out.push((trimmed_start, trimmed_start + dest.len()));
        }
        i = close_paren + 1;
    }
    out
}

fn overlaps_any(start: usize, end: usize, covered: &[(usize, usize)]) -> bool {
    covered.iter().any(|&(c0, c1)| start < c1 && end > c0)
}

fn word_bounds(line: &str) -> Vec<(usize, usize)> {
    let mut words = Vec::new();
    let mut start = None;

    for (index, character) in line.char_indices() {
        if character.is_whitespace() {
            if let Some(word_start) = start.take() {
                words.push((word_start, index));
            }
        } else if start.is_none() {
            start = Some(index);
        }
    }

    if let Some(word_start) = start {
        words.push((word_start, line.len()));
    }

    words
}

fn trim_wrappers(line: &str, (mut start, mut end): (usize, usize)) -> (usize, usize) {
    while start < end {
        let character = line[start..end].chars().next().expect("non-empty slice");
        if matches!(character, '"' | '\'' | '`' | '(' | '[' | '{' | '<') {
            start += character.len_utf8();
        } else {
            break;
        }
    }

    while start < end {
        let character = line[start..end]
            .chars()
            .next_back()
            .expect("non-empty slice");
        if matches!(
            character,
            '"' | '\'' | '`' | ')' | ']' | '}' | '>' | ',' | ';' | '.' | '|'
        ) {
            end -= character.len_utf8();
        } else {
            break;
        }
    }

    (start, end)
}

fn env_assignment_path(line: &str, start: usize, end: usize) -> Option<(usize, usize)> {
    let word = &line[start..end];
    let (lhs, rhs) = word.split_once('=')?;
    if lhs.is_empty()
        || !lhs.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        || !lhs
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
    {
        return None;
    }
    if !is_path_start(rhs) {
        return None;
    }
    let rhs_start = start + lhs.len() + 1;
    Some((rhs_start, end))
}

fn is_duration_token(word: &str) -> bool {
    let Some(rest) = word.strip_suffix('s') else {
        return false;
    };
    if rest.is_empty() || !rest.chars().any(|c| c.is_ascii_digit()) {
        return false;
    }
    rest.chars().all(|c| c.is_ascii_digit() || c == '.')
}

fn is_url(word: &str) -> bool {
    word.starts_with("http://") || word.starts_with("https://")
}

/// One-line JSON / `cat` blobs — scan embedded `/…` paths instead of whole-word match.
fn is_json_blob_word(word: &str) -> bool {
    word.starts_with('{')
        || word.contains("\":\"")
        || (word.starts_with('"') && word.contains('{'))
}

/// Whole line or terminal-wrapped fragment of JSON tool output (e.g. `.rustc_info.json`).
fn line_looks_like_json_payload(line: &str) -> bool {
    line.starts_with('{')
        || line.contains("\"rustc_fingerprint\"")
        || line.contains("ess\":true")
        || line.contains(":true,\"")
        || line.contains(",\"code\":")
        || ((line.contains("\"stdout\"") || line.contains("\"stderr\""))
            && line.contains("\":\""))
}

/// Placeholder paths inside rustc target-json blobs, not real filesystem locations.
fn is_rustc_json_placeholder_path(raw: &str) -> bool {
    raw.starts_with("/lib___.") || raw == "/lib___"
}

/// `PATH`-style list (`/usr/bin:/usr/sbin`) — not `file.rs:42` or `user@host:~/x$`.
fn path_list_colon_ranges(line: &str, word_start: usize, word_end: usize) -> Vec<(usize, usize)> {
    let word = &line[word_start..word_end];
    if word.contains("://") || !word.contains(':') {
        return Vec::new();
    }
    if shell_prompt_path_range(line, word_start, word_end).is_some() {
        return Vec::new();
    }
    if is_file_line_suffix_token(word) {
        return Vec::new();
    }
    let segments: Vec<&str> = word.split(':').collect();
    if segments.len() < 2 || !segments.iter().all(|seg| is_path_list_segment(seg)) {
        return Vec::new();
    }
    let mut ranges = Vec::with_capacity(segments.len());
    let mut pos = 0usize;
    for (index, seg) in segments.iter().enumerate() {
        if index > 0 {
            pos += 1;
        }
        let start = pos;
        pos += seg.len();
        ranges.push((word_start + start, word_start + pos));
    }
    ranges
}

fn is_path_list_segment(seg: &str) -> bool {
    let seg = seg.trim();
    if seg.is_empty() {
        return false;
    }
    seg.starts_with('/')
        || seg.starts_with('~')
        || seg.starts_with('.')
        || (seg.contains('/') && is_path_continuation(seg))
}

/// True when the only colon separates a file path from a `:line` / `:start-end` suffix.
fn is_file_line_suffix_token(word: &str) -> bool {
    let Some((path, suffix)) = word.rsplit_once(':') else {
        return false;
    };
    if path.is_empty() {
        return false;
    }
    is_line_number_suffix(suffix) && has_filename_shape(path)
}

fn is_line_number_suffix(suffix: &str) -> bool {
    if let Ok(number) = suffix.parse::<usize>() {
        return number >= 1;
    }
    let Some((start, end)) = suffix.split_once('-') else {
        return false;
    };
    let Ok(start) = start.parse::<usize>() else {
        return false;
    };
    let Ok(end) = end.parse::<usize>() else {
        return false;
    };
    start >= 1 && end >= 1
}

/// Bash/zsh invitation token (`user@host:~/repo$`): return the path slice only.
fn shell_prompt_path_range(line: &str, word_start: usize, word_end: usize) -> Option<(usize, usize)> {
    let word = &line[word_start..word_end];
    if !word.contains('@') {
        return None;
    }
    let trimmed = word.trim_end();
    if !trimmed
        .chars()
        .last()
        .is_some_and(|c| matches!(c, '$' | '#' | '>' | '%'))
    {
        return None;
    }
    let without_invitation = trimmed.trim_end_matches(|c| matches!(c, '$' | '#' | '>' | '%'));
    let colon = without_invitation.rfind(':')?;
    let at = without_invitation.find('@')?;
    if colon <= at {
        return None;
    }
    let path_part = without_invitation[colon + 1..].trim();
    if path_part.is_empty() || !is_path_start(path_part) {
        return None;
    }
    let path_start_in_word = colon + 1 + (without_invitation[colon + 1..].len() - path_part.len());
    let path_end_in_word = path_start_in_word + path_part.len();
    Some((word_start + path_start_in_word, word_start + path_end_in_word))
}

fn is_path_start(word: &str) -> bool {
    if is_url(word) || word.is_empty() || word == "/" || word == "." || word == ".." {
        return false;
    }
    // Relative/absolute with a separator, directory slash, or bare filename.ext
    word.contains('/') || is_bare_filename(word)
}

fn is_bare_filename(word: &str) -> bool {
    if word.contains('/') || is_duration_token(word) {
        return false;
    }
    let name = word.split(':').next().unwrap_or(word);
    if name.is_empty() || name == "." || name == ".." {
        return false;
    }
    // Require a non-leading '.' so Cargo.toml / main.rs / .gitignore match,
    // but plain words like "error" / "src" do not.
    match name.rfind('.') {
        Some(0) => {
            // ".gitignore" — hidden file without a second dot: treat as path-like.
            name.len() > 1
                && name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
        }
        Some(dot) => {
            let ext = &name[dot + 1..];
            !ext.is_empty()
                && ext.chars().all(|c| c.is_ascii_alphanumeric())
                && name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '+'))
        }
        None => false,
    }
}

fn has_filename_shape(word: &str) -> bool {
    if word.ends_with('/') {
        return true;
    }
    let final_component = word.rsplit('/').next().unwrap_or(word);
    let name = final_component.split(':').next().unwrap_or(final_component);
    is_bare_filename(name)
        || name
            .rfind('.')
            .is_some_and(|dot| dot > 0 && dot + 1 < name.len())
}

fn has_distinctive_path_punctuation(word: &str) -> bool {
    let final_component = word.rsplit('/').next().unwrap_or(word);
    let name = final_component.split(':').next().unwrap_or(final_component);
    let stem = name.rsplit_once('.').map_or(name, |(stem, _)| stem);

    stem.chars()
        .any(|character| matches!(character, '-' | '+' | '~' | '%' | '…'))
}

fn is_path_continuation(word: &str) -> bool {
    !word.is_empty()
        && word.chars().all(|character| {
            character.is_alphanumeric()
                || matches!(character, '_' | '-' | '.' | '+' | '~' | '%' | '…' | '/')
        })
}

fn text_range(line_start: usize, start: usize, end: usize) -> (usize, usize) {
    (line_start + start, line_start + end)
}

fn make_span((start, end): (usize, usize), raw: &str) -> Span {
    Span {
        start,
        end,
        raw: raw.to_owned(),
    }
}
