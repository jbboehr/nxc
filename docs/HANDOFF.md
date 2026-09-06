# `nxc` — Scaffold Handoff

**Nix for Cniles**
**Lex Ferrata**
**『鉄律』**
**〜ＴＥＴＳＵＲＩＴＳＵ〜**

## Goal

Scaffold `nxc`, a Rust frontend for the Nix language with a more C/Rust-like concrete syntax.

The overriding design constraint is:

> **Change syntax, not Nix semantics.**

An `nxc` program should lower mechanically to ordinary Nix and ultimately be evaluated by the real Nix evaluator. Do not introduce a custom runtime, different evaluation rules, imperative semantics, eager evaluation, true multi-argument functions, or otherwise “improve” Nix semantics.

Plan for **bidirectional conversion from the beginning**:

```text
.nxc ──parse──► semantic Nix IR ──emit──► .nix

.nix ──rnix───► semantic Nix IR ──emit──► .nxc
```

This is important because we want to use the entire nixpkgs source tree as a corpus:

```text
Nix
 ↓
IR₁
 ↓
nxc
 ↓
IR₂
 ↓
Nix
 ↓
IR₃

IR₁ == IR₂ == IR₃
```

The initial scaffold does **not** need to achieve 100% nixpkgs coverage. It does need to make corpus round-tripping a first-class test harness so coverage can be driven toward 100% incrementally.

---

# 1. Keep the implementation small

Do not overengineer the initial repository.

Use a small Cargo workspace:

```text
nxc/
├── Cargo.toml
├── Cargo.lock
├── README.md
├── flake.nix
├── crates/
│   ├── nxc/
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── syntax/
│   │       │   ├── mod.rs
│   │       │   ├── kind.rs
│   │       │   ├── lexer.rs
│   │       │   ├── parser.rs
│   │       │   ├── cst.rs
│   │       │   └── ast.rs
│   │       ├── ir/
│   │       │   └── mod.rs
│   │       ├── nix/
│   │       │   ├── mod.rs
│   │       │   ├── import.rs
│   │       │   └── emit.rs
│   │       └── emit/
│   │           └── nxc.rs
│   └── nxc-cli/
│       └── src/main.rs
└── xtask/
    └── src/main.rs
```

Do not split every layer into a separate crate yet. Split later only if there is a demonstrated compile/API boundary worth enforcing.

---

# 2. Parser stack

Own the `nxc` lexer and grammar.

Use:

* **Logos** for lexing.
* **Chumsky** for parsing.
* Chumsky's **Pratt parser** support for expressions/operators.
* Chumsky recovery facilities for malformed/incomplete input.
* **Rowan** for a lossless CST.
* **rnix** only for parsing native `.nix` input and as a reference/oracle for native Nix behavior.

Do **not** fork or adapt rnix into the nxc parser.

The intended flow is:

```text
nxc source
    ↓
Logos
    ↓
tokens + spans + trivia
    ↓
Chumsky grammar / Pratt expressions
    ↓
Rowan lossless CST
    ↓
typed nxc AST
    ↓
semantic Nix IR
```

Native Nix takes a separate path:

```text
native Nix
    ↓
rnix
    ↓
rnix AST adapter
    ↓
same semantic Nix IR
```

Do not couple our Rowan CST types to rnix's Rowan types. The IR is the boundary.

Preserve whitespace and comments in the nxc CST even though the initial semantic IR may ignore them.

Parser errors should recover where practical, particularly at:

```text
call arguments   → ',' or ')'
lists            → ',' or ']'
attrsets         → ';' or '}'
let blocks       → ';', 'yield', or '}'
parameters       → ',' or ')'
```

Do not spend excessive effort on perfect diagnostics in the scaffold. Establish the architecture.

---

# 3. Semantic IR

The IR represents **Nix semantics**, not either source syntax.

It should be possible for both native Nix and nxc syntax to lower into the same structures.

Use a reasonably direct model along these lines:

