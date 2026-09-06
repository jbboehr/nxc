# nxc

Nix for Cniles\
Lex Ferrata\
『鉄律』\
〜ＴＥＴＳＵＲＩＴＳＵ〜

A C/Rust-flavored concrete syntax for Nix with unchanged Nix evaluation semantics.

This project is at the repository-scaffolding stage. Parsing and conversion are
not available yet. The examples below show the intended syntax.

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
