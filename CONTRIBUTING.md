# Development

Enter the development shell with Nix flakes enabled:

```sh
nix develop
```

The shell provides stable Rust through Fenix, Cargo, rustfmt, Clippy,
rust-analyzer, Rust sources, Nix, and nixfmt. `flake.lock` pins the toolchain and
Nix dependencies; `Cargo.lock` pins the Rust dependencies. Optional direnv
support is provided by `.envrc` (`direnv allow`).

## Workspace

- `crates/nxc`: library for the frontend, semantic IR, and converters.
- `crates/nxc-cli`: the `nxc` executable.
- `xtask`: development tasks, with a `cargo xtask` alias.

The first expression slice implements identifiers, integers, parentheses,
arithmetic, and calls in both conversion directions. `nxc` provides `check`,
`to-nix`, and `from-nix`. `xtask` remains a placeholder for the corpus runner in
[the handoff](docs/HANDOFF.md). All crates currently disable publishing.

```sh
cargo run -p nxc-cli -- --help
cargo xtask --help
```

## Verification

Inside the development shell:

```sh
cargo build --workspace --locked
cargo test --workspace --locked
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
nixfmt --check flake.nix
```

Tests cover source reconstruction, malformed input, argument recovery, semantic
round trips, CLI output and error handling, and resource limits. Proptest checks
arbitrary UTF-8 input and generated semantic expressions. The native Nix oracle
checks generated syntax, precedence, currying, laziness, and evaluation failures;
it skips only when `nix-instantiate` is unavailable. Nix is provided in the dev
shell and package checks. The oracle uses Nix's dummy store so it can run inside
the package build sandbox without a daemon or writable Nix state directory.

From any shell with Nix:

```sh
nix build
nix flake check
./result/bin/nxc --help
```

`nix build` packages only the `nxc` executable and runs workspace tests.
`nix flake check` also checks Rust/Nix formatting and runs Clippy across all
workspace targets. Flake outputs cover x86_64/aarch64 Linux and aarch64 macOS;
checks build on the current system by default.

To evaluate every supported system without attempting foreign builds:

```sh
nix flake check --all-systems --no-build
```

Nix's Git flake source includes only files known to Git. While bootstrapping an
untracked checkout, use `nix develop path:.`, `nix build path:.`, and
`nix flake check path:.`. Once the source files are tracked, ordinary commands
work. Re-run `cargo generate-lockfile` after dependency changes and
`nix flake update` when updating the toolchain or Nix dependencies.

## License provenance

`LICENSE.md` and `docs/LICENSE_EXCEPTION.md` were copied verbatim from
[`jbboehr/phpstan-array-merge` at `75e0a612`](https://github.com/jbboehr/phpstan-array-merge/tree/75e0a612bd317460e8d79f11a611260d6a5546dd).
The workspace and Nix package use that project's license expression:
`AGPL-3.0-only WITH romic-exception`.

## Frontend boundaries

`syntax/lexer.rs` uses Logos and retains trivia and UTF-8 byte spans.
`syntax/parser.rs` uses Chumsky Pratt parsing to construct a temporary expression
tree. `syntax/cst.rs` fills its spans with the original tokens to build an owned
Rowan CST; `syntax/ast.rs` provides the typed expression view used for lowering.
Recovery stops at call-argument separators, and any diagnostic blocks lowering.
Even invalid or unsupported input keeps a lossless CST, except when it exceeds
the source-size limit.

The native path uses rnix exclusively inside `nix/import.rs`. Its Rowan types
stay inside the adapter. Preflight checks reject bare-CR line comments and
whitespace that rnix accepts but native Nix does not. Both paths lower to
`ir::Expr`, where application is
unary and parentheses and source metadata do not participate in equality.
`canonical()` is an identity view for this subset: no constant folding or other
evaluation takes place. In particular, `true`, `false`, and `null` remain variable
references, preserving Nix shadowing behavior.

Diagnostics carry byte spans separately from the IR. The nxc CST retains the
source locations; CLI diagnostics attach the originating file path. Emitters
validate IR supplied by callers and add parentheses conservatively. Paths are
neither resolved nor rewritten. Reserved keywords, `__curPos`, and `__nxc_*`
intrinsics are rejected until their semantics are implemented.

The limits in `lib.rs` are intentionally conservative for this slice. Both
parsers reject excessive source size, token count, or nesting, and emitters
reject output that would exceed the corresponding parser's size/token limits.
Token and nesting limits are checked before collecting lexical diagnostics;
over-budget nxc input produces one limit diagnostic and a flat lossless CST.
Raise these bounds only with coverage for parser, CST, IR, and emitter depth.
