use std::{
    io::{self, Write},
    process::ExitCode,
};

fn main() -> ExitCode {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let (output, usage_error) = match args.as_slice() {
        [arg] if arg == "--version" || arg == "-V" => (
            format!("onebudgetspec-plugin-onevcs {}", env!("CARGO_PKG_VERSION")),
            false,
        ),
        [] => (help().to_owned(), false),
        [arg] if arg == "--help" || arg == "-h" => (help().to_owned(), false),
        _ => (
            "error: unsupported arguments; scaffold performs no work. Try --help.".to_owned(),
            true,
        ),
    };
    let result = if usage_error {
        writeln!(io::stderr().lock(), "{output}")
    } else {
        writeln!(io::stdout().lock(), "{output}")
    };
    match result {
        Ok(()) if usage_error => ExitCode::from(2),
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            // Reporting is best-effort when the diagnostic stream itself is broken.
            let _ = writeln!(
                io::stderr().lock(),
                "error: could not write output: {error}; restore the output stream and retry."
            );
            ExitCode::FAILURE
        }
    }
}

fn help() -> &'static str {
    "onebudgetspec-plugin-onevcs — general onebudgetspec-to-onevcs tool (scaffold)\n\nUsage: onebudgetspec-plugin-onevcs [--help | --version]\n\nOptions:\n  -h, --help     Show this help\n  -V, --version  Show the package version\n\nNo plugin, action or reporting operations are implemented."
}
