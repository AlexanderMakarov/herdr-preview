use herdr_preview::hint::{
    build_entries, format_list, run_hint_list, serialize_entries, HINT_KEYS,
};
use std::fs;
use std::path::PathBuf;

fn temp_fixture(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "herdr-preview-hint-it-{}-{}",
        std::process::id(),
        name
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create fixture dir");
    dir
}

#[test]
fn headless_list_mode_prints_path_targets_not_urls() {
    let root = temp_fixture("list");
    let cwd = root.join("repo");
    fs::create_dir_all(cwd.join("docs")).unwrap();
    fs::write(cwd.join("docs/plan.md"), "# plan\n").unwrap();

    let text = "see docs/plan.md and docs/missing.md\nhttps://github.com/org/repo/pull/1\n";
    let output = run_hint_list(text, &cwd).expect("list");

    assert!(output.contains("docs/plan.md"));
    assert!(!output.contains("https://github.com/org/repo/pull/1"));
    assert!(output.contains("docs/missing.md"));
    assert!(output.contains("\tmissing\t"));
}

#[test]
fn manifest_has_no_link_handlers() {
    let manifest = fs::read_to_string("herdr-plugin.toml").expect("read manifest");
    assert!(
        !manifest.contains("[[link_handlers]]"),
        "manifest must not register link_handlers"
    );
}

#[test]
fn manifest_declares_hint_action_and_panes() {
    let manifest = fs::read_to_string("herdr-plugin.toml").expect("read manifest");
    assert!(manifest.contains("id = \"hint\""));
    assert!(manifest.contains("id = \"hint-overlay\""));
    assert!(manifest.contains("id = \"less\""));
    assert!(manifest.contains("id = \"browse\""));
}

#[test]
fn hint_keys_are_unique_and_exclude_cancel() {
    assert_eq!(HINT_KEYS.len(), 25);
    assert!(!HINT_KEYS.contains('q'));
}

#[test]
fn serialize_matches_format_list() {
    let root = temp_fixture("serialize");
    let cwd = root.join("repo");
    fs::create_dir_all(cwd.join("src")).unwrap();
    fs::write(cwd.join("src/a.rs"), "x\n").unwrap();
    let text = "src/a.rs\n";
    let entries = build_entries(text, &cwd);
    assert!(!entries.is_empty());
    assert_eq!(serialize_entries(&entries), format_list(&entries));
}

#[test]
fn builds_hint_for_path_only_in_visible_worktree() {
    let root = temp_fixture("wt-hint");
    let cwd = root.join("repo");
    let wt = cwd.join(".claude/worktrees/feat-109-explain-the-product");
    fs::create_dir_all(wt.join("context/spec/008-make-product-explain-itself")).unwrap();
    let rel = "context/spec/008-make-product-explain-itself/technical-considerations.md";
    fs::write(wt.join(rel), "# tech\n").unwrap();

    // Relative path appears BEFORE the worktree dir on screen (common agent layout).
    let text = format!(
        "Approval gate ready.\n\n  {rel}\n\n  worktree: {wt_disp}\n",
        wt_disp = wt.display()
    );
    let entries = build_entries(&text, &cwd);
    let hit = entries
        .iter()
        .find(|e| e.raw == rel)
        .expect("relative path in worktree should be hinted");
    match &hit.target {
        herdr_preview::classify::Target::File {
            open_spec, path, ..
        } => {
            assert_eq!(
                open_spec,
                &format!(".claude/worktrees/feat-109-explain-the-product/{rel}")
            );
            assert_eq!(path, &wt.join(rel));
        }
        other => panic!("expected file target, got {other:?}"),
    }
}

#[test]
fn builds_hint_for_worktree_path_even_when_worktree_not_on_screen() {
    let root = temp_fixture("wt-disk");
    let cwd = root.join("repo");
    // Minimal git repo so discover_worktree_roots can also use git; .claude path alone is enough.
    let wt = cwd.join(".claude/worktrees/feat-only-on-disk");
    fs::create_dir_all(wt.join("context/spec")).unwrap();
    let rel = "context/spec/technical-considerations.md";
    fs::write(wt.join(rel), "# tech\n").unwrap();

    let text = format!("Approval gate ready.\n\n  {rel}\n");
    let entries = build_entries(&text, &cwd);
    let hit = entries
        .iter()
        .find(|e| e.raw == rel)
        .expect("disk worktree should rescue the path without it being on-screen");
    match &hit.target {
        herdr_preview::classify::Target::File {
            open_spec, path, ..
        } => {
            assert_eq!(
                path.canonicalize().unwrap(),
                wt.join(rel).canonicalize().unwrap()
            );
            assert_eq!(
                open_spec,
                &format!(".claude/worktrees/feat-only-on-disk/{rel}")
            );
        }
        other => panic!("expected file target, got {other:?}"),
    }
}

