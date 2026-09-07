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

The current subset implements identifiers, integers, parentheses, arithmetic,
calls, and simple/attribute-pattern lambdas in both conversion directions.
It also includes static attrsets, dotted bindings, inheritance, and selections
with defaults, lists, `let` and `with` expressions, double-quoted/indented strings, and
interpolation. `nxc` provides `check`, `to-nix`, and `from-nix`.
`xtask` provides the corpus runner described in
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
checks generated syntax, precedence, currying, parameter scope, lazy defaults,
argument validation, recursive set merges, local binding and `with` scope, inheritance,
list boundaries/laziness, string coercion/context, and evaluation failures; it skips only when
`nix-instantiate` is unavailable.
Nix is provided in the dev
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

## Corpus coverage

Run against an external directory, such as a nixpkgs checkout:

```sh
cargo xtask corpus /path/to/nixpkgs
cargo xtask corpus /path/to/nixpkgs --filter pkgs/by-name/ --fail-fast
```

The runner discovers regular `.nix` files recursively, skips `.git` directories
and symbolic links below the root, and processes files in sorted path order.
The root itself may be a symlink to a directory. `--filter` is a case-sensitive
literal substring of the path relative to the root; it is not a glob or regex.

Each selected file runs through `Nix → IR₁ → nxc → IR₂ → Nix → IR₃`.
Both generated expressions must preserve the original canonical IR. Source text
is not compared, files are not rewritten, and the runner uses in-process parsers
without evaluating expressions or invoking Nix per file.

Stdout reports discovered, selected, and processed file counts, followed by
success/failure counts for reading, parsing, lowering, emission, and each IR
comparison. A file stops at its first failed stage. Stderr records that failure's
relative path, stage, and first diagnostic (with byte span when available).
Spans refer to the input of the named stage, which may be generated source.

Exit status is zero only if at least one file is selected and every selected file
completes the round trip. Unsupported syntax, malformed input, resource limits,
read failures, and an empty selection return status 1. By default, per-file
failures do not stop later files. `--fail-fast` stops processing after the first
failed file, while discovered/selected counts still describe the full selection.
Discovery errors abort before processing because the file list is incomplete.

Native parse counts include the library's compatibility and resource preflight
checks. Lowering is counted separately, so valid unsupported forms such as
paths and `if` expressions are distinguishable from parse failures.
Coverage is expected to be low until those syntax forms are implemented. Small temporary
corpora in the xtask tests exercise reporting and failure handling; no nixpkgs
checkout is required by the test suite or vendored into this repository.

## License provenance