```rust
enum Expr {
    Literal(Literal),
    Variable(Name),

    String(StringExpr),
    Path(PathExpr),

    List(Vec<Expr>),

    AttrSet {
        recursive: bool,
        bindings: Vec<Binding>,
    },

    Let {
        bindings: Vec<Binding>,
        body: Box<Expr>,
    },

    Lambda {
        parameter: Pattern,
        body: Box<Expr>,
    },

    Apply {
        function: Box<Expr>,
        argument: Box<Expr>,
    },

    Select {
        value: Box<Expr>,
        path: AttrPath,
        default: Option<Box<Expr>>,
    },

    HasAttr {
        value: Box<Expr>,
        path: AttrPath,
    },

    If {
        condition: Box<Expr>,
        then_expr: Box<Expr>,
        else_expr: Box<Expr>,
    },

    Assert {
        condition: Box<Expr>,
        body: Box<Expr>,
    },

    With {
        scope: Box<Expr>,
        body: Box<Expr>,
    },

    Unary { ... },
    Binary { ... },

    // Add other native Nix semantic forms as coverage requires.
}
```

This sketch is guidance, not an API mandate. Match native Nix constructs accurately.

Important:

* **Function application is unary in the IR.**
* Parentheses do not need a semantic IR node.
* Source spans/trivia/origin metadata must not participate in semantic equality.
* Keep enough source-origin information outside semantic equality to support diagnostics and eventual path rewriting.
* Do not desugar constructs in a way that changes laziness or failure behavior.

The corpus tests should compare a canonical semantic representation, not syntax trees.

---

# 4. Current nxc syntax

These are the current working decisions.

## Function application

Whitespace application is removed.

```nix
f(a)
f(a, b)
f(a, b, c)
```

lowers exactly to:

```nix
f a
f a b
f a b c
```

Therefore:

```nix
f(a, b)
```

means nested unary application:

```text
Apply(
    Apply(f, a),
    b,
)
```

It does **not** introduce multi-argument function semantics.

Partial application remains normal Nix behavior.

The nxc emitter should normally flatten a left-associated application chain:

```text
Apply(Apply(Apply(f, a), b), c)
```

to:

```nix
f(a, b, c)
```

Nested argument applications remain nested:

```nix
f(g(x), y)
```

---

## Lambdas

Native Nix `:` lambda syntax is **not part of nxc**.

Use:

```nix
x => expr
(x) => expr
```

and optionally:

```nix
fn(x) => expr
```

`fn` is optional for all lambdas.

The parser should therefore accept both:

```nix
x => x + 1
fn(x) => x + 1
```

with identical semantics.

Pattern lambdas:

```nix
({ pkgs, lib, config, ... }) => expr
```

or:

```nix
fn({ pkgs, lib, config, ... }) => expr
```

No compatibility parsing of:

```nix
x: expr
```

This is deliberate.

Reserve `:` for named arguments and future type annotations.

---

## Named-argument sugar

Support the following call form:

```nix
foo(
    name: "hello",
    enable: true,
)
```

as pure sugar for a **single attrset argument**:

```nix
foo({
    name = "hello";
    enable = true;
})
```

and therefore native Nix:

```nix
foo {
    name = "hello";
    enable = true;
}
```

This does **not** add real named parameters.

Do not allow positional and named arguments to mix in one argument list for now.

Thus:

```nix
foo(a, b)
```

means two successive applications.

```nix
foo(a: 1, b: 2)
```

means one application whose argument is one attrset.

Future type syntax must not make call-site `:` ambiguous: type annotations are only valid in declaration/pattern positions, not ordinary call expressions.

---

## Lists

Support commas:

```nix
[
    a,
    b,
    f(x),
]
```

Also allow commas to be omitted when the expression grammar makes the boundary unambiguous:

```nix
[
    a
    b
    f(x)
]
```

Use maximal-expression parsing.

For example:

```nix
[
    a - b
]
```

is one element.

To mean `a` followed by `-b`, require the comma:

```nix
[
    a,
    -b,
]
```

Canonical generated nxc should use commas.

---

## Attrsets

Keep Nix's existing JSON-ish brace syntax:

```nix
{
    name = "foo";
    enable = true;
}
```

A bare `{ ... }` is always an attrset.

Do **not** make bare braces into general-purpose statement/expression blocks.

Keep current Nix attrset constructs unless explicitly replaced later:

```nix
rec { ... }

foo.bar = value;

inherit foo bar;
inherit (source) foo bar;

foo.${name}

{
    ${name} = value;
}
```

Do not introduce `{ foo, bar }` shorthand yet. It has been discussed but is not decided.

---

## `let`

Replace native:

```nix
let
    a = b;
    c = d;
in
    expr
```

with:

```nix
let {
    a = b;
    c = d;

    yield expr;
}
```

`yield`:

* is required;
* occurs exactly once;
* must be the final item;
* requires a semicolon;
* has no early-return/control-flow semantics;
* merely selects the body/result expression.

