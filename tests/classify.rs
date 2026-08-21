use herdr_preview::classify::{classify, Target};
use std::fs;
use std::path::PathBuf;

fn temp_fixture(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "herdr-preview-classify-{}-{}",
        std::process::id(),
        name
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create fixture dir");
    dir
}

fn file_target_path(target: Target) -> PathBuf {
    match target {
        Target::File { path, .. } => path,
        other => panic!("expected file target, got {other:?}"),
    }
}

fn file_open_spec(target: Target) -> String {
    match target {
        Target::File { open_spec, .. } => open_spec,
        other => panic!("expected file target, got {other:?}"),
    }
}

fn dir_target_path(target: Target) -> PathBuf {
    match target {
        Target::Dir { path, .. } => path,
        other => panic!("expected dir target, got {other:?}"),
    }
}

#[test]
fn classifies_existing_file_relative_to_cwd() {
    let root = temp_fixture("file");
    let cwd = root.join("repo");
    fs::create_dir_all(cwd.join("src")).unwrap();
    fs::write(cwd.join("src/app.rs"), "fn main() {}\n").unwrap();

    let target = classify("src/app.rs", &cwd);

    assert_eq!(file_target_path(target.clone()), cwd.join("src/app.rs"));
    assert_eq!(file_open_spec(target), "src/app.rs");
}

#[test]
fn classifies_existing_directory() {
    let root = temp_fixture("dir");
    let cwd = root.join("repo");
    let docs = cwd.join("docs");
    fs::create_dir_all(&docs).unwrap();

    let target = classify("docs", &cwd);

    assert_eq!(dir_target_path(target.clone()), docs);
    match target {
        Target::Dir { open_spec, .. } => assert_eq!(open_spec, "docs"),
        other => panic!("expected dir target, got {other:?}"),
    }
}

#[test]
fn classifies_https_url_without_filesystem_probe() {
    let root = temp_fixture("url");
    let cwd = root.join("repo");
    fs::create_dir_all(&cwd).unwrap();

    let target = classify("https://github.com/org/repo/pull/1", &cwd);

    assert_eq!(
        target,
        Target::Url("https://github.com/org/repo/pull/1".into())
    );
}

#[test]
fn classifies_missing_path() {
    let root = temp_fixture("missing");
    let cwd = root.join("repo");
    fs::create_dir_all(&cwd).unwrap();

    let target = classify("src/missing.rs", &cwd);

    assert_eq!(
        target,
        Target::Missing {
            display: "src/missing.rs".into()
        }
    );
}

#[test]
fn percent_decodes_filesystem_path() {
    let root = temp_fixture("pct");
    let cwd = root.join("repo");
    fs::create_dir_all(cwd.join("docs")).unwrap();
    fs::write(cwd.join("docs/Tray status.md"), "# tray\n").unwrap();

    let target = classify("docs/Tray%20status.md", &cwd);

    assert_eq!(
        file_target_path(target.clone()),
        cwd.join("docs/Tray status.md")
    );
    assert_eq!(file_open_spec(target), "docs/Tray status.md");
}

#[test]
fn keeps_single_line_suffix_in_open_spec() {
    let root = temp_fixture("line");
    let cwd = root.join("repo");
    fs::create_dir_all(cwd.join("src")).unwrap();
    fs::write(cwd.join("src/app.rs"), "fn main() {}\n").unwrap();

    let target = classify("src/app.rs:42", &cwd);

    assert_eq!(file_target_path(target.clone()), cwd.join("src/app.rs"));
    assert_eq!(file_open_spec(target), "src/app.rs:42");
}

#[test]
fn keeps_line_range_suffix_in_open_spec() {
    let root = temp_fixture("range");
    let cwd = root.join("repo");
    fs::create_dir_all(cwd.join("src")).unwrap();
    fs::write(cwd.join("src/app.rs"), "fn main() {}\n").unwrap();

    let target = classify("src/app.rs:10-20", &cwd);

    assert_eq!(file_target_path(target.clone()), cwd.join("src/app.rs"));
    assert_eq!(file_open_spec(target), "src/app.rs:10-20");
}

