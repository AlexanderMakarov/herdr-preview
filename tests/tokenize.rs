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
fn finds_absolute_path_embedded_in_json_one_liner() {
    let line = r#"{"stdout":"___\n/home/user/.rustup/toolchains/stable-x86_64-unknown-linux-gnu\noff"}"#;
    let spans = find_candidates(line);
    assert!(
        spans
            .iter()
            .any(|s| s.raw == "/home/user/.rustup/toolchains/stable-x86_64-unknown-linux-gnu"),
        "expected embedded rustup toolchain path, got {:?}",
        spans.iter().map(|s| &s.raw).collect::<Vec<_>>()
    );
}

#[test]
fn json_heuristic_line_still_finds_relative_path() {
    let line = r#"ess":true, edit src/app.rs, "stdout":"/home/user/out.log""#;
    let spans = find_candidates(line);
    let raws: Vec<_> = spans.iter().map(|s| s.raw.as_str()).collect();
    assert!(
        raws.contains(&"src/app.rs"),
        "relative paths must survive json-heuristic lines, got {raws:?}"
    );
    assert!(
        raws.contains(&"/home/user/out.log"),
        "embedded absolutes must still be found, got {raws:?}"
    );
}

#[test]
fn terminal_wrapped_json_fragment_uses_embedded_paths_not_word_blob() {
    let line = r#"ess":true,"status":"","code":0,"stdout":"___\nlib___.rlib\n/home/user/.rustup/toolchains/stable-x86_64-unknown-linux-gnu\noff"#;
    let spans = find_candidates(line);
    let raws: Vec<_> = spans.iter().map(|s| s.raw.as_str()).collect();
    assert!(
        raws.contains(&"/home/user/.rustup/toolchains/stable-x86_64-unknown-linux-gnu"),
        "expected rustup path, got {raws:?}"
    );
    assert!(
        !raws.iter().any(|r| r.contains("ess\":true")),
        "must not treat wrapped json as one word span, got {raws:?}"
    );
    assert!(!raws.iter().any(|r| r.starts_with("/lib___.")));
}

#[test]
fn splits_path_list_on_colon() {
    let line = "PATH=/usr/bin:/usr/etc";
    let spans = find_candidates(line);
    let raws: Vec<_> = spans.iter().map(|s| s.raw.as_str()).collect();
    assert!(raws.contains(&"/usr/bin"));
    assert!(raws.contains(&"/usr/etc"));
}

#[test]
fn keeps_file_line_suffix_as_one_token() {
    let spans = find_candidates("open src/app.rs:42 now");
    assert!(spans.iter().any(|s| s.raw == "src/app.rs:42"));
    assert!(!spans.iter().any(|s| s.raw == "src/app.rs"));
}

#[test]
fn finds_directory_path_inside_bash_shell_invitation() {
    let line = "user@host:~/code/scripts$";
    let spans = find_candidates(line);

    assert_eq!(spans.len(), 1);
    assert_eq!(spans[0].raw, "~/code/scripts");
    assert_eq!(&line[spans[0].start..spans[0].end], "~/code/scripts");
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

#[test]
fn extracts_markdown_link_destinations() {
    let text = "| [docs/index.md](docs/index.md) | see also [setup](docs/tutorials/setup-guide.md)";
    let spans = find_candidates(text);
    let raws: Vec<_> = spans.iter().map(|s| s.raw.as_str()).collect();
    assert!(
        raws.contains(&"docs/index.md"),
        "expected docs/index.md in {raws:?}"
    );
    assert!(
        raws.contains(&"docs/tutorials/setup-guide.md"),
        "expected setup-guide path in {raws:?}"
    );
    assert!(
        !raws.iter().any(|r| r.contains("](")),
        "should not keep glued markdown tokens: {raws:?}"
    );
}

#[test]
fn finds_tilde_home_paths() {
    let spans = find_candidates("less ~/code/llm-wiki/README.md and ~/scripts/");
    let raws: Vec<_> = spans.iter().map(|s| s.raw.as_str()).collect();
    assert!(raws.contains(&"~/code/llm-wiki/README.md"), "{raws:?}");
    assert!(raws.contains(&"~/scripts/"), "{raws:?}");
}

#[test]
fn finds_cursor_style_collapsed_path() {
    let text = "    Read ...xt/spec/009-one-call-per-source-synth/review.md";
    let spans = find_candidates(text);
    assert_eq!(spans.len(), 1);
    assert_eq!(
        spans[0].raw,
        "...xt/spec/009-one-call-per-source-synth/review.md"
    );
}

#[test]
fn splits_env_assignment_keeping_path_after_equals() {
    let text = "CARGO_TARGET_DIR=/home/user/code/herdr-preview/target cargo build";
    let spans = find_candidates(text);
    let raws: Vec<_> = spans.iter().map(|s| s.raw.as_str()).collect();
    assert!(
        raws.contains(&"/home/user/code/herdr-preview/target"),
        "{raws:?}"
    );
    assert!(
        !raws.iter().any(|r| r.contains("CARGO_TARGET_DIR=")),
        "{raws:?}"
    );
}

#[test]
fn does_not_join_command_duration_onto_extensionless_binary() {
    let text = "/home/user/code/herdr-preview/target/release/herdr-preview 4.4s";
    let spans = find_candidates(text);
    let raws: Vec<_> = spans.iter().map(|s| s.raw.as_str()).collect();
    assert!(
        raws.contains(&"/home/user/code/herdr-preview/target/release/herdr-preview"),
        "{raws:?}"
    );
    assert!(!raws.iter().any(|r| r.contains("4.4s")), "{raws:?}");
}