Bindings retain ordinary Nix `let` semantics:

* lazy;
* mutually recursive;
* order-independent semantically.

Example:

```nix
let {
    a = b + 1;
    b = 2;

    yield a;
}
```

is valid and lowers to:

```nix
let
    a = b + 1;
    b = 2;
in
    a
```

Keep bare `{ ... }` exclusively for attrsets; the leading `let` is intentionally the semantic discriminator.

---

## `if`

Keep native Nix expression syntax for now:

```nix
if condition then a else b
```

Do not invent C-style statement blocks.

Parenthesized conditions may be accepted naturally if ordinary parenthesized expressions already work:

```nix
if (condition) then a else b
```

but do not make parentheses mandatory.

---

## `assert`

Working syntax:

```nix
assert(condition, expression)
```

lowering to native:

```nix
assert condition; expression
```

Treat this as a special form, not an ordinary eager function call.

---

## `with`

Working syntax:

```nix
with(context, expression)
```

lowering to native:

```nix
with context; expression
```

Again, special form; preserve Nix scoping semantics.

---

## Comments

We want normal C/Rust line comments:

```nix
// comment
```

Also continue to accept:

```nix
# comment

/* block comment */
```

This means native Nix's attrset-update operator:

```nix
a // b
```

cannot retain that public spelling in nxc.

**The replacement public syntax is not decided yet. Do not invent one and freeze it.**

For bidirectional corpus conversion, use a clearly reserved internal compatibility form, for example:

```nix
__nxc_update(a, b)
```

whose only purpose is to represent the native `//` operation losslessly until its public nxc spelling is decided.

Reserve the `__nxc_` prefix for compiler/converter intrinsics.

Generated corpus nxc may contain these internal forms. Normal user-facing examples/docs should not present them as settled language syntax.

---

## Boolean implication / `->`

Native Nix has:

```nix
a -> b
```

for Boolean implication.

Do not expose this spelling in nxc.

Reserve:

```text
->
```

for future function return/type syntax, for example eventually:

```nix
fn(x: Int) -> String => ...
```

When importing native Nix, implication may be normalized to the semantically equivalent short-circuit form:

```nix
!a || b
```

provided this is verified to preserve Nix behavior.

Do not implement type syntax yet.

---

## Future type syntax reservations

Types are deliberately out of scope for the scaffold, but avoid closing off these forms:

```nix
let {
    x: Int = 1;
    yield x;
}

(x: Int) => x + 1

fn(x: Int) -> String => ...

List<String>
String | Path
```

Current intended punctuation allocation:

```text
:     type annotation / named call argument
=>    lambda body
->    return/function type
=     value binding
,     argument/list/member separation where applicable
;     binding/field termination
```

Do not add general expression-level:

```nix
expr: Type
```

syntax.

---

# 5. Native Nix compatibility strategy

The nxc frontend does **not** need to be a syntactic superset of Nix.

It needs semantic coverage.

Native `.nix` files are parsed through rnix and translated into the semantic IR.

From there, the nxc emitter chooses canonical nxc spellings.

Examples:

```nix
# native
f x y
```

becomes:

```nix
f(x, y)
```

```nix
# native
x: x + 1
```

becomes:

```nix
x => x + 1
```

```nix
# native
let
    x = 1;
in
    x + 1
```

becomes:

```nix
let {
    x = 1;
    yield x + 1;
}
```

```nix
# native
[ a b c ]
```

becomes:

```nix
[a, b, c]
```

Native constructs whose public nxc spelling is not decided should use reserved `__nxc_*` compatibility syntax rather than forcing a premature language-design decision.

---

# 6. Strings, paths, URLs

For the scaffold, preserve Nix string/path semantics as closely as possible.

Nix lexical behavior around:

* quoted strings and interpolation;
* indented strings;
* relative paths;
* absolute paths;
* home-relative paths;
* search paths;
* interpolation in paths;

is subtle. Refer to the official Nix parser/manual and rnix tests as behavioral references.

Do not resolve paths during pure syntax conversion.

This distinction is important:

```text
syntax conversion:
    preserve source-relative path expression

future actual nxc import/compiler integration:
    resolve/rewrite relative paths according to original source location
```

Those are separate stages.

Bare native URL literals may be canonicalized to quoted strings during Nix → nxc conversion. Do not invest heavily in preserving discouraged URL-literal syntax.

---

# 7. Bidirectional conversion

Implement both directions from the start.

