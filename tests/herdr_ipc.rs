use herdr_preview::herdr_ipc::{read_focused_snapshot, HerdrIpcError, PaneSnapshot};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};

static ENV_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

fn temp_fixture(name: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("herdr-preview-ipc-{}-{}", std::process::id(), name));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create fixture dir");
    dir
}

fn write_executable(path: &Path, contents: &str) {
    fs::write(path, contents).expect("write stub");
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).expect("chmod stub");
}

fn with_herdr_bin<F, T>(herdr: &Path, f: F) -> T
where
    F: FnOnce() -> T,
{
    let _guard = ENV_LOCK.lock().unwrap_or_else(|poison| poison.into_inner());
    let old = std::env::var_os("HERDR_BIN_PATH");
    let old_ctx = std::env::var_os("HERDR_PLUGIN_CONTEXT_JSON");
    std::env::set_var("HERDR_BIN_PATH", herdr);
    std::env::remove_var("HERDR_PLUGIN_CONTEXT_JSON");
    let result = f();
    match old_ctx {
        Some(value) => std::env::set_var("HERDR_PLUGIN_CONTEXT_JSON", value),
        None => std::env::remove_var("HERDR_PLUGIN_CONTEXT_JSON"),
    }
    match old {
        Some(value) => std::env::set_var("HERDR_BIN_PATH", value),
        None => std::env::remove_var("HERDR_BIN_PATH"),
    }
    result
}

const PANE_CURRENT_FIXTURE: &str = r#"{"id":"cli:pane:current","result":{"pane":{"pane_id":"w9:p1","cwd":"/tmp/herdr-repo"}},"type":"pane_current"}"#;

const VISIBLE_TEXT_FIXTURE: &str = "src/main.rs:10: fn main() {\nhttps://example.com/pr/1\n";

#[test]
fn read_focused_snapshot_parses_pane_current_and_visible_text() {
    let root = temp_fixture("happy");
    let herdr = root.join("herdr");
    write_executable(
        &herdr,
        r#"#!/bin/bash
if [ "$1" = pane ] && [ "$2" = current ]; then
  cat <<'EOF'
{"id":"cli:pane:current","result":{"pane":{"pane_id":"w9:p1","cwd":"/tmp/herdr-repo"}},"type":"pane_current"}
EOF
  exit 0
fi
if [ "$1" = pane ] && [ "$2" = read ] && [ "$3" = w9:p1 ] && [ "$4" = --source ] && [ "$5" = visible ] && [ "$6" = --format ] && [ "$7" = text ]; then
  printf '%s' 'src/main.rs:10: fn main() {
https://example.com/pr/1
'
  exit 0
fi
echo "unexpected: $*" >&2
exit 1
"#,
    );

    let snapshot = with_herdr_bin(&herdr, read_focused_snapshot).expect("snapshot");

    assert_eq!(
        snapshot,
        PaneSnapshot {
            pane_id: "w9:p1".to_string(),
            cwd: PathBuf::from("/tmp/herdr-repo"),
            visible_text: VISIBLE_TEXT_FIXTURE.to_string(),
        }
    );
}

#[test]
fn read_focused_snapshot_rejects_invalid_pane_current_json() {
    let root = temp_fixture("bad-json");
    let herdr = root.join("herdr");
    write_executable(
        &herdr,
        r#"#!/bin/bash
if [ "$1" = pane ] && [ "$2" = current ]; then
  printf 'not json\n'
  exit 0
fi
exit 1
"#,
    );

    let err = with_herdr_bin(&herdr, read_focused_snapshot).unwrap_err();
    assert!(matches!(err, HerdrIpcError::InvalidPaneCurrentJson(_)));
}

#[test]
fn read_focused_snapshot_errors_when_pane_id_missing() {
    let root = temp_fixture("no-pane-id");
    let herdr = root.join("herdr");
    write_executable(
        &herdr,
        r#"#!/bin/bash
if [ "$1" = pane ] && [ "$2" = current ]; then
  printf '{"result":{"pane":{"cwd":"/tmp/repo"}}}\n'
  exit 0
fi
exit 1
"#,
    );

    let err = with_herdr_bin(&herdr, read_focused_snapshot).unwrap_err();
    assert!(matches!(err, HerdrIpcError::MissingPaneId));
}

