// SPDX-License-Identifier: AGPL-3.0-only WITH romic-exception

mod corpus;

use clap::{Parser, Subcommand};
use std::{
    io::{self, Write},
    path::PathBuf,
    process::ExitCode,
};

#[derive(Parser)]
#[command(name = "cargo xtask", version, about = "Development tasks for nxc")]
struct Xtask {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Measure semantic round-trip coverage of .nix files in a directory.
    Corpus {
        root: PathBuf,
        /// Select relative paths containing this literal substring.
        #[arg(long)]
        filter: Option<String>,
        /// Stop processing after the first failed file (discovery still completes).
        #[arg(long)]
        fail_fast: bool,
    },
}

fn main() -> ExitCode {
    let Command::Corpus {
        root,
        filter,
        fail_fast,
    } = Xtask::parse().command;
    match corpus::run(
        &root,
        filter.as_deref(),
        fail_fast,
        &mut io::stdout().lock(),
        &mut io::stderr().lock(),
    ) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(error) => {
            // The original error may already be a failed stderr write.
            let _ = writeln!(io::stderr().lock(), "{error}");
            ExitCode::FAILURE
        }
    }
}