#[test]
fn hints_collapsed_path_and_marks_ambiguous_kind() {
    let root = temp_fixture("collapsed-hint");
    let cwd = root.join("repo");
    let a = cwd.join("aaa/xt/spec/009-one-call-per-source-synth/review.md");
    let z = cwd.join("zzz/xt/spec/009-one-call-per-source-synth/review.md");
    fs::create_dir_all(a.parent().unwrap()).unwrap();
    fs::create_dir_all(z.parent().unwrap()).unwrap();
    fs::write(&a, "a\n").unwrap();
    fs::write(&z, "z\n").unwrap();

    let raw = "...xt/spec/009-one-call-per-source-synth/review.md";
    let entries = build_entries(&format!("Read {raw}\n"), &cwd);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].raw, raw);
    match &entries[0].target {
        herdr_preview::classify::Target::File {
            path, ambiguous, ..
        } => {
            assert_eq!(path, &a);
            assert!(ambiguous);
        }
        other => panic!("expected ambiguous file, got {other:?}"),
    }
    let tsv = serialize_entries(&entries);
    assert!(
        tsv.contains("\tfile-warn\t"),
        "warn kind for overlay color: {tsv}"
    );
}

#[test]
fn hints_internal_ellipsis_as_existing_file() {
    let root = temp_fixture("collapsed-middle-hint");
    let cwd = root.join("repo");
    let dest = cwd.join("context/spec/009-one-call-per-source-synth/review.md");
    fs::create_dir_all(dest.parent().unwrap()).unwrap();
    fs::write(&dest, "# review\n").unwrap();

    let raw = "context/spec/…/review.md";
    let entries = build_entries(&format!("Read {raw}\n"), &cwd);
    let hit = entries
        .iter()
        .find(|e| e.raw == raw)
        .expect("internal ellipsis hinted");
    match &hit.target {
        herdr_preview::classify::Target::File {
            path, ambiguous, ..
        } => {
            assert_eq!(path, &dest);
            assert!(!*ambiguous);
        }
        other => panic!("expected unique file, got {other:?}"),
    }
    assert!(serialize_entries(&entries).contains("\tfile\t"));
}

#[test]
fn hints_missing_path_with_missing_kind() {
    let root = temp_fixture("missing-hint");
    let cwd = root.join("repo");
    fs::create_dir_all(&cwd).unwrap();
    let entries = build_entries("open docs/nope.md\n", &cwd);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].key, None);
    assert!(matches!(
        entries[0].target,
        herdr_preview::classify::Target::Missing { .. }
    ));
    let tsv = serialize_entries(&entries);
    assert!(
        tsv.starts_with("-\t"),
        "missing must not take a letter: {tsv}"
    );
    assert!(tsv.contains("\tmissing\t"));
}

/// Agent panes often print the same absolute path twice (e.g. inside `$ ls -la …`
/// and again as a clean follow-up line). Deduping by `raw` left only the first
/// span, so the visible standalone path stayed unhighlighted.
#[test]
fn paints_every_occurrence_of_duplicate_path_with_shared_key() {
    let root = temp_fixture("dup-path");
    let cwd = root.join("repo");
    fs::create_dir_all(cwd.join("docs")).unwrap();
    fs::write(cwd.join("docs/plan.md"), "# plan\n").unwrap();

    let abs = cwd.join("docs/plan.md");
    let abs_s = abs.to_string_lossy();
    let text = format!("$ ls -la {abs_s}\n\n  {abs_s}\n");
    let entries = build_entries(&text, &cwd);

    let hits: Vec<_> = entries.iter().filter(|e| e.raw == abs_s).collect();
    assert_eq!(
        hits.len(),
        2,
        "both occurrences must stay as hint entries, got: {:?}",
        entries
            .iter()
            .map(|e| (e.key, e.start, &e.raw))
            .collect::<Vec<_>>()
    );
    assert_eq!(hits[0].key, hits[1].key);
    assert!(hits[0].key.is_some(), "existing file must get a letter");
    assert!(hits[0].start < hits[1].start);
    assert_eq!(&text[hits[0].start..hits[0].end], abs_s.as_ref());
    assert_eq!(&text[hits[1].start..hits[1].end], abs_s.as_ref());
}

#[test]
fn duplicate_missing_paths_all_get_no_key() {
    let root = temp_fixture("dup-missing");
    let cwd = root.join("repo");
    fs::create_dir_all(&cwd).unwrap();

    let text = "docs/nope.md\n\n  docs/nope.md\n";
    let hits: Vec<_> = build_entries(text, &cwd)
        .into_iter()
        .filter(|e| e.raw == "docs/nope.md")
        .collect();
    assert_eq!(hits.len(), 2);
    assert!(hits.iter().all(|e| e.key.is_none()));
}

#[test]
fn duplicate_paths_on_same_line_keep_both_spans() {
    let root = temp_fixture("dup-same-line");
    let cwd = root.join("repo");
    fs::create_dir_all(cwd.join("docs")).unwrap();
    fs::write(cwd.join("docs/plan.md"), "# plan\n").unwrap();

    let abs = cwd.join("docs/plan.md");
    let abs_s = abs.to_string_lossy();
    let text = format!("{abs_s} then {abs_s}\n");
    let hits: Vec<_> = build_entries(&text, &cwd)
        .into_iter()
        .filter(|e| e.raw == abs_s)
        .collect();
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].key, hits[1].key);
    assert!(hits[0].key.is_some());
}
