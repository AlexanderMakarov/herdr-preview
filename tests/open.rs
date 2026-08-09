use herdr_preview::open::{detect_file_viewer, open_file_viewer, open_less, open_url, OpenBackend};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};

static ENV_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

fn temp_fixture(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "herdr-preview-open-{}-{}",
        std::process::id(),
        name
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create fixture dir");
    dir
}

fn write_executable(path: &Path, contents: &str) {
    fs::write(path, contents).expect("write stub");
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).expect("chmod stub");
}

fn stub_log_path(root: &Path) -> PathBuf {
    root.join("invocations.log")
}

fn log_args_stub(log: &Path) -> String {
    format!(
        r#"for arg in "$0" "$@"; do
  printf '%s' "$arg" >>"{}"
  printf '\0' >>"{}"
done
"#,
        log.display(),
        log.display()
    )
}

fn log_args_stub_with_pwd(log: &Path) -> String {
    format!(
        r#"printf 'META:PWD=%s' "$PWD" >>"{}"
printf '\0' >>"{}"
{body}"#,
        log.display(),
        log.display(),
        body = log_args_stub(log)
    )
}

fn bash_stub_header() -> &'static str {
    "#!/bin/bash\n"
}

fn fake_herdr(root: &Path) -> PathBuf {
    let log = stub_log_path(root);
    let stub = root.join("herdr");
    write_executable(
        &stub,
        &format!(
            "{header}log={log:?}\n{body}exit 0\n",
            header = bash_stub_header(),
            log = log.display(),
            body = log_args_stub(&log)
        ),
    );
    stub
}

fn fake_gh(root: &Path) -> PathBuf {
    let log = root.join("gh.log");
    let stub = root.join("gh");
    write_executable(
        &stub,
        &format!(
            r#"#!/bin/bash
echo "gh called: $*" >>{log:?}
exit 1
"#,
            log = log.display()
        ),
    );
    stub
}

fn fake_xdg_open(root: &Path) -> PathBuf {
    let log = stub_log_path(root);
    let stub = root.join("xdg-open");
    write_executable(
        &stub,
        &format!(
            "{header}log={log:?}\n{body}exit 0\n",
            header = bash_stub_header(),
            log = log.display(),
            body = log_args_stub(&log)
        ),
    );
    stub
}

fn fake_less(root: &Path) -> PathBuf {
    let stub = root.join("less");
    write_executable(
        &stub,
        r#"#!/bin/bash
exit 0
"#,
    );
    stub
}

fn read_invocations(log: &Path) -> Vec<Vec<String>> {
    let bytes = fs::read(log).unwrap_or_default();
    let mut invocations = Vec::new();
    let mut current = Vec::new();
    for chunk in bytes.split(|&b| b == 0) {
        if chunk.is_empty() {
            if !current.is_empty() {
                invocations.push(current);
                current = Vec::new();
            }
            continue;
        }
        current.push(String::from_utf8_lossy(chunk).into_owned());
    }
    if !current.is_empty() {
        invocations.push(current);
    }
    invocations
}

fn with_path_only<F: FnOnce()>(root: &Path, f: F) {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|poison| poison.into_inner());
    let old_path = std::env::var_os("PATH");
    let old_herdr = std::env::var_os("HERDR_BIN_PATH");
    std::env::set_var("PATH", root);
    f();
    match old_herdr {
        Some(value) => std::env::set_var("HERDR_BIN_PATH", value),
        None => std::env::remove_var("HERDR_BIN_PATH"),
    }
    match old_path {
        Some(value) => std::env::set_var("PATH", value),
        None => std::env::remove_var("PATH"),
    }
}

#[test]
fn detect_file_viewer_true_when_plugin_listed() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|poison| poison.into_inner());
    let root = temp_fixture("detect-true");
    let herdr = fake_herdr(&root);
    let plugin_list = root.join("herdr-plugin-list");
    write_executable(
        &plugin_list,
        r#"#!/bin/bash
cat <<'EOF'
2 plugins installed:
- herdr-file-viewer (herdr-file-viewer) enabled
- other-plugin (other) enabled
EOF
"#,
    );
    write_executable(
        &herdr,
        &format!(
            r#"#!/bin/bash
if [ "$1" = plugin ] && [ "$2" = list ]; then
  exec "{plugin_list}"
fi
{body}exit 0
"#,
            plugin_list = plugin_list.display(),
            body = log_args_stub(&stub_log_path(&root))
        ),
    );

    assert!(detect_file_viewer(&herdr));
}

#[test]
fn detect_file_viewer_false_when_absent() {
    let root = temp_fixture("detect-false");
    let herdr = fake_herdr(&root);
    write_executable(
        &herdr,
        r#"#!/bin/bash
if [ "$1" = plugin ] && [ "$2" = list ]; then
  printf '%s\n' "1 plugins installed:"
  printf '%s\n' "- other-plugin (other) enabled"
  exit 0
fi
exit 0
"#,
    );

    assert!(!detect_file_viewer(&herdr));
}