#[test]
fn resolves_relative_path_against_cwd() {
    let root = temp_fixture("resolve");
    let cwd = root.join("repo/sub");
    fs::create_dir_all(&cwd).unwrap();
    fs::write(root.join("repo/sibling.md"), "# sibling\n").unwrap();

    let target = classify("../sibling.md", &cwd);

    assert_eq!(
        file_target_path(target.clone()),
        root.join("repo/sibling.md")
    );
    assert_eq!(file_open_spec(target), "../sibling.md");
}

#[test]
fn strips_file_url_prefix_and_resolves() {
    let root = temp_fixture("file-url");
    let cwd = root.join("repo");
    fs::create_dir_all(cwd.join("docs")).unwrap();
    fs::write(cwd.join("docs/readme.md"), "# readme\n").unwrap();

    let target = classify("file://docs/readme.md", &cwd);

    assert_eq!(file_target_path(target.clone()), cwd.join("docs/readme.md"));
    assert_eq!(file_open_spec(target), "docs/readme.md");
}

#[test]
fn expands_tilde_home_paths() {
    let root = temp_fixture("tilde");
    let home = root.join("home");
    let cwd = root.join("cwd");
    fs::create_dir_all(home.join("code/proj")).unwrap();
    fs::create_dir_all(&cwd).unwrap();
    fs::write(home.join("code/proj/README.md"), "# hi\n").unwrap();
    fs::create_dir_all(home.join("scripts")).unwrap();

    let old_home = std::env::var_os("HOME");
    std::env::set_var("HOME", &home);

    let file = classify("~/code/proj/README.md", &cwd);
    let dir = classify("~/scripts/", &cwd);

    match old_home {
        Some(value) => std::env::set_var("HOME", value),
        None => std::env::remove_var("HOME"),
    }

    assert_eq!(
        file_target_path(file.clone()),
        home.join("code/proj/README.md")
    );
    assert_eq!(
        file_open_spec(file),
        format!("{}/code/proj/README.md", home.display())
    );
    assert_eq!(dir_target_path(dir), home.join("scripts"));
}

#[test]
fn fallback_worktree_rewrites_open_spec_under_primary_cwd() {
    use herdr_preview::classify::classify_with_fallbacks;

    let root = temp_fixture("wt");
    let cwd = root.join("repo");
    let wt = cwd.join(".claude/worktrees/feat-x");
    fs::create_dir_all(wt.join("context/spec")).unwrap();
    fs::write(
        wt.join("context/spec/technical-considerations.md"),
        "# tech\n",
    )
    .unwrap();

    assert!(matches!(
        classify("context/spec/technical-considerations.md", &cwd),
        Target::Missing { .. }
    ));

    let target = classify_with_fallbacks(
        "context/spec/technical-considerations.md",
        &cwd,
        &[wt.clone()],
    );
    assert_eq!(
        file_target_path(target.clone()),
        wt.join("context/spec/technical-considerations.md")
    );
    assert_eq!(
        file_open_spec(target),
        ".claude/worktrees/feat-x/context/spec/technical-considerations.md"
    );
}

#[test]
fn resolves_leading_ascii_ellipsis_to_unique_cwd_file() {
    let root = temp_fixture("collapsed-unique");
    let cwd = root.join("repo");
    let dest = cwd.join("context/spec/009-one-call-per-source-synth/review.md");
    fs::create_dir_all(dest.parent().unwrap()).unwrap();
    fs::write(&dest, "# review\n").unwrap();

    let target = classify("...xt/spec/009-one-call-per-source-synth/review.md", &cwd);

    assert_eq!(file_target_path(target.clone()), dest);
    assert_eq!(
        file_open_spec(target),
        "context/spec/009-one-call-per-source-synth/review.md"
    );
}

