# Corpus capacity

The byte/token ceiling increase keeps the 128-level nesting guard. It supports
wide generated files without increasing the recursive lowering depth.

The nixpkgs checkout used for measurement was the pinned source at
`/nix/store/10lgvzyi60fmfdn0svsifazgwq4kcclh-source`. Its largest rejected file,
`pkgs/development/haskell-modules/hackage-packages.nix`, contained 16,633,890
bytes and 1,965,376 non-trivia native tokens, with delimiter depth 6. All 24
native preflight failures under the old budgets exceeded the 16,384-token cap;
the maximum delimiter depth among the 33 resource-limited files was 11.

With 32 MiB and 4,194,304-token ceilings, the release build completed 44,490 of
44,497 full canonical Nix → nxc → Nix round trips, up from 44,459. It read and
parsed every source file. Five files still failed lowering because of `__curPos`,
and two exceeded semantic depth 128. There were no generated parse/lower failures
or canonical IR mismatches.

The full run took 27.18 seconds with peak resident memory of 782,084 KiB on
x86_64 Linux, while workspace tests ran concurrently. These figures describe one
measurement, not a memory or performance guarantee. The corpus runner parses
and compares expressions; it does not evaluate nixpkgs or build packages.

Repeat measurements after relevant changes:

```sh
cargo build --release --workspace --locked
env time -v target/release/xtask corpus /path/to/nixpkgs
```

Use a platform-appropriate process measurement tool if GNU time is unavailable.
Record the source revision, selected files, command, wall time, maximum resident
memory, stage counts, and each failure. Input size alone does not bound tree
memory tightly; include broad sets, lists, deeply nested expressions, and
malformed inputs when assessing a future increase.

`Limits` may reduce byte/token budgets but cannot raise the supported ceilings
or nesting guard. Conversion callers must supply the same budget to parsing and
emission when they want one policy throughout a pipeline. Each emitter also
checks its own output; source acceptance does not imply emitted punctuation and
escaping fit the chosen budget.

Per-file failures identify the first failing stage. Raising a budget can expose
later unsupported syntax, so rerun the complete corpus rather than counting old
limit failures as automatically recovered. Source-location behavior such as
`__curPos` remains a separate compatibility task.
