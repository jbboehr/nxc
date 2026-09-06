# nxc

Nix for Cniles\
Lex Ferrata\
『鉄律』\
〜ＴＥＴＳＵＲＩＴＳＵ〜

A C/Rust-flavored concrete syntax for Nix with unchanged Nix evaluation semantics.

The first expression subset supports identifiers, integer literals, parentheses,
arithmetic (`+`, `-`, `*`, `/`, and unary `-`), and curried function calls.

For example, `f(1 + 2, x)` converts to native Nix equivalent to `f (1 + 2) x`.
Conversion preserves the expression's structure and leaves evaluation to Nix.

```sh
nix build
./result/bin/nxc check example.nxc
./result/bin/nxc to-nix example.nxc -o example.nix
./result/bin/nxc from-nix example.nix -o converted.nxc
```

Omit `-o` to write conversions to stdout. `check` validates the implemented
syntax subset; it does not evaluate the expression or check whether names exist.
Malformed and unsupported input produces a nonzero exit status with a source
location. Conversions currently discard comments and reformat expressions.

Identifiers retain Nix's hyphens and apostrophes: `a-b` is one identifier, while
`a - b` is subtraction. Use spaces around `/` for division; path expressions are
not supported yet. Comments may use `//`, `#`, or `/* ... */`, and calls may have
a trailing comma. Calls require at least one argument.

Native `#` line comments must use LF or CRLF endings. Bare-CR line comments are
currently rejected by `from-nix`.

Inputs are currently limited to 1 MiB, 1,024 non-trivia tokens, and 128 levels of
parenthesis or semantic-expression nesting. Integer literals range from `0` to
`9223372036854775807`; negative values use unary `-`.
Generated output must fit these limits as well.

The broader syntax below is planned; attrsets, lists, lambdas, `let`, strings,
and paths are not implemented yet.

```nix
# Nix
{ pkgs, lib, config, ... }:
let
  packages = lib.optionals config.dev.enable [
    pkgs.git
    pkgs.ripgrep
  ];
in {
  environment.systemPackages = packages;
}
```

```nix
// nxc
fn({ pkgs, lib, config, ... }) => let {
    packages = lib.optionals(
        config.dev.enable,
        [
            pkgs.git,
            pkgs.ripgrep,
        ],
    );

    yield {
        environment.systemPackages = packages;
    };
}
```

`fn` is optional. Calls such as `f(a, b)` mean ordinary curried Nix application,
equivalent to `f a b`.

Licensed under **AGPL-3.0-only WITH romic-exception**. See [LICENSE.md](LICENSE.md)
and the [Romic Exception](docs/LICENSE_EXCEPTION.md) for the complete terms.
