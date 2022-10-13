{
  description = "nix dev environment";

  inputs = {
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay.url = "github:oxalica/rust-overlay";
    rust-overlay.inputs.nixpkgs.follows = "nixpkgs";
  };

  outputs = { self, nixpkgs, flake-utils, rust-overlay, ... }:
    flake-utils.lib.eachDefaultSystem(system:
    let
      overlays = [ (import rust-overlay) ];
      pkgs = import nixpkgs {
        inherit system overlays;
      };
    in {
      devShell = pkgs.mkShell {
        buildInputs = with pkgs; [
          pkg-config
          openssl
          (rust-bin.selectLatestNightlyWith (toolchain: toolchain.default))
        ];
      };
    });
}