#[test]
fn resolves_leading_unicode_ellipsis_to_unique_cwd_file() {
    let root = temp_fixture("collapsed-unicode");
    let cwd = root.join("repo");
    let dest = cwd.join("context/spec/009-one-call-per-source-synth/review.md");
    fs::create_dir_all(dest.parent().unwrap()).unwrap();
    fs::write(&dest, "# review\n").unwrap();

    let target = classify("…xt/spec/009-one-call-per-source-synth/review.md", &cwd);

    assert_eq!(file_target_path(target), dest);
}

#[test]
fn resolves_internal_unicode_ellipsis_as_component_wildcard() {
    let root = temp_fixture("collapsed-middle");
    let cwd = root.join("repo");
    let dest = cwd.join("context/spec/009-one-call-per-source-synth/review.md");
    fs::create_dir_all(dest.parent().unwrap()).unwrap();
    fs::write(&dest, "# review\n").unwrap();

    let target = classify("context/spec/…/review.md", &cwd);

    assert_eq!(file_target_path(target.clone()), dest);
    assert_eq!(
        file_open_spec(target),
        "context/spec/009-one-call-per-source-synth/review.md"
    );
}

#[test]
fn resolves_leading_and_internal_ellipsis_together() {
    let root = temp_fixture("collapsed-both");
    let cwd = root.join("repo");
    let dest = cwd.join("context/spec/009-one-call-per-source-synth/review.md");
    fs::create_dir_all(dest.parent().unwrap()).unwrap();
    fs::write(&dest, "# review\n").unwrap();

    let target = classify("...xt/spec/…/review.md", &cwd);

    assert_eq!(file_target_path(target), dest);
}

#[test]
fn collapsed_without_slash_in_remainder_stays_missing() {
    let root = temp_fixture("collapsed-noslash");
    let cwd = root.join("repo");
    fs::create_dir_all(cwd.join("src")).unwrap();
    fs::write(cwd.join("src/app.rs"), "fn main() {}\n").unwrap();

    let target = classify("...rs", &cwd);

    assert_eq!(
        target,
        Target::Missing {
            display: "...rs".into()
        }
    );
}

#[test]
fn parent_dir_prefix_is_not_treated_as_collapsed() {
    let root = temp_fixture("not-collapsed-dotdot");
    let cwd = root.join("repo/sub");
    fs::create_dir_all(&cwd).unwrap();
    fs::write(root.join("repo/sibling.md"), "# sibling\n").unwrap();

    let target = classify("../sibling.md", &cwd);

    assert_eq!(file_target_path(target), root.join("repo/sibling.md"));
}

#[test]
fn collapsed_keeps_line_suffix_on_resolved_open_spec() {
    let root = temp_fixture("collapsed-line");
    let cwd = root.join("repo");
    let dest = cwd.join("context/spec/009-one-call-per-source-synth/review.md");
    fs::create_dir_all(dest.parent().unwrap()).unwrap();
    fs::write(&dest, "# review\n").unwrap();

    let target = classify(
        "...xt/spec/009-one-call-per-source-synth/review.md:12",
        &cwd,
    );

    assert_eq!(file_target_path(target.clone()), dest);
    assert_eq!(
        file_open_spec(target),
        "context/spec/009-one-call-per-source-synth/review.md:12"
    );
}

#[test]
fn collapsed_ambiguous_cwd_picks_lexicographic_first() {
    let root = temp_fixture("collapsed-ambig-cwd");
    let cwd = root.join("repo");
    let a = cwd.join("aaa/xt/spec/009-one-call-per-source-synth/review.md");
    let z = cwd.join("zzz/xt/spec/009-one-call-per-source-synth/review.md");
    fs::create_dir_all(a.parent().unwrap()).unwrap();
    fs::create_dir_all(z.parent().unwrap()).unwrap();
    fs::write(&a, "a\n").unwrap();
    fs::write(&z, "z\n").unwrap();

    let target = classify("...xt/spec/009-one-call-per-source-synth/review.md", &cwd);

    match target {
        Target::File {
            path, ambiguous, ..
        } => {
            assert_eq!(path, a);
            assert!(ambiguous);
        }
        other => panic!("expected file, got {other:?}"),
    }
}

