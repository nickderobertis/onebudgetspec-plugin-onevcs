use std::{
    io::{self, Write},
    process::ExitCode,
};

fn main() -> ExitCode {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let response = match args.as_slice() {
        [arg] if arg == "--version" || arg == "-V" => Response::Information(format!(
            "onebudgetspec-plugin-onevcs {}",
            env!("CARGO_PKG_VERSION")
        )),
        [] => Response::Information(help().to_owned()),
        [arg] if arg == "--help" || arg == "-h" => Response::Information(help().to_owned()),
        _ => Response::UsageError,
    };
    let result = match response {
        Response::Information(output) => {
            writeln!(io::stdout().lock(), "{output}").map(|()| ExitCode::SUCCESS)
        }
        Response::UsageError => writeln!(
            io::stderr().lock(),
            "error: unsupported arguments; scaffold performs no work. Try --help."
        )
        .map(|()| ExitCode::from(2)),
    };
    match result {
        Ok(status) => status,
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

enum Response {
    Information(String),
    UsageError,
}

fn help() -> &'static str {
    "onebudgetspec-plugin-onevcs — general onebudgetspec-to-onevcs tool (scaffold)\n\nUsage: onebudgetspec-plugin-onevcs [--help | --version]\n\nOptions:\n  -h, --help     Show this help\n  -V, --version  Show the package version\n\nNo plugin, action or reporting operations are implemented."
}
