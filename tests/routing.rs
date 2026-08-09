//! End-to-end routing: `open_entry` picks FV, less, or browser from peer detect.

use herdr_preview::classify::Target;
use herdr_preview::hint::{open_entry, route_entry, HintEntry, OpenRoute, DIR_SKIP_NOTICE};
use herdr_preview::open::detect_file_viewer;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};

static ENV_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

fn temp_fixture(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "herdr-preview-routing-{}-{}",
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
printf '\0' >>"{}"
"#,
        log.display(),
        log.display(),
        log.display()
    )
}

fn bash_stub_header() -> &'static str {
    "#!/bin/bash\n"
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

fn herdr_with_plugin_list(root: &Path, list_body: &str) -> PathBuf {
    let log = stub_log_path(root);
    let herdr = root.join("herdr");
    let plugin_list = root.join("herdr-plugin-list");
    let lines: Vec<_> = list_body
        .lines()
        .filter(|line| !line.is_empty())
        .map(|line| format!("  '{line}'"))
        .collect();
    write_executable(
        &plugin_list,
        &format!(
            "{header}printf '%s\\n' {lines}\n",
            header = bash_stub_header(),
            lines = lines.join(" ")
        ),
    );

    let plugin_root = root.join("fv-plugin");
    let viewer_bin = plugin_root.join("target/release/herdr-file-viewer");
    fs::create_dir_all(viewer_bin.parent().unwrap()).unwrap();
    write_executable(&viewer_bin, "#!/bin/bash\nexit 0\n");
    let list_json = if list_body.contains("herdr-file-viewer") {
        format!(
            r#"{{"result":{{"plugins":[{{"plugin_id":"herdr-file-viewer","plugin_root":"{}"}}]}}}}"#,
            plugin_root.display()
        )
    } else {
        r#"{"result":{"plugins":[]}}"#.to_string()
    };

    write_executable(
        &herdr,
        &format!(
            r#"#!/bin/bash
if [ "$1" = plugin ] && [ "$2" = list ] && [ "${{3:-}}" = --json ]; then
  printf '%s\n' '{list_json}'
  exit 0
fi
if [ "$1" = plugin ] && [ "$2" = list ]; then
  exec "{plugin_list}"
fi
log={log:?}
if [ "$1" = tab ] && [ "$2" = create ]; then
  {body}
  printf '%s\n' '{{"result":{{"root_pane":{{"pane_id":"w1:p1"}}}}}}'
  exit 0
fi
{body}
exit 0
"#,
            plugin_list = plugin_list.display(),
            list_json = list_json,
            log = log.display(),
            body = log_args_stub(&log)
        ),
    );
    herdr
}

fn with_env<F: FnOnce()>(root: &Path, herdr: &Path, f: F) {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|poison| poison.into_inner());
    let old_path = std::env::var_os("PATH");
    let old_herdr = std::env::var_os("HERDR_BIN_PATH");
    std::env::set_var("PATH", root);
    std::env::set_var("HERDR_BIN_PATH", herdr);
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

fn file_entry(path: &Path, open_spec: &str) -> HintEntry {
    HintEntry {
        key: 'a',
        start: 0,
        end: open_spec.len(),
        raw: open_spec.to_string(),
        target: Target::File {
            path: path.to_path_buf(),
            open_spec: open_spec.to_string(),
        },
    }
}

fn dir_entry(path: &Path, open_spec: &str) -> HintEntry {
    HintEntry {
        key: 'a',
        start: 0,
        end: open_spec.len(),
        raw: format!("{}/", path.display()),
        target: Target::Dir {
            path: path.to_path_buf(),
            open_spec: open_spec.to_string(),
        },
    }
}

fn url_entry(url: &str) -> HintEntry {
    HintEntry {
        key: 'a',
        start: 0,
        end: url.len(),
        raw: url.to_string(),
        target: Target::Url(url.to_string()),
    }
}

