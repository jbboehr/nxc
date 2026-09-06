# nxc

Nix for Cniles\
Lex Ferrata\
『鉄律』\
〜ＴＥＴＳＵＲＩＴＳＵ〜

A C/Rust-flavored concrete syntax for Nix with unchanged Nix evaluation semantics.

The current subset supports identifiers, integer literals, parentheses,
arithmetic (`+`, `-`, `*`, `/`, and unary `-`), curried function calls, and lambdas
with simple or attribute-pattern parameters. Static attrsets and attribute
selections are also supported.

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

Lambdas use `=>`. The forms `x => x + 1`, `(x) => x + 1`, and
`fn(x) => x + 1` all convert to native `x: x + 1`. Multiple arguments use nested
lambdas: `(x => y => x + y)(1, 2)` evaluates to `3` in Nix. Parenthesize a
lambda when calling it or using it as an arithmetic operand.

Attribute patterns keep Nix's lazy defaults (`?`), extra-attribute marker
(`...`), and whole-argument capture (`@`):

```nix
fn({ x, y ? x + 1, ... }) => y
(args@{ x ? 1 }) => args
```

The pattern must be parenthesized; `fn` is optional. Capture may also follow
the pattern, as in `({ x ? 1 }@args) => args`. Captures preserve the supplied
argument, without adding values supplied by defaults. Pattern fields are
comma-separated; a trailing comma is allowed after a field, but not after `...`.

Attrsets use Nix's semicolon-terminated bindings, including `rec`, dotted paths,
and both forms of `inherit`:

```nix
(rec { a = b + 1; b = 2; }).a
{ a.b = 1; a.c = 2; }
{ inherit x; inherit (source) y; }
(fn({ x }) => x + 1)({ x = 2; })
```

Bindings and selections currently accept bare static attribute names, including
attribute names such as `fn`, `yield`, and `or`. Binding conflicts are rejected;
literal nested attrsets merge according to Nix's rules. Values remain lazy, and
`inherit x` retains its enclosing-scope lookup even inside a recursive attrset.

Use `value.a.b` to select an attribute and `value.a or fallback` for a missing
attribute. `or` keeps Nix's tight precedence: `s.f or fallback(x)` means
`(s.f or fallback)(x)`, and `s.a or 2 + 3` means `(s.a or 2) + 3`. Parenthesize
a call or arithmetic expression to use the whole expression as the fallback,
as in `s.a or (fallback(x))`. Existing attributes are returned without evaluating
the fallback; failures while evaluating an existing value are preserved.

Native `#` line comments must use LF or CRLF endings. Bare-CR line comments are
currently rejected by `from-nix`.

Inputs are currently limited to 1 MiB, 1,024 non-trivia tokens, and 128 levels of
parenthesis, brace, or semantic-expression nesting. Integer literals range from
`0` to `9223372036854775807`; negative values use unary `-`.
Attribute paths have at most 128 components, and dotted bindings count toward
semantic nesting. Generated output must fit these limits as well.

The broader syntax below is planned; lists, `let`, strings, quoted/dynamic
attributes, attribute-existence tests (`?`), and paths are not implemented yet.

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
