const VERSION: &str = env!("CARGO_PKG_VERSION");

fn print_usage() {
    eprintln!(
        "herdr-preview {VERSION} — Herdr plugin for path/URL hint preview\n\
         \n\
         Usage:\n\
           herdr-preview hint [--list] [--text TEXT] [--cwd PATH]\n\
           herdr-preview hint-overlay\n\
           herdr-preview browse\n\
           herdr-preview [OPTIONS]\n\
         \n\
         Options:\n\
           -h, --help       Print help\n\
           -V, --version    Print version"
    );
}

fn print_version() {
    println!("herdr-preview {VERSION}");
}

fn run_hint(args: &[String]) -> i32 {
    use herdr_preview::herdr_ipc::read_focused_snapshot;
    use herdr_preview::hint::{run_hint_action, run_hint_list, HintError};
    use std::path::PathBuf;

    let mut list = false;
    let mut text: Option<String> = None;
    let mut cwd: Option<PathBuf> = None;

    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--list" => list = true,
            "--text" => {
                index += 1;
                text = Some(args.get(index).cloned().unwrap_or_else(|| {
                    eprintln!("herdr-preview hint: --text requires a value");
                    std::process::exit(1);
                }));
            }
            "--cwd" => {
                index += 1;
                cwd = Some(args.get(index).map(PathBuf::from).unwrap_or_else(|| {
                    eprintln!("herdr-preview hint: --cwd requires a value");
                    std::process::exit(1);
                }));
            }
            flag => {
                eprintln!("herdr-preview hint: unknown argument `{flag}`");
                return 1;
            }
        }
        index += 1;
    }

    let result = if list {
        if let (Some(text), Some(cwd)) = (text, cwd) {
            run_hint_list(&text, &cwd)
        } else {
            read_focused_snapshot()
                .map_err(HintError::from)
                .and_then(|snapshot| run_hint_list(&snapshot.visible_text, &snapshot.cwd))
        }
    } else if text.is_some() || cwd.is_some() {
        Err(HintError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "--text/--cwd require --list for headless mode",
        )))
    } else {
        run_hint_action().map(|_| String::new())
    };

    match result {
        Ok(output) => {
            if list {
                print!("{output}");
            }
            0
        }
        Err(HintError::NoOpenableTargets) => {
            eprintln!("herdr-preview: no openable targets in visible pane text");
            0
        }
        Err(err) => {
            eprintln!("herdr-preview hint: {err}");
            1
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [] => {
            print_usage();
            std::process::exit(0);
        }
        [flag] if flag == "--help" || flag == "-h" => {
            print_usage();
            std::process::exit(0);
        }
        [flag] if flag == "--version" || flag == "-V" => {
            print_version();
            std::process::exit(0);
        }
        [cmd] if cmd == "hint" => {
            std::process::exit(run_hint(&[]));
        }
        [cmd, rest @ ..] if cmd == "hint" => {
            std::process::exit(run_hint(rest));
        }
        [cmd] if cmd == "hint-overlay" => {
            use herdr_preview::hint::{run_hint_overlay, HintError};
            match run_hint_overlay() {
                Ok(()) => std::process::exit(0),
                Err(HintError::OverlayEnv(msg)) if msg.contains("missing") => {
                    eprintln!("herdr-preview hint-overlay: {msg}");
                    std::process::exit(1);
                }
                Err(err) => {
                    eprintln!("herdr-preview hint-overlay: {err}");
                    std::process::exit(1);
                }
            }
        }
        [cmd] if cmd == "browse" => match herdr_preview::browse::run_browse_overlay() {
            Ok(()) => std::process::exit(0),
            Err(err) => {
                eprintln!("herdr-preview browse: {err}");
                std::process::exit(1);
            }
        },
        _ => {
            eprintln!("herdr-preview: unknown arguments (try --help)");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn it_compiles() {}
}
