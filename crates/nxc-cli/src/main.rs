// SPDX-License-Identifier: AGPL-3.0-only WITH romic-exception

use clap::Parser;

#[derive(Parser)]
#[command(
    name = "nxc",
    version,
    about = "A C/Rust-flavored concrete syntax for Nix",
    after_help = "Project scaffold: parsing and conversion commands are not implemented yet.",
    arg_required_else_help = true
)]
struct Cli {}

fn main() {
    Cli::parse();
}