#[test]
fn collapsed_skips_node_modules_git_and_target_dirs() {
    let root = temp_fixture("collapsed-skip");
    let cwd = root.join("repo");
    let hidden = cwd.join("node_modules/xt/spec/009-one-call-per-source-synth/review.md");
    fs::create_dir_all(hidden.parent().unwrap()).unwrap();
    fs::write(&hidden, "pkg\n").unwrap();

    let target = classify("...xt/spec/009-one-call-per-source-synth/review.md", &cwd);

    assert_eq!(
        target,
        Target::Missing {
            display: "...xt/spec/009-one-call-per-source-synth/review.md".into()
        }
    );
}

#[test]
fn collapsed_prefers_cwd_over_worktree_fallback() {
    use herdr_preview::classify::classify_with_fallbacks;

    let root = temp_fixture("collapsed-cwd-wins");
    let cwd = root.join("repo");
    let wt = cwd.join(".claude/worktrees/other");
    let cwd_file = cwd.join("context/spec/009-one-call-per-source-synth/review.md");
    let wt_file = wt.join("context/spec/009-one-call-per-source-synth/review.md");
    fs::create_dir_all(cwd_file.parent().unwrap()).unwrap();
    fs::create_dir_all(wt_file.parent().unwrap()).unwrap();
    fs::write(&cwd_file, "cwd\n").unwrap();
    fs::write(&wt_file, "wt\n").unwrap();

    let target = classify_with_fallbacks(
        "...xt/spec/009-one-call-per-source-synth/review.md",
        &cwd,
        &[wt],
    );

    match target {
        Target::File {
            path, ambiguous, ..
        } => {
            assert_eq!(path, cwd_file);
            assert!(!ambiguous);
        }
        other => panic!("expected cwd file, got {other:?}"),
    }
}

#[test]
fn collapsed_unique_worktree_hit_rewrites_open_spec() {
    use herdr_preview::classify::classify_with_fallbacks;

    let root = temp_fixture("collapsed-wt-unique");
    let cwd = root.join("repo");
    let wt = cwd.join(".claude/worktrees/feat-x");
    let dest = wt.join("context/spec/009-one-call-per-source-synth/review.md");
    fs::create_dir_all(&cwd).unwrap();
    fs::create_dir_all(dest.parent().unwrap()).unwrap();
    fs::write(&dest, "# review\n").unwrap();

    let target = classify_with_fallbacks(
        "...xt/spec/009-one-call-per-source-synth/review.md",
        &cwd,
        &[wt.clone()],
    );

    assert_eq!(file_target_path(target.clone()), dest);
    assert_eq!(
        file_open_spec(target.clone()),
        ".claude/worktrees/feat-x/context/spec/009-one-call-per-source-synth/review.md"
    );
    match target {
        Target::File { ambiguous, .. } => assert!(!ambiguous),
        other => panic!("expected file, got {other:?}"),
    }
}

#[test]
fn collapsed_two_worktrees_opens_first_fallback_as_ambiguous() {
    use herdr_preview::classify::classify_with_fallbacks;

    let root = temp_fixture("collapsed-wt-ambig");
    let cwd = root.join("repo");
    let wt_a = cwd.join(".claude/worktrees/aaa");
    let wt_z = cwd.join(".claude/worktrees/zzz");
    let a = wt_a.join("context/spec/009-one-call-per-source-synth/review.md");
    let z = wt_z.join("context/spec/009-one-call-per-source-synth/review.md");
    fs::create_dir_all(&cwd).unwrap();
    fs::create_dir_all(a.parent().unwrap()).unwrap();
    fs::create_dir_all(z.parent().unwrap()).unwrap();
    fs::write(&a, "a\n").unwrap();
    fs::write(&z, "z\n").unwrap();

    let target = classify_with_fallbacks(
        "...xt/spec/009-one-call-per-source-synth/review.md",
        &cwd,
        &[wt_a.clone(), wt_z],
    );

    match target {
        Target::File {
            path, ambiguous, ..
        } => {
            assert_eq!(path, a);
            assert!(ambiguous);
        }
        other => panic!("expected first-fallback file, got {other:?}"),
    }
}
