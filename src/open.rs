use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenBackend {
    FileViewer,
    Less,
}

pub fn detect_file_viewer(herdr_bin: &Path) -> bool {
    let output = match Command::new(herdr_bin).args(["plugin", "list"]).output() {
        Ok(output) => output,
        Err(_) => return false,
    };

    if !output.status.success() {
        return false;
    }

    let text = String::from_utf8_lossy(&output.stdout);
    text.lines()
        .any(|line| line.contains("herdr-file-viewer") && line.starts_with('-'))
}

fn resolve_herdr_bin() -> PathBuf {
    std::env::var_os("HERDR_BIN_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("herdr"))
}

fn browser_opener() -> &'static str {
    if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    }
}

fn less_available() -> bool {
    Command::new("less")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

pub fn open_url(url: &str) -> io::Result<()> {
    let status = Command::new(browser_opener())
        .arg(url)
        .status()
        .map_err(|err| {
            if err.kind() == io::ErrorKind::NotFound {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("{} not found on PATH", browser_opener()),
                )
            } else {
                err
            }
        })?;

    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "{} exited with {}",
            browser_opener(),
            status
        )))
    }
}

pub fn open_file_viewer(open_spec: &str, cwd: &Path) -> io::Result<()> {
    let herdr_bin = resolve_herdr_bin();
    let env = format!("HERDR_FILE_VIEWER_OPEN={open_spec}");

    let status = Command::new(&herdr_bin)
        .args([
            "plugin",
            "pane",
            "open",
            "--plugin",
            "herdr-file-viewer",
            "--entrypoint",
            "file-viewer",
            "--placement",
            "split",
            "--focus",
            "--cwd",
        ])
        .arg(cwd)
        .args(["--env", &env])
        .status()
        .map_err(|err| {
            if err.kind() == io::ErrorKind::NotFound {
                io::Error::new(io::ErrorKind::NotFound, "herdr not found on PATH")
            } else {
                err
            }
        })?;

    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "herdr plugin pane open exited with {status}"
        )))
    }
}

pub fn open_less(path: &Path, line: Option<u32>, herdr_bin: &Path) -> io::Result<()> {
    if path.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "less cannot open a directory",
        ));
    }

    if !less_available() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "less not found on PATH",
        ));
    }

    let file_env = format!("HERDR_PREVIEW_LESS_FILE={}", path.display());
    let mut cmd = Command::new(herdr_bin);
    cmd.args([
        "plugin",
        "pane",
        "open",
        "--plugin",
        "herdr-preview",
        "--entrypoint",
        "less",
        "--placement",
        "overlay",
        "--focus",
        "--env",
        &file_env,
    ]);

    if let Some(line) = line {
        let line_env = format!("HERDR_PREVIEW_LESS_LINE={line}");
        cmd.args(["--env", &line_env]);
    }

    let status = cmd.status().map_err(|err| {
        if err.kind() == io::ErrorKind::NotFound {
            io::Error::new(io::ErrorKind::NotFound, "herdr not found on PATH")
        } else {
            err
        }
    })?;

    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "herdr plugin pane open exited with {status}"
        )))
    }
}