`LICENSE.md` and `docs/LICENSE_EXCEPTION.md` were copied verbatim from
[`jbboehr/phpstan-array-merge` at `75e0a612`](https://github.com/jbboehr/phpstan-array-merge/tree/75e0a612bd317460e8d79f11a611260d6a5546dd).
The workspace and Nix package use that project's license expression:
`AGPL-3.0-only WITH romic-exception`.

## Frontend boundaries

`syntax/lexer.rs` uses Logos and retains trivia and UTF-8 byte spans. An iterative
mode stack switches between string text and interpolation expressions, tracking
braces within interpolations. Escaped quotes and comment markers inside string text
remain literal; nested strings and comments inside interpolations use their
own lexical rules.
`syntax/parser.rs` uses Chumsky Pratt parsing to construct a temporary expression
tree. `syntax/cst.rs` fills its spans with the original tokens to build an owned
Rowan CST; `syntax/ast.rs` provides the typed expression view used for lowering.
Recovery stops at call-argument/list commas or binding semicolons, skipping nested
delimiter groups. Inside a `let` block it also stops before `yield`, preserving
the result after a malformed binding. Any diagnostic blocks lowering.
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

Lambda IR retains the single parameter, required fields, unevaluated defaults,
ellipsis, and optional whole-argument binding. Parameter spellings (`fn`,
parentheses, and either position of `@`) lower to the same representation.
Duplicate parameter names, including collisions with the whole-argument
binding, are rejected. Default expressions remain in the parameter scope;
conversion does not insert defaults into the captured argument or evaluate them.

Attrset IR retains binding order, dotted paths, recursive flags, and inheritance
sources. Avoid sorting or expanding bindings: when Nix merges literal nested
sets, the first declaration's recursive flag can affect scope. Structural
validation checks static binding conflicts after the entire IR passes resource
bounds. Inheritance remains distinct from assignment to preserve its scope.
Static attribute names have separate validation from variable names; bare names
such as `fn` and `or` are permitted in attribute positions. Quoted and dynamic
attributes remain unsupported.

`Expr::Let` reuses ordered bindings and retains a separate body. Bindings are
neither expanded into assignments nor rewritten as recursive attrset selections;
plain inheritance keeps its outer-scope lookup. The first path component and
inherited names bind variables, so they use variable-name validation even for
`inherit (source)`. Remaining path components use attribute-name validation.
Both binding values and the body pass the shared resource checks.

The nxc parser treats the delimited `let { ... yield ...; }` form as an atom.
Exactly one final `yield` is required. `yield` stays valid as an attribute name
outside that result marker. Native lowering accepts `let ... in ...` and rejects
the legacy `let { body = ...; }` form. Native emission parenthesizes let-expressions
to preserve boundaries in lists, selection defaults, calls, and arithmetic.

`Expr::With` retains separate, unevaluated scope and body expressions. The nxc
parser reuses call-argument parsing and recovery, requiring exactly two expressions
inside `with(...)`, with an optional trailing comma. The delimited form is an
atom. Native lowering preserves `with scope; body`, and emission parenthesizes
the whole expression. Both children pass shared semantic and resource validation.
Conversion performs no scope resolution: lexical bindings keep priority over
`with` attributes, nested contexts retain their lookup order, and unused contexts
stay lazy. Native AST parentheses are unwrapped iteratively so generated output
at the supported nesting limit does not add recursive lowering frames.

Selections retain their full static path and optional lazy default. The parser
uses Nix's simple-expression precedence for `or`; emitters parenthesize fallback
expressions to preserve the IR. Attribute paths are bounded, and dotted bindings
contribute their implicit attrset depth to the semantic nesting limit.

List IR retains ordered, unevaluated elements and nested list boundaries. The
nxc parser consumes each full element expression once and then accepts an
optional comma; generated nxc always separates elements with commas. Native
emission uses whitespace, with non-simple elements already parenthesized by
the expression renderer. The native adapter rejects rnix's bare lambda list
elements before lowering erases parentheses. Brackets count toward delimiter
depth, and list elements pass the shared semantic validation and output limits.

String IR retains decoded literal text and unevaluated interpolation expressions.
Canonical parts contain no empty or adjacent literals; an empty vector represents
an empty string. Both adapters normalize literal parts, and emitters validate
caller-built IR against the same invariant. Interpolations are never folded into
literal text or rewritten as ordinary addition, preserving Nix coercion and
string context.

`string.rs` normalizes raw fragments from both frontends. Double-quoted strings
normalize raw CR/CRLF; indented strings preserve them. Indented normalization
computes indentation before decoding escapes, then strips spaces across literal
and interpolation boundaries. Escapes end indentation measurement even when
they decode to whitespace. Final-line trimming respects escape fragment boundaries.
The normalizer follows Nix's [lexer](https://github.com/NixOS/nix/blob/2.34.8/src/libexpr/lexer.l)
and [indentation rules](https://github.com/NixOS/nix/blob/2.34.8/src/libexpr/include/nix/expr/parser-state.hh),
with native-oracle coverage for blank lines, tabs, newlines, escapes, and context.
It uses rnix's raw parts because rnix's normalization does not reproduce every
native whitespace/escape case.

Both quote styles use the same CST string nodes and canonical IR. The lexer
tracks the quote style in its iterative mode stack. Canonical output uses double
quotes and escapes every literal dollar to preserve interpolation boundaries.
String and interpolation delimiters count toward nesting limits in both paths.
Quoted/dynamic attribute paths remain a later slice.

`nix::parse` returns an owned `nix::Parsed` wrapper with a separate `lower()`
operation, allowing the corpus runner to count parsing and lowering without
parsing twice. rnix types remain private. `nix::import` still performs both steps
for callers that only need the IR.

Diagnostics carry byte spans separately from the IR. The nxc CST retains the
source locations; CLI diagnostics attach the originating file path. Emitters
validate IR supplied by callers and add parentheses conservatively. Paths are
neither resolved nor rewritten. Reserved variable names, `__curPos`, and
`__nxc_*` intrinsics are rejected in expression positions until their semantics
are implemented.

The limits in `lib.rs` are intentionally conservative for this slice. Both
parsers reject excessive source size, token count, or nesting, and emitters
reject output that would exceed the corresponding parser's size/token limits.
Token and nesting limits are checked before collecting lexical diagnostics;
over-budget nxc input produces one limit diagnostic and a flat lossless CST.
Raise these bounds only with coverage for parser, CST, IR, and emitter depth.
