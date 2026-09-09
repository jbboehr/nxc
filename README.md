# nxc

Nix for Cniles\
Lex Ferrata\
『鉄律』\
〜ＴＥＴＳＵＲＩＴＳＵ〜

A C/Rust-flavored concrete syntax for Nix with unchanged Nix evaluation semantics.

The current subset supports identifiers, integer and floating-point literals,
parentheses, arithmetic, comparison and Boolean operators, curried function calls, and lambdas
with simple or attribute-pattern parameters. Attrsets, attribute selections and
existence checks, lists, `let`, `with`, `if`, and `assert` expressions,
double-quoted and indented strings, string interpolation, search paths, and
relative, absolute, and home-relative paths are also supported.

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
`a - b` is subtraction. Use spaces around `/` for division: `1 / 2` divides,
while `1/2` is a relative path. Comments may use `//`, `#`, or `/* ... */`, and
calls may have a trailing comma. Calls require at least one argument.

Floating-point literals include a decimal point: `1.0`, `.5`, `2.`, and
`2.5e-3`. Use unary `-` for negative values. Conversion may change a literal's
spelling while preserving its binary64 value and its floating-point type;
`1.0` remains distinct from the integer `1`. Arithmetic is left to Nix.

Operators follow [Nix precedence](https://nix.dev/manual/nix/2.34/language/operators).
Calls and selections bind more tightly than the following groups, listed from
tightest to loosest:

| Operators | Meaning |
| --- | --- |
| unary `-` | Arithmetic negation |
| `++` | List concatenation |
| `*`, `/` | Multiplication, division |
| `+`, `-` | Addition, subtraction |
| `!` | Boolean negation |
| `<`, `<=`, `>`, `>=` | Ordering comparisons |
| `==`, `!=` | Equality, inequality |
| `&&` | Boolean AND |
| `||` | Boolean OR |

Arithmetic and Boolean binary operators associate to the left. Comparisons in
the same group require parentheses when nested: `a < b < c` and `a == b != c`
are errors. Use `a < b && b < c` to combine two comparisons.
`!a + b` means `!(a + b)`, while `!a == b` means `(!a) == b`.
List concatenation associates to the right: `a ++ b ++ c` means `a ++ (b ++ c)`.

`&&` and `||` preserve Nix's short-circuit evaluation: `false && (1 / 0)` is
`false`, and `true || (1 / 0)` is `true`. Conversion leaves operand type checks,
collection comparisons, and evaluation failures to Nix. For example:

```nix
assert(enabled && count > 0, packages)
```

Native implication `a -> b` imports as `(!a) || b`, preserving the original
grouping and skipping `b` when `a` is false. Use `!` and `||` in nxc; `->` is
reserved for future type syntax. In native input, separate an identifier from
`->`: `a->b` means `a- > b` because identifiers can end in a hyphen.

Lambdas use `=>`. The forms `x => x + 1`, `(x) => x + 1`, and
`fn(x) => x + 1` all convert to native `x: x + 1`. Multiple arguments use nested
lambdas: `(x => y => x + y)(1, 2)` evaluates to `3` in Nix. Parenthesize a
lambda when calling it or using it as an operator operand.

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
attribute names such as `fn`, `yield`, and `or`. Double quotes allow names such
as `"foo.bar"`, `"a b"`, `"if"`, and `""`. For example,
`{ "foo.bar" = 1; }."foo.bar"` evaluates to `1`; the dot inside the name does
not create a nested attribute. Quoted names use the same escapes as strings,
and also work in `inherit (source) "a b";`. Conversion may remove quotes when
the decoded name is a valid bare attribute name. Binding conflicts are rejected;
literal nested attrsets merge according to Nix's rules. Values remain lazy, and
`inherit x` retains its enclosing-scope lookup even inside a recursive attrset.

Selections also accept dynamic keys, using the same syntax as Nix:

```nix
let {
    name = "x";
    values = { x.answer = 42; "prefix-x" = 7; };
    yield [values.${name}.answer, values."prefix-${name}" or 0];
}
```

`${expression}` requires a string key without store-path context. Quoted
interpolation performs Nix's usual string coercion first. Conversion preserves
this distinction and may print `values."${name}"` as `values.${"${name}"}`.
The `or` default applies to the whole path; a missing component skips any
remaining key expressions. Conversion does not evaluate keys or defaults.

Use `?` to test whether an attribute path exists:

```nix
values ? settings.theme
values ? ${name}.enabled
```

The result is a Boolean. A missing component or a non-set intermediate value
returns `false`; an existing final attribute returns `true` without evaluating
its value. Nix evaluates intermediate values as needed and skips later key
expressions after a missing component. Dynamic keys follow the same string rules
as selections. Conversion does not evaluate the set or its keys.

Bindings also accept dynamic names and mixed dotted paths:

```nix
{ ${name} = value; "prefix-${name}".answer = 42; }
```

Nix evaluates computed names when constructing the set; binding values remain
lazy. A direct `null` key omits that entry without evaluating its remaining path
or value. Computed keys that collide produce an evaluation error. Direct literal
keys such as `${"x"}` follow the same scope and conflict rules as `x`, while
`"${"x"}"` remains computed and does not introduce a variable into recursive scope.
Escapes in a direct indented-string key can also make it computed in Nix;
conversion preserves this distinction, sometimes using quoted interpolation.

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
`let { yield { a = 1; }; }.a`. Local bindings can use quoted names too:
`let { "a b" = 1; yield { inherit "a b"; }; }`. The names `or`, `__curPos`, and
`__nxc_*` remain reserved as local binding keys and in plain inheritance,
including when quoted.
The first component of a `let` binding must be a static name, including a direct
literal such as `${"x"}`. Later components can be computed: `a.${name} = value;`.
Inheritance also requires static names and accepts `inherit ${"x"};`.

Native variables named `fn` or `yield` use temporary compatibility spellings in
nxc because those words have syntax roles:

| Native name | nxc variable or parameter |
| --- | --- |
| `fn` | `__nxc_ident_fn` |
| `yield` | `__nxc_ident_yield` |

These spellings also work in attribute patterns and `@` captures. Attribute keys
keep their literal names: `({ __nxc_ident_fn }) => __nxc_ident_fn` accepts an
argument with an attribute named `fn`. Quote a `yield` key at the start of a
local binding to distinguish it from the result marker:

```nix
let {
    fn = x => x + 1;
    "yield" = __nxc_ident_fn(2);
    yield __nxc_ident_yield;
}
```

This converts to native `let fn = x: x + 1; yield = fn 2; in yield`.
Selections such as `value.fn` and literal keys such as `"__nxc_ident_fn"` keep
their exact names. Conversion may add quotes around `yield` keys. These
compatibility spellings are temporary; general escaped-identifier syntax is
not yet defined.

Use `with(context, expression)` to make attributes from a context available
inside an expression:

```nix
with({ x = 2; }, x + 1)
```

This converts to native `with { x = 2; }; x + 1` and evaluates to `3` in Nix.
Lexical bindings take priority over context attributes; nested `with` expressions
prefer the inner context. An unused context remains unevaluated. The form
requires exactly two expressions and permits a trailing comma. It can appear
directly in other expressions, such as `[with(pkgs, git), with(pkgs, ripgrep)]`.

Conditionals keep native Nix syntax:

```nix
if enabled then start(service) else fallback
if (enabled) then 1 else 2
```

Both branches are required. Nix evaluates the condition as a Boolean and then
evaluates only the selected branch; for example, `if true then 1 else 1 / 0`
evaluates to `1`. Conversion retains all three expressions and performs no
evaluation or type checking.

The final branch extends to the right: `if c then a else b + 1` adds only in the
`else` branch. Parenthesize the whole conditional when calling its result, selecting
an attribute, using it in arithmetic, or supplying a selection default:
`(if c then f else g)(x)` or `s.a or (if c then 1 else 2)`.
Conditionals can appear directly as nxc list elements and call arguments, as in
`[if c then 1 else 2, 3]` and `f(if c then 1 else 2)`.

Use `assert(condition, expression)` to require a condition before evaluating an
expression:

```nix
assert(enabled, start(service))
```

This converts to native `assert enabled; start service`. Nix evaluates the body
only if the condition is `true`; `false` raises an assertion failure, and a
non-Boolean condition raises a type error. An unused assertion stays unevaluated.
The form requires exactly two expressions, permits a trailing comma, and can
appear directly in other expressions, such as `assert(enabled, { x = 1; }).x`.
Conversion preserves both expressions without evaluating or type-checking them.

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
operators, lambdas, conditionals, `let ... in ...`, and `with ...; ...` used as
individual elements.

`++` concatenates lists: `[1, 2] ++ [3]` evaluates to `[1, 2, 3]`. Nix evaluates
both list operands while leaving their elements lazy; nested lists stay nested.

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

Literal relative paths retain their spelling, including `./foo`, `../foo`,
`foo/bar`, and `./a/../b`. They work in calls such as `import(./module.nix)`;
conversion does not require the referenced file to exist. Evaluation resolves
them relative to the generated Nix file, so keep generated files beside their
input when existing relative references should address the same files.
Generated paths are parenthesized to preserve token boundaries: `-./foo` is a
path, whereas `-(./foo)` is negation. Paths cannot have trailing slashes or empty
components. Paths starting with `...` require an explicit `./` prefix, such as
`./.../foo`.

Literal absolute paths such as `/etc/nixos/configuration.nix` work in both
directions, including `import(/etc/nixos/configuration.nix)`. Conversion retains
their spelling, including `.` and `..` components, without accessing the target.
They keep the same target when the generated Nix file moves. Use `/.` for the
root directory; a bare `/`, trailing slashes, and empty components are invalid.
Use spaces around division: `1 / 2` divides, while native Nix reads `1 /2` as
function application to the absolute path `/2`, written `1(/2)` in nxc.

Home-relative paths such as `~/project/default.nix` also work in both directions,
including `import(~/project/default.nix)`. Conversion preserves `~/` and all path
components without reading the home directory or requiring the target to exist.
Nix expands `~` when it reads the generated expression, so use the same home
environment when original and generated files should address the same targets.
Use `~/.` to refer to the home directory; bare `~`, bare `~/`, and named-user forms
such as `~alice/file` are invalid. Nix rejects home-relative paths in
[pure evaluation](https://nix.dev/manual/nix/2.34/language/syntax#path), including
paths in unused branches.

Relative, absolute, and home-relative paths can contain `${...}` interpolation:

```nix
let {
    name = "example";
    yield import(./packages/${name}/default.nix);
}
```

These expressions remain paths, and conversion preserves the literal fragments
and embedded expressions. Nix evaluates the interpolation and resolves the path
when needed. Keep relative paths in the same source directory when evaluating a
round trip. Conversion does not require targets to exist. As with native Nix,
there must be a slash before the first interpolation, and a literal trailing slash
is invalid: use `./${name}`, not `${name}/file` or `./${name}/`.
Search paths do not support interpolation. Empty literal path components (`//`)
and relative paths starting with `...` remain unsupported. When applying an
interpolated path to a home-relative path, native input needs whitespace between
them, such as `./${name} ~/file`.

Search paths such as `<nixpkgs>` and `<nixpkgs/lib>` work in both conversion
directions, including calls such as `import(<nixpkgs>)`. Conversion preserves the
lookup name verbatim and does not resolve it or require its target to exist.
Nix resolves the generated lookup when it is evaluated, using that evaluation's
search path. Keep the same search-path environment when the original and
generated files should find the same targets. Components may contain ASCII
letters, digits, `.`, `_`, `-`, and `+`; empty components and interpolation are
not allowed.

Inputs are currently limited to 32 MiB, 4,194,304 non-trivia tokens, and 128 levels of
parenthesis, brace, bracket, string, interpolation, or semantic-expression nesting.
Integer literals range from `0` to `9223372036854775807`; negative values use unary `-`.
Float overflow and inexact subnormal literals are rejected. Exact subnormal
values are supported, and their emitted decimal spellings can be long.
Spellings just below the smallest normal value that round up to it are also
rejected to avoid depending on native libc's underflow-boundary behavior.
Attribute paths have at most 128 components, and dotted bindings count toward
semantic nesting. Generated output must fit these limits as well, including the
CLI's final newline. Limit errors identify the resource, observed count, and
allowed count. Both frontends return at most 100 diagnostics for malformed input.
Library callers can select smaller byte/token budgets with `Limits` and the
`*_with_limits` entry points.

Native attrset updates (`//`) round-trip through the reserved internal form
`__nxc_update(a, b)`. This is converter compatibility syntax; the public update
syntax is still undecided. `//` remains a line comment in nxc.

Computed inheritance names are not implemented yet.
The older native `let { body = ...; }` syntax is also unsupported.
Native import currently requires parentheses around `!` expressions
nested inside arithmetic or concatenation, such as `-(!x)` or `a ++ (!b)`.

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
