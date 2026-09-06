// SPDX-License-Identifier: AGPL-3.0-only WITH romic-exception

use clap::Parser;

#[derive(Parser)]
#[command(
    name = "cargo xtask",
    version,
    about = "Development tasks for nxc",
    after_help = "Project scaffold: the corpus runner is not implemented yet.",
    arg_required_else_help = true
)]
struct Xtask {}

fn main() {
    Xtask::parse();
}
