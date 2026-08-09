use herdr_preview::tokenize::find_candidates;

#[test]
fn finds_bare_relative_path_with_byte_offsets() {
    let spans = find_candidates("open docs/foo.md now");

    assert_eq!(spans.len(), 1);
    assert_eq!(spans[0].start, 5);
    assert_eq!(spans[0].end, 16);
    assert_eq!(spans[0].raw, "docs/foo.md");
}

#[test]
fn finds_absolute_path() {
    let spans = find_candidates("read /tmp/x");

    assert_eq!(spans.len(), 1);
    assert_eq!(spans[0].raw, "/tmp/x");
}

#[test]
fn keeps_clearly_path_shaped_spaces_in_one_candidate() {
    let text = "open …/Tray status icon-….plan.md next";
    let spans = find_candidates(text);

    assert_eq!(spans.len(), 1);
    assert_eq!(spans[0].raw, "…/Tray status icon-….plan.md");
    assert_eq!(&text[spans[0].start..spans[0].end], spans[0].raw);
}

#[test]
fn keeps_percent_encoded_spaces_in_raw_candidate() {
    let spans = find_candidates("open docs/Tray%20status.md");

    assert_eq!(spans.len(), 1);
    assert_eq!(spans[0].raw, "docs/Tray%20status.md");
}

#[test]
fn finds_https_url_candidate() {
    let spans = find_candidates("see https://github.com/org/repo/pull/1");

    assert_eq!(spans.len(), 1);
    assert_eq!(spans[0].raw, "https://github.com/org/repo/pull/1");
}

#[test]
fn keeps_line_suffix_in_raw_candidate() {
    let spans = find_candidates("error at src/app.rs:42");

    assert_eq!(spans.len(), 1);
    assert_eq!(spans[0].raw, "src/app.rs:42");
}

#[test]
fn keeps_line_range_suffix_in_raw_candidate() {
    let spans = find_candidates("inspect src/app.rs:10-20");

    assert_eq!(spans.len(), 1);
    assert_eq!(spans[0].raw, "src/app.rs:10-20");
}
