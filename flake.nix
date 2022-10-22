{
  description = "nix dev environment";

  inputs = {
    flake-utils.url = "github:numtide/flake-utils";
    naersk.url = "github:nix-community/naersk";
    naersk.inputs.nixpkgs.follows = "nixpkgs";
    rust-overlay.url = "github:oxalica/rust-overlay";
    rust-overlay.inputs.nixpkgs.follows = "nixpkgs";
  };

  outputs = { self, nixpkgs, flake-utils, naersk, rust-overlay, ... }:
    flake-utils.lib.eachDefaultSystem(system:
    let
      overlays = [ (import rust-overlay) ];
      pkgs = import nixpkgs {
        inherit system overlays;
      };
      toolchain = pkgs.rust-bin.selectLatestNightlyWith (toolchain: toolchain.default);
      naersk' = pkgs.callPackage naersk {
        cargo = toolchain;
        rustc = toolchain;
      };
    in {
      defaultPackage = naersk'.buildPackage {
        src = ./.;
      };
      devShell = pkgs.mkShell {
        buildInputs = with pkgs; [
          pkg-config
          openssl
          toolchain
        ];
      };
    });
}
