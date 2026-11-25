{
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = {
    nixpkgs,
    flake-utils,
    rust-overlay,
    ...
  }:
    flake-utils.lib.eachSystem ["x86_64-linux"] (system: let
      pkgs = import nixpkgs {
        inherit system;
        overlays = [rust-overlay.overlays.default];
      };

      rust = pkgs.rust-bin.stable."1.95.0".default.override {
        extensions = ["rust-src" "rust-analyzer"];
        targets = ["x86_64-unknown-linux-gnu" "x86_64-pc-windows-msvc"];
      };

      tools = with pkgs; [
        # Languages
        rust
        (python312.withPackages (ps: [ps.pefile]))

        # Windows PE toolchain (clang-cl is only in the unwrapped package)
        llvmPackages.clang-unwrapped
        llvmPackages.lld
        llvmPackages.llvm
        cargo-xwin
        directx-shader-compiler

        # Launcher and GPU checks
        umu-launcher
        bubblewrap
        xvfb-run
        mangohud
        zenity
      ];
    in {
      devShells.default = pkgs.mkShell {
        packages = tools;
        buildInputs = [pkgs.lz4];

        # Python loads liblz4 through ctypes
        LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath [pkgs.lz4];
        VULKAN_HEADERS_INCLUDE = "${pkgs.vulkan-headers}/include";
      };
    });
}
