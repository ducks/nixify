{
  description = "Read-only Linux to NixOS migration assistant";
  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  outputs = { self, nixpkgs, ... }:
    let
      systems = [ "x86_64-linux" "aarch64-linux" ];
      each = f: nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});
    in {
      packages = each (pkgs: {
        default = pkgs.rustPlatform.buildRustPackage {
          pname = "nixify";
          version = "0.1.0";
          src = ./.;
          cargoLock.lockFile = ./Cargo.lock;
        };
      });
      devShells = each (pkgs: { default = pkgs.mkShell { packages = with pkgs; [ cargo rustc rustfmt clippy nix ]; }; });
      formatter = each (pkgs: pkgs.nixfmt-rfc-style);
      checks = each (pkgs: {
        package = self.packages.${pkgs.stdenv.hostPlatform.system}.default;
        lint = self.packages.${pkgs.stdenv.hostPlatform.system}.default.overrideAttrs (old: {
          nativeBuildInputs = (old.nativeBuildInputs or []) ++ [ pkgs.clippy pkgs.rustfmt ];
          checkPhase = ''
            runHook preCheck
            cargo fmt --all -- --check
            cargo clippy --all-targets --offline -- -D warnings
            runHook postCheck
          '';
          doCheck = true;
        });
      });
    };
}
