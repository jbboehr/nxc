# nxc

Nix for Cniles\
Lex Ferrata\
『鉄律』\
〜ＴＥＴＳＵＲＩＴＳＵ〜

A C/Rust-flavored concrete syntax for Nix with unchanged Nix evaluation semantics.

The current subset supports identifiers, integer literals, parentheses,
arithmetic (`+`, `-`, `*`, `/`, and unary `-`), curried function calls, and lambdas
with simple or attribute-pattern parameters. Static attrsets and attribute
selections, lists, `let` expressions, double-quoted and indented strings, and
string interpolation are also supported.

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

Attrset bindings and selections accept bare static attribute names, including
attribute names such as `fn`, `yield`, and `or`. Binding conflicts are rejected;
literal nested attrsets merge according to Nix's rules. Values remain lazy, and
`inherit x` retains its enclosing-scope lookup even inside a recursive attrset.

Local bindings use `let { ... yield ...; }`:

```nix
let {
    a = b + 1;
    b = 2;
    yield a;
}
```

This converts to native `let a = b + 1; b = 2; in a` and evaluates to `3` in
Nix. Bindings are lazy and mutually recursive, so their order does not limit
which variables they can reference. Dotted bindings and both forms of `inherit`
work here too. `yield` selects the result expression: it is required exactly
once, must be the final item, and needs a semicolon. It does not return early.

A `let` block is an expression and can appear directly in calls, lists,
arithmetic, and selections, for example `f(let { yield 1; })` or
`let { yield { a = 1; }; }.a`. The first component of a local binding must be a
supported variable name; `fn`, `yield`, `or`, `__curPos`, and `__nxc_*` remain
reserved there. Nested attribute names keep the attrset rules.

Use `value.a.b` to select an attribute and `value.a or fallback` for a missing
attribute. `or` keeps Nix's tight precedence: `s.f or fallback(x)` means
`(s.f or fallback)(x)`, and `s.a or 2 + 3` means `(s.a or 2) + 3`. Parenthesize
a call or arithmetic expression to use the whole expression as the fallback,
as in `s.a or (fallback(x))`. Existing attributes are returned without evaluating
the fallback; failures while evaluating an existing value are preserved.

Native `#` line comments must use LF or CRLF endings. Bare-CR line comments are
currently rejected by `from-nix`.

Lists support commas, including a trailing comma. Commas may be omitted when
the next token cannot continue the current expression:

```nix
[a, b, f(x),]
[a b f(x)]
[[1, 2], [], [3]]
```

Each element consumes a full expression: `[a - b]` contains one subtraction,
while `[a, -b]` contains two elements. Likewise, `[f (x)]` contains one call;
use `[f, (x)]` for two elements. Generated nxc always includes commas between
elements. Conversion preserves element order, nesting, and lazy evaluation.
Native Nix input keeps its own list rules, including parentheses around calls,
arithmetic, lambdas, and `let ... in ...` used as individual elements.

Double-quoted strings use Nix's escapes and `${...}` interpolation. Expressions
inside interpolations use nxc syntax, including explicit function calls:

```nix
"hello ${name}"
"value=${builtins.toString(42)}"
"${{ value = "nested"; }.value}"
"literal: \${name}"
```

Escapes follow Nix: `\n`, `\r`, and `\t` produce control characters; `\"` and
`\\` produce a quote and backslash. Other escapes discard the backslash, so
`\q` produces `q`. Paired dollar signs remain literal: `"$${name}"` does not
interpolate. Raw CR and CRLF inside strings become LF; escaped CR is preserved.
Strings cannot contain null bytes. Interpolation retains Nix's coercion rules,
string context, and lazy evaluation; conversion does not evaluate expressions.

Indented strings use `''` delimiters and the same interpolation syntax:

```nix
''
  hello ${name}
    this line keeps two spaces
''
```

Indentation follows Nix: common leading spaces are removed from each line,
with blank lines excluded when measuring indentation. An initial line containing
only spaces followed by LF is omitted. Tabs are preserved. Ordinary backslashes
and double quotes are literal; use `'''` for
two single quotes, `''${` for literal `${`, and `''\n`, `''\r`, or `''\t` for
control characters. Raw CR/CRLF is preserved in indented strings. Conversion
currently emits double-quoted strings with the same value, interpolation,
and string context; it does not retain the original quote style.

Inputs are currently limited to 1 MiB, 1,024 non-trivia tokens, and 128 levels of
parenthesis, brace, bracket, string, interpolation, or semantic-expression nesting.
Integer literals range from `0` to `9223372036854775807`; negative values use unary `-`.
Attribute paths have at most 128 components, and dotted bindings count toward
semantic nesting. Generated output must fit these limits as well.

Quoted/dynamic attributes, attribute-existence tests (`?`), paths, `if`, `with`,
and `assert` are not implemented yet. The older native `let { body = ...; }`
syntax is also unsupported.

A complete conversion example:

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
