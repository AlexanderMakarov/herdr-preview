const VERSION: &str = env!("CARGO_PKG_VERSION");

fn print_usage() {
    eprintln!(
        "herdr-preview {VERSION} — Herdr plugin for path/URL hint preview\n\
         \n\
         Usage:\n\
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
