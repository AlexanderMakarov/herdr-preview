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

fn find_in_line(line: &str, line_start: usize, spans: &mut Vec<Span>) {
    // Shell/Markdown panes often wrap destinations as `[label](path)` — extract the
    // destination before word-splitting so `docs/a.md](docs/a.md` is not one token.
    let mut covered: Vec<(usize, usize)> = Vec::new();
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
        let word = &line[word_start..word_end];
        let abs_start = line_start + word_start;
        let abs_end = line_start + word_end;
        if overlaps_any(abs_start, abs_end, &covered) {
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
            let mut candidate_end = word_end;
            let mut consumed_through = index;

            if !has_filename_shape(word) {
                let starts_at_directory_boundary = word.ends_with('/');
                for (next_index, bounds) in words.iter().enumerate().skip(index + 1) {
                    let (next_start, next_end) = trim_wrappers(line, *bounds);
                    let next_word = &line[next_start..next_end];
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
    covered
        .iter()
        .any(|&(c0, c1)| start < c1 && end > c0)
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

fn is_url(word: &str) -> bool {
    word.starts_with("http://") || word.starts_with("https://")
}

fn is_path_start(word: &str) -> bool {
    if is_url(word) || word.is_empty() || word == "/" || word == "." || word == ".." {
        return false;
    }
    // Relative/absolute with a separator, directory slash, or bare filename.ext
    word.contains('/') || is_bare_filename(word)
}

fn is_bare_filename(word: &str) -> bool {
    if word.contains('/') {
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
                && name.chars().all(|c| {
                    c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.')
                })
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
