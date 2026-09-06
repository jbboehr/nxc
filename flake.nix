# SPDX-License-Identifier: AGPL-3.0-only WITH romic-exception
{
  description = "nxc — a C/Rust-flavored concrete syntax for Nix";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    fenix = {
      url = "github:nix-community/fenix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    {
      self,
      nixpkgs,
      fenix,
    }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "aarch64-darwin"
      ];
      forAllSystems =
        f:
        nixpkgs.lib.genAttrs systems (
          system:
          let
            pkgs = nixpkgs.legacyPackages.${system};
            toolchain = fenix.packages.${system}.stable.withComponents [
              "cargo"
              "clippy"
              "rust-src"
              "rustc"
              "rustfmt"
              "rust-analyzer"
            ];
            rustPlatform = pkgs.makeRustPlatform {
              cargo = toolchain;
              rustc = toolchain;
            };
          in
          f {
            inherit
              system
              pkgs
              toolchain
              rustPlatform
              ;
          }
        );
      manifest = builtins.fromTOML (builtins.readFile ./Cargo.toml);
      src = nixpkgs.lib.fileset.toSource {
        root = ./.;
        fileset = nixpkgs.lib.fileset.unions [
          ./.cargo
          ./Cargo.toml
          ./Cargo.lock
          ./crates
          ./xtask
          ./LICENSE.md
          ./docs/LICENSE_EXCEPTION.md
        ];
      };
    in
    {
      packages = forAllSystems (
        { pkgs, rustPlatform, ... }: {
          default = rustPlatform.buildRustPackage {
            pname = "nxc";
            version = manifest.workspace.package.version;
            inherit src;
            cargoLock.lockFile = ./Cargo.lock;
            cargoBuildFlags = [
              "--package"
              "nxc-cli"
            ];
            cargoTestFlags = [ "--workspace" ];
            nativeCheckInputs = [ pkgs.nix ];
            postInstall = ''
              install -Dm644 LICENSE.md "$out/share/doc/nxc/LICENSE.md"
              install -Dm644 docs/LICENSE_EXCEPTION.md "$out/share/doc/nxc/docs/LICENSE_EXCEPTION.md"
            '';
            meta = {
              description = "A C/Rust-flavored concrete syntax for Nix";
              homepage = manifest.workspace.package.repository;
              license = pkgs.lib.licenses.agpl3Only // {
                spdxId = manifest.workspace.package.license;
                fullName = "GNU Affero General Public License v3.0 only with Romic Exception";
                url = "https://github.com/jbboehr/nxc/blob/master/docs/LICENSE_EXCEPTION.md";
              };
              mainProgram = "nxc";
              platforms = systems;
            };
          };
        }
      );

      checks = forAllSystems (
        {
          system,
          pkgs,
          toolchain,
          ...
        }:
        {
          package = self.packages.${system}.default;
          clippy = self.packages.${system}.default.overrideAttrs {
            pname = "nxc-clippy";
            buildPhase = ''
              runHook preBuild
              cargo clippy --offline --locked --workspace --all-targets --all-features -- -D warnings
              runHook postBuild
            '';
            doCheck = false;
            installPhase = ''mkdir -p "$out"'';
            postInstall = "";
          };
          formatting =
            pkgs.runCommand "nxc-formatting"
              {
                nativeBuildInputs = [
                  toolchain
                  pkgs.nixfmt
                ];
              }
              ''
                cp -r ${src} source
                chmod -R u+w source
                cd source
                cargo fmt --all --check
                nixfmt --check ${./flake.nix}
                mkdir -p "$out"
              '';
        }
      );

      devShells = forAllSystems (
        { pkgs, toolchain, ... }: {
          default = pkgs.mkShell {
            packages = [
              toolchain
              pkgs.nix
              pkgs.nixfmt
            ];
            RUST_SRC_PATH = "${toolchain}/lib/rustlib/src/rust/library";
          };
        }
      );

      formatter = forAllSystems ({ pkgs, ... }: pkgs.nixfmt);
    };
}
