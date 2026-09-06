// SPDX-License-Identifier: AGPL-3.0-only WITH romic-exception

use clap::{Parser, Subcommand};
use std::{
    fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    process::ExitCode,
};

#[derive(Parser)]
#[command(
    name = "nxc",
    version,
    about = "A C/Rust-flavored concrete syntax for Nix"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Validate a file against the supported nxc syntax subset.
    Check { file: PathBuf },
    /// Convert nxc to native Nix, writing to stdout by default.
    ToNix {
        file: PathBuf,
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Convert native Nix to nxc, writing to stdout by default.
    FromNix {
        file: PathBuf,
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
}

fn main() -> ExitCode {
    match run(Cli::parse().command) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

fn run(command: Command) -> Result<(), String> {
    let (file, output) = match &command {
        Command::Check { file } => (file, None),
        Command::ToNix { file, output } | Command::FromNix { file, output } => {
            (file, output.as_ref())
        }
    };
    let input = fs::File::open(file).map_err(|e| format!("{}: {e}", file.display()))?;
    let mut bytes = Vec::new();
    input
        .take((nxc::MAX_SOURCE_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("{}: {e}", file.display()))?;
    if bytes.len() > nxc::MAX_SOURCE_BYTES {
        return Err(format!(
            "{}: source exceeds the 1 MiB limit",
            file.display()
        ));
    }
    let source = String::from_utf8(bytes)
        .map_err(|e| format!("{}: input is not UTF-8: {e}", file.display()))?;
    let expr = match command {
        Command::FromNix { .. } => nxc::nix::import(&source),
        _ => nxc::parse_nxc(&source),
    }
    .map_err(|errors| diagnostics(file, &source, &errors))?;

    let converted = match command {
        Command::Check { .. } => return Ok(()),
        Command::ToNix { .. } => nxc::nix::emit(&expr),
        Command::FromNix { .. } => nxc::emit::nxc(&expr),
    }
    .map_err(|error| diagnostics(file, &source, &[error]))?;
    let converted = format!("{converted}\n");
    if converted.len() > nxc::MAX_SOURCE_BYTES {
        return Err(format!(
            "{}: converted output exceeds the 1 MiB limit",
            file.display()
        ));
    }
    // Validate and convert completely before opening the destination.
    if let Some(output) = output {
        fs::write(output, converted).map_err(|e| format!("{}: {e}", output.display()))
    } else {
        io::stdout()
            .lock()
            .write_all(converted.as_bytes())
            .map_err(|e| format!("stdout: {e}"))
    }
}

fn diagnostics(path: &Path, source: &str, errors: &[nxc::Diagnostic]) -> String {
    errors
        .iter()
        .map(|error| {
            let prefix = &source[..error.span.start];
            let line = prefix.bytes().filter(|&b| b == b'\n').count() + 1;
            let column = prefix.rsplit('\n').next().unwrap_or("").chars().count() + 1;
            format!("{}:{line}:{column}: {}", path.display(), error.message)
        })
        .collect::<Vec<_>>()
        .join("\n")
}
