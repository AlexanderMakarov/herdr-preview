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
fn keeps_nested_spaced_path_in_one_candidate() {
    let spans = find_candidates("open /tmp/My folder/file.md");

    assert_eq!(spans.len(), 1);
    assert_eq!(spans[0].raw, "/tmp/My folder/file.md");
}

#[test]
fn does_not_join_directory_prefix_through_prose() {
    let spans = find_candidates("docs/ directory and file.md");

    assert!(spans.iter().any(|s| s.raw == "docs/"));
    assert!(spans.iter().any(|s| s.raw == "file.md"));
    assert!(!spans.iter().any(|s| s.raw.contains("directory and")));
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
fn finds_bare_filename_with_extension() {
    let spans = find_candidates("edit Cargo.toml please");
    assert_eq!(spans.len(), 1);
    assert_eq!(spans[0].raw, "Cargo.toml");
}

#[test]
fn finds_extensionless_slash_path() {
    let spans = find_candidates("see src/tokenize and docs/");
    assert!(spans.iter().any(|s| s.raw == "src/tokenize"));
    assert!(spans.iter().any(|s| s.raw == "docs/"));
}

#[test]
fn ignores_plain_prose_words() {
    let spans = find_candidates("nothing pathlike here at all");
    assert!(spans.is_empty());
}