CLI shape:

```text
nxc check FILE.nxc

nxc to-nix FILE.nxc
nxc to-nix FILE.nxc -o FILE.nix

nxc from-nix FILE.nix
nxc from-nix FILE.nix -o FILE.nxc
```

Names may be adjusted slightly if Clap ergonomics suggest something better, but keep the operations explicit.

Do not implement Nix plugin/IFD integration yet.

---

# 8. Corpus testing against nixpkgs

Add an `xtask` command:

```text
cargo xtask corpus /path/to/nixpkgs
```

It should recursively find `.nix` files and perform:

```text
native source
    ↓ rnix
IR₁
    ↓ nxc emitter
generated nxc
    ↓ nxc parser
IR₂
    ↓ native Nix emitter
generated native Nix
    ↓ rnix
IR₃
```

Then require:

```text
canonical(IR₁) == canonical(IR₂)
canonical(IR₁) == canonical(IR₃)
```

Do not compare source text.

Whitespace, comments, redundant parentheses, choice of equivalent syntax, etc. must not affect semantic equivalence.

Report at least:

```text
files discovered
native parse successes/failures
Nix → IR successes
IR → nxc successes
generated nxc parse successes
IR₁ == IR₂ successes
IR → native Nix successes
generated native parse successes
IR₁ == IR₃ successes
```

For failures, record:

* source path;
* pipeline stage;
* error;
* ideally the smallest useful source span/node kind.

Allow:

```text
cargo xtask corpus ... --fail-fast
cargo xtask corpus ... --filter PATH_OR_PATTERN
```

if cheap to add.

Do not commit nixpkgs into the repository.

The corpus runner should take an external checkout/path.

The first scaffold does not need a perfect score. It should make unsupported native constructs obvious and measurable.

---

# 9. Semantic equality / normalization

Create a canonicalization layer specifically for round-trip testing.

Examples of syntax that should collapse to the same IR include:

```nix
f(a, b)
```

and native:

```nix
f a b
```

Likewise:

```nix
x => expr
fn(x) => expr
```

are identical.

And:

```nix
let {
    x = y;
    yield z;
}
```

maps to the same semantic form as native `let ... in`.

Parentheses and trivia do not survive into semantic equality.

If native constructs are deliberately represented through equivalent nxc syntax—for example Boolean implication rewritten as `!a || b`—normalize consistently so corpus equality still works.

Do **not** normalize distinctions that actually affect Nix evaluation.

---

# 10. Comments / formatting during reverse conversion

Do not make perfect comment preservation a prerequisite for semantic corpus testing.

Both rnix and the nxc Rowan CST are lossless at their respective syntax layers, so preserve the architecture needed to attach trivia later, but initial Nix ↔ nxc conversion may drop/reformat comments.

The important initial invariant is semantic IR round-tripping.

Do not let cross-dialect comment placement become a blocker for parser/compiler work.

---

# 11. Native Nix oracle

rnix is the fast in-process parser for the corpus.

Also add a small integration-test layer that, when the Nix executable is available, verifies generated `.nix` through the real Nix parser.

Do not shell out to Nix once per nixpkgs file in the normal corpus loop; that will make the corpus test unnecessarily expensive.

Use the real Nix parser for:

* fixture smoke tests;
* suspicious corpus failures;
* targeted semantic/evaluation tests.

Eventually add evaluation-equivalence tests for small self-contained expressions:

```text
nxc
 ↓
native Nix
 ↓ nix eval

expected value
```

For JSON-serializable values, comparing `nix eval --json` output is useful.

---

# 12. Testing

Use ordinary Rust unit/integration tests plus snapshots where helpful.

Add fixture directories approximately like:

```text
tests/
├── syntax/
│   ├── calls/
│   ├── lambdas/
│   ├── lists/
│   ├── attrsets/
│   ├── let/
│   ├── strings/
│   └── paths/
├── roundtrip/
└── invalid/
```

Important invariants:

1. Lexer must never panic on arbitrary input.
2. Parser must never panic on arbitrary input.
3. Lossless CST reconstruction reproduces nxc input byte-for-byte.
4. Valid nxc → IR → native Nix always produces parseable Nix.
5. Native Nix → IR → nxc → IR preserves canonical semantic IR.
6. nxc → IR → Nix → IR preserves canonical semantic IR.
7. Conversion is deterministic.

Add `proptest` where useful.

Set up `cargo-fuzz` scaffolding if inexpensive, but do not spend the initial task implementing an elaborate fuzzing campaign.

