use std::path::PathBuf;

fn main() {
    let mut arguments = std::env::args().skip(1);
    let command = arguments.next();
    let mut root = None;
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--root" => {
                root = arguments.next().map(PathBuf::from);
            }
            other => {
                eprintln!("xtask: unknown argument {other}");
                std::process::exit(2);
            }
        }
    }

    let root = root.unwrap_or_else(xtask::default_root);
    match command.as_deref() {
        Some("bless") => match xtask::bless(&root) {
            Ok(outcomes) => {
                xtask::report(&root, &outcomes);
                std::process::exit(0);
            }
            Err(error) => fail(error),
        },
        Some("check-goldens") => match xtask::check(&root) {
            Ok(outcomes) => {
                let passed = xtask::report(&root, &outcomes);
                std::process::exit(if passed { 0 } else { 1 });
            }
            Err(error) => fail(error),
        },
        Some("help" | "--help" | "-h") | None => {
            println!("usage: xtask <command> [--root <dir>]");
            println!();
            println!("commands:");
            println!("  check-goldens   compare tests/golden against the pipeline");
            println!("  bless           rewrite expected.rs for every case");
            if command.is_none() {
                std::process::exit(2);
            }
        }
        Some(other) => {
            eprintln!("xtask: unknown command {other}");
            std::process::exit(2);
        }
    }
}

fn fail(error: String) -> ! {
    eprintln!("xtask: {error}");
    std::process::exit(2);
}
