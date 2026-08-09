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
    let words = word_bounds(line);
    let mut index = 0;

    while index < words.len() {
        let (word_start, word_end) = trim_wrappers(line, words[index]);
        let word = &line[word_start..word_end];

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
                    if has_filename_shape(next_word) {
                        let has_nested_separator = next_word.contains('/');
                        let is_immediate_next_word = next_index == index + 1;
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

            let raw = &line[word_start..candidate_end];
            spans.push(make_span(
                text_range(line_start, word_start, candidate_end),
                raw,
            ));
            index = consumed_through + 1;
            continue;
        }

        index += 1;
    }
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
            '"' | '\'' | '`' | ')' | ']' | '}' | '>' | ',' | ';' | '.'
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
    !is_url(word) && word.contains('/') && word != "/"
}

fn has_filename_shape(word: &str) -> bool {
    let final_component = word.rsplit('/').next().unwrap_or(word);
    final_component
        .split(':')
        .next()
        .is_some_and(|name| name.rfind('.').is_some_and(|dot| dot > 0))
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