#[test]
fn fv_listed_routes_file_to_file_viewer_open_env() {
    let root = temp_fixture("fv-file");
    let list = "2 plugins installed:\n- herdr-file-viewer (herdr-file-viewer) enabled\n- other (other) enabled\n";
    let herdr = herdr_with_plugin_list(&root, list);
    let _gh = fake_gh(&root);
    let cwd = root.join("repo");
    fs::create_dir_all(&cwd).unwrap();
    let file = cwd.join("src/app.rs");
    fs::create_dir_all(file.parent().unwrap()).unwrap();
    fs::write(&file, "fn main() {}\n").unwrap();

    with_env(&root, &herdr, || {
        assert!(
            detect_file_viewer(&herdr),
            "stub plugin list should list herdr-file-viewer"
        );
        assert_eq!(
            route_entry(&file_entry(&file, "src/app.rs:42"), &herdr),
            OpenRoute::FileViewer
        );
        open_entry(&file_entry(&file, "src/app.rs:42"), &cwd, None).expect("open_entry");
    });

    let invocations = read_invocations(&stub_log_path(&root));
    assert_eq!(
        invocations.len(),
        2,
        "expected tab create + pane run: {invocations:?}"
    );
    let create = &invocations[0];
    assert!(create.windows(2).any(|w| w == ["tab", "create"]));
    assert!(create
        .windows(2)
        .any(|w| w[0] == "--env" && w[1] == "HERDR_FILE_VIEWER_OPEN=src/app.rs:42"));
    let run = &invocations[1];
    assert!(run.windows(2).any(|w| w == ["pane", "run"]));
    assert!(!invocations
        .iter()
        .any(|args| args.contains(&"less".to_string())));
    assert!(!root.join("gh.log").exists());
}

#[test]
fn fv_absent_routes_file_to_less_overlay() {
    let root = temp_fixture("less-file");
    let list = "1 plugins installed:\n- other (other) enabled\n";
    let herdr = herdr_with_plugin_list(&root, list);
    let _less = fake_less(&root);
    let _gh = fake_gh(&root);
    let cwd = root.join("repo");
    fs::create_dir_all(&cwd).unwrap();
    let file = cwd.join("doc.md");
    fs::write(&file, "# hi\n").unwrap();

    with_env(&root, &herdr, || {
        let entry = file_entry(&file, "doc.md:10");
        assert_eq!(route_entry(&entry, &herdr), OpenRoute::Less);
        open_entry(&entry, &cwd, None).expect("open_entry");
    });

    let invocations = read_invocations(&stub_log_path(&root));
    assert_eq!(invocations.len(), 1, "expected single less overlay summon");
    let args = &invocations[0];
    assert!(args.contains(&"--plugin".to_string()));
    assert!(args.contains(&"herdr-preview".to_string()));
    assert!(args.contains(&"less".to_string()));
    assert!(args.contains(&"overlay".to_string()));
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
    assert!(envs.iter().any(|e| e.contains("10")));
    assert!(!args.contains(&"herdr-file-viewer".to_string()));
    assert!(!root.join("gh.log").exists());
}

#[test]
fn fv_absent_skips_directory_with_notice() {
    let root = temp_fixture("less-dir-skip");
    let list = "1 plugins installed:\n- other (other) enabled\n";
    let herdr = herdr_with_plugin_list(&root, list);
    let _less = fake_less(&root);
    let cwd = root.join("repo");
    let dir = cwd.join("docs");
    fs::create_dir_all(&dir).unwrap();
    let entry = dir_entry(&dir, "docs/");

    with_env(&root, &herdr, || {
        assert_eq!(route_entry(&entry, &herdr), OpenRoute::DirSkip);
        open_entry(&entry, &cwd, None).expect("open_entry");
    });

    let invocations = read_invocations(&stub_log_path(&root));
    assert!(
        invocations.iter().all(|args| {
            args.windows(2).any(|w| w == ["notification", "show"])
                && !args.contains(&"less".to_string())
                && !args.contains(&"herdr-file-viewer".to_string())
        }),
        "directory skip may toast, but must not open less/FV: {invocations:?}"
    );
    assert!(DIR_SKIP_NOTICE.contains("directories need herdr-file-viewer"));
}

#[test]
fn url_routes_to_browser_helper_only() {
    let root = temp_fixture("url-route");
    let list = "2 plugins installed:\n- herdr-file-viewer (herdr-file-viewer) enabled\n";
    let herdr = herdr_with_plugin_list(&root, list);
    let _gh = fake_gh(&root);
    fake_xdg_open(&root);
    let cwd = root.join("repo");
    fs::create_dir_all(&cwd).unwrap();

    with_env(&root, &herdr, || {
        let entry = url_entry("https://github.com/org/repo/pull/1");
        assert_eq!(route_entry(&entry, &herdr), OpenRoute::Browser);
        open_entry(&entry, &cwd, None).expect("open_entry");
    });

    let invocations = read_invocations(&stub_log_path(&root));
    assert_eq!(invocations.len(), 1, "expected single browser invocation");
    assert_eq!(invocations[0][0], root.join("xdg-open").to_string_lossy());
    assert_eq!(invocations[0][1], "https://github.com/org/repo/pull/1");
    assert!(!root.join("gh.log").exists());
}
