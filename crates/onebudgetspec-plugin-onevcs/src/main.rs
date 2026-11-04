use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    match args.as_slice() {
        [arg] if arg == "--version" || arg == "-V" => {
            println!("onebudgetspec-plugin-onevcs {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        [] => {
            help();
            ExitCode::SUCCESS
        }
        [arg] if arg == "--help" || arg == "-h" => {
            help();
            ExitCode::SUCCESS
        }
        _ => {
            eprintln!("error: unsupported arguments; scaffold performs no work. Try --help.");
            ExitCode::from(2)
        }
    }
}

fn help() {
    println!(
        "onebudgetspec-plugin-onevcs — general onebudgetspec-to-onevcs tool (scaffold)\n\nUsage: onebudgetspec-plugin-onevcs [--help | --version]\n\nOptions:\n  -h, --help     Show this help\n  -V, --version  Show the package version\n\nNo plugin, action or reporting operations are implemented."
    );
}