Useful fuzz targets eventually:

```text
lexer_never_panics
parser_never_panics
parse_print_identity
nxc_to_nix_parseable
semantic_roundtrip
```

---

# 13. Dependencies

Use current stable compatible releases rather than old examples from blog posts.

Expected core dependencies are approximately:

```toml
logos = "0.16"
chumsky = { version = "0.13", features = ["pratt"] }
rowan = "0.17"
rnix = "0.14"
```

Add a diagnostics crate such as Ariadne only if useful for the initial CLI.

Note that rnix may depend on a different Rowan version. That is fine: rnix syntax objects should be converted immediately through an adapter into our semantic IR. Do not expose rnix syntax-node types through nxc's public API.

Use Clap for the CLI unless there is a compelling reason not to.

---

# 14. Nix project support

Provide:

```text
flake.nix
```

with at least:

```text
nix develop
nix build
nix flake check
```

working.

The dev shell should provide:

* stable Rust toolchain;
* cargo;
* rustfmt;
* clippy;
* Nix itself;
* whatever minimal tooling tests require.

Do not build an elaborate Nix packaging framework in the scaffold.

---

# 15. README

The README should use the full title:

```text
nxc
Nix for Cniles
Lex Ferrata
『鉄律』
〜ＴＥＴＳＵＲＩＴＳＵ〜
```

Then one short description:

> A C/Rust-flavored concrete syntax for Nix with unchanged Nix evaluation semantics.

Include a compact comparison:

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

Mention that `fn` is optional.

Do not write a huge manifesto yet.

---

# 16. First implementation milestone

For this task, prioritize a **working vertical scaffold**, not complete language coverage.

The first pass should leave us with:

1. Cargo workspace builds.
2. Nix flake builds/checks.
3. Logos lexer exists with trivia/spans.
4. Chumsky parser exists with Pratt wiring and recovery architecture.
5. Rowan CST is constructed losslessly for the supported subset.
6. A semantic IR exists.
7. rnix native-Nix adapter exists.
8. Both Nix and nxc emitters exist.
9. CLI can perform `to-nix`, `from-nix`, and `check` for a representative subset.
10. `cargo xtask corpus /path/to/nixpkgs` exists and reports coverage/failures.
11. Unit and round-trip fixtures cover the implemented subset.
12. Unsupported syntax fails explicitly rather than being guessed at.

A representative vertical subset should include at least:

* identifiers and basic literals;
* attrsets;
* lists;
* selections;
* function calls;
* simple and attrset-pattern lambdas;
* `let { ... yield ...; }`;
* `if`;
* enough operators to exercise Pratt parsing.

Strings/paths/interpolation should have lexer architecture in place, but if complete Nix-compatible handling would substantially expand the initial task, implement them incrementally and make corpus failures explicit.

---

# 17. Things explicitly out of scope

Do not implement yet:

* type checking;
* type inference;
* LSP;
* full formatter;
* Nix evaluator;
* Nix plugin;
* IFD integration;
* automatic `.nxc` import from native Nix;
* path rewriting for generated store files;
* public syntax for attrset `//` update;
* generalized statement blocks;
* mutation;
* real multi-argument functions;
* changed laziness/recursion semantics.

Design the boundaries so those can be added later without rewriting the parser or IR.

---

# 18. Design rule when uncertain

If a syntax question is unresolved:

**do not invent a permanent user-facing syntax.**

Either:

1. preserve the native Nix spelling if it does not conflict with nxc;
2. use a reserved `__nxc_*` converter intrinsic temporarily;
3. report it as unsupported.

Prefer an explicit hole over freezing a questionable language-design choice just to make a test green.

Likewise, do not “simplify” Nix semantics in the IR. The IR is the semantic source of truth.

The project is an alternate concrete syntax for Nix, not a Nix-inspired language.

---

# Definition of done for the scaffold

The task is done when I can approximately do:

```bash
nix develop

cargo test

cargo run -p nxc-cli -- from-nix example.nix
cargo run -p nxc-cli -- to-nix example.nxc

cargo xtask corpus ~/src/nixpkgs
```

and see a useful corpus coverage report, with implemented files proving:

```text
Nix → semantic IR → nxc → semantic IR → Nix
```

works without semantic drift.

Do not chase 100% corpus coverage in the scaffold PR unless it falls out naturally. Establish the architecture and make the remaining coverage mechanically attackable.