#[test]
fn read_focused_snapshot_never_uses_scrollback_source() {
    let root = temp_fixture("visible-only");
    let log = root.join("invocations.log");
    let herdr = root.join("herdr");
    write_executable(
        &herdr,
        &format!(
            r#"#!/bin/bash
log={log:?}
printf '%s\0' "$@" >>"$log"
if [ "$1" = pane ] && [ "$2" = current ]; then
  cat <<'EOF'
{fixture}
EOF
  exit 0
fi
if [ "$1" = pane ] && [ "$2" = read ]; then
  printf 'visible only\n'
  exit 0
fi
exit 1
"#,
            log = log.display(),
            fixture = PANE_CURRENT_FIXTURE
        ),
    );

    with_herdr_bin(&herdr, || {
        read_focused_snapshot().expect("snapshot");
    });

    let bytes = fs::read(&log).unwrap();
    let args: Vec<String> = bytes
        .split(|&b| b == 0)
        .filter(|chunk| !chunk.is_empty())
        .map(|chunk| String::from_utf8_lossy(chunk).into_owned())
        .collect();

    assert!(args.windows(2).any(|w| w == ["pane", "read"]));
    assert!(args.windows(2).any(|w| w == ["--source", "visible"]));
    assert!(args.windows(2).any(|w| w == ["--format", "text"]));
    for (idx, arg) in args.iter().enumerate() {
        if arg == "--source" {
            assert_ne!(args.get(idx + 1).map(String::as_str), Some("recent"));
            assert_ne!(
                args.get(idx + 1).map(String::as_str),
                Some("recent-unwrapped")
            );
        }
    }
}

#[test]
fn read_focused_snapshot_prefers_plugin_context_over_pane_current() {
    let root = temp_fixture("context");
    let herdr = root.join("herdr");
    write_executable(
        &herdr,
        r#"#!/bin/bash
# pane current would lie — context must win
if [ "$1" = pane ] && [ "$2" = current ]; then
  printf '%s\n' '{"result":{"pane":{"pane_id":"WRONG","cwd":"/wrong"}}}'
  exit 0
fi
if [ "$1" = pane ] && [ "$2" = read ] && [ "$3" = "w9:agent" ]; then
  printf '%s' 'agent pane: docs/plan.md\n'
  exit 0
fi
echo "unexpected: $*" >&2
exit 1
"#,
    );

    let _guard = ENV_LOCK.lock().unwrap_or_else(|poison| poison.into_inner());
    let old = std::env::var_os("HERDR_BIN_PATH");
    let old_ctx = std::env::var_os("HERDR_PLUGIN_CONTEXT_JSON");
    std::env::set_var("HERDR_BIN_PATH", &herdr);
    std::env::set_var(
        "HERDR_PLUGIN_CONTEXT_JSON",
        r#"{"focused_pane_id":"w9:agent","focused_pane_cwd":"/tmp/agent-repo"}"#,
    );
    let snapshot = read_focused_snapshot().expect("snapshot");
    match old_ctx {
        Some(value) => std::env::set_var("HERDR_PLUGIN_CONTEXT_JSON", value),
        None => std::env::remove_var("HERDR_PLUGIN_CONTEXT_JSON"),
    }
    match old {
        Some(value) => std::env::set_var("HERDR_BIN_PATH", value),
        None => std::env::remove_var("HERDR_BIN_PATH"),
    }

    assert_eq!(snapshot.pane_id, "w9:agent");
    assert_eq!(snapshot.cwd, PathBuf::from("/tmp/agent-repo"));
    assert!(snapshot.visible_text.contains("docs/plan.md"));
}

#[test]
fn read_focused_snapshot_prefers_live_foreground_cwd_over_context() {
    let root = temp_fixture("fg-cwd");
    let herdr = root.join("herdr");
    write_executable(
        &herdr,
        r#"#!/bin/bash
if [ "$1" = pane ] && [ "$2" = get ] && [ "$3" = "w9:shell" ]; then
  printf '%s\n' '{"result":{"pane":{"pane_id":"w9:shell","cwd":"/stale","foreground_cwd":"/tmp/live-shell"}}}'
  exit 0
fi
if [ "$1" = pane ] && [ "$2" = read ] && [ "$3" = "w9:shell" ]; then
  printf '%s' 'see Cargo.toml\n'
  exit 0
fi
echo "unexpected: $*" >&2
exit 1
"#,
    );

    let _guard = ENV_LOCK.lock().unwrap_or_else(|poison| poison.into_inner());
    let old = std::env::var_os("HERDR_BIN_PATH");
    let old_ctx = std::env::var_os("HERDR_PLUGIN_CONTEXT_JSON");
    std::env::set_var("HERDR_BIN_PATH", &herdr);
    std::env::set_var(
        "HERDR_PLUGIN_CONTEXT_JSON",
        r#"{"focused_pane_id":"w9:shell","focused_pane_cwd":"/tmp/context-stale"}"#,
    );
    let snapshot = read_focused_snapshot().expect("snapshot");
    match old_ctx {
        Some(value) => std::env::set_var("HERDR_PLUGIN_CONTEXT_JSON", value),
        None => std::env::remove_var("HERDR_PLUGIN_CONTEXT_JSON"),
    }
    match old {
        Some(value) => std::env::set_var("HERDR_BIN_PATH", value),
        None => std::env::remove_var("HERDR_BIN_PATH"),
    }

    assert_eq!(snapshot.pane_id, "w9:shell");
    assert_eq!(snapshot.cwd, PathBuf::from("/tmp/live-shell"));
    assert!(snapshot.visible_text.contains("Cargo.toml"));
}