#[test]
fn open_file_viewer_summons_fv_with_open_env() {
    let root = temp_fixture("fv-open");
    let log = stub_log_path(&root);
    let herdr = root.join("herdr");
    write_executable(
        &herdr,
        &format!(
            "{header}{body}exit 0\n",
            header = bash_stub_header(),
            body = log_args_stub_with_pwd(&log)
        ),
    );
    let _gh = fake_gh(&root);
    let cwd = root.join("repo");
    fs::create_dir_all(&cwd).unwrap();

    with_path_only(&root, || {
        std::env::set_var("HERDR_BIN_PATH", &herdr);
        open_file_viewer("src/app.rs:42", &cwd).expect("open_file_viewer");
    });

    let invocations = read_invocations(&stub_log_path(&root));
    assert_eq!(invocations.len(), 1);
    let args = &invocations[0];
    assert_eq!(args[0], format!("META:PWD={}", cwd.display()));
    assert_eq!(args[1], herdr.to_string_lossy());
    assert!(args.windows(2).any(|w| w == ["plugin", "pane"]));
    assert!(args.windows(2).any(|w| w == ["pane", "open"]));
    assert!(args.contains(&"--plugin".to_string()));
    assert!(args.contains(&"herdr-file-viewer".to_string()));
    assert!(args.contains(&"--entrypoint".to_string()));
    assert!(args.contains(&"file-viewer".to_string()));
    assert!(args.contains(&"--focus".to_string()));
    assert!(args
        .windows(2)
        .any(|w| w[0] == "--env" && w[1] == "HERDR_FILE_VIEWER_OPEN=src/app.rs:42"));
    assert!(!args.contains(&"--cwd".to_string()));
    assert!(!root.join("gh.log").exists());
}

#[test]
fn open_less_spawns_overlay_with_line() {
    let root = temp_fixture("less-line");
    let herdr = fake_herdr(&root);
    let _less = fake_less(&root);
    let file = root.join("doc.md");
    fs::write(&file, "# hi\n").unwrap();

    with_path_only(&root, || {
        open_less(&file, Some(42), &herdr).expect("open_less");
    });

    let invocations = read_invocations(&stub_log_path(&root));
    assert_eq!(invocations.len(), 1);
    let args = &invocations[0];
    assert!(args.contains(&"--plugin".to_string()));
    assert!(args.contains(&"herdr-preview".to_string()));
    assert!(args.contains(&"--entrypoint".to_string()));
    assert!(args.contains(&"less".to_string()));
    assert!(args.contains(&"--placement".to_string()));
    assert!(args.contains(&"overlay".to_string()));
    assert!(args.contains(&"--focus".to_string()));
    let envs: Vec<_> = args
        .windows(2)
        .filter(|w| w[0] == "--env")
        .map(|w| w[1].as_str())
        .collect();
    assert!(
        envs.iter()
            .any(|e| e.contains(&file.to_string_lossy().to_string())),
        "expected file path in env, got {envs:?}"
    );
    assert!(envs.iter().any(|e| e.contains("42")));
}

#[test]
fn open_less_rejects_directory() {
    let root = temp_fixture("less-dir");
    let herdr = fake_herdr(&root);
    let _less = fake_less(&root);
    let dir = root.join("docs");
    fs::create_dir_all(&dir).unwrap();

    with_path_only(&root, || {
        let err = open_less(&dir, None, &herdr).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
    });

    assert!(!stub_log_path(&root).exists());
}

#[test]
fn open_less_errors_when_less_missing() {
    let root = temp_fixture("less-missing");
    let herdr = fake_herdr(&root);
    let file = root.join("doc.md");
    fs::write(&file, "# hi\n").unwrap();

    with_path_only(&root, || {
        let err = open_less(&file, None, &herdr).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
    });
}

#[test]
fn open_url_uses_browser_helper_only() {
    let root = temp_fixture("url");
    let _gh = fake_gh(&root);
    fake_xdg_open(&root);

    with_path_only(&root, || {
        open_url("https://example.com/pr/1").expect("open_url");
    });

    let invocations = read_invocations(&stub_log_path(&root));
    assert_eq!(invocations.len(), 1);
    assert_eq!(invocations[0][0], root.join("xdg-open").to_string_lossy());
    assert_eq!(invocations[0][1], "https://example.com/pr/1");
    assert!(!root.join("gh.log").exists());
}

#[test]
fn open_backend_enum_is_public() {
    let backend = OpenBackend::FileViewer;
    assert_ne!(backend, OpenBackend::Less);
}
