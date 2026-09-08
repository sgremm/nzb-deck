{ pkgs ? import <nixpkgs> {} }:

let
  rust-overlay = import (builtins.fetchTarball
    "https://github.com/oxalica/rust-overlay/archive/master.tar.gz");
  pkgs' = import <nixpkgs> { overlays = [ rust-overlay ]; };
  rust-bin = pkgs'.rust-bin.stable."1.93.0".default.override {
    extensions = [ "rust-src" "rust-analyzer" "clippy" "rustfmt" ];
  };
in
pkgs'.mkShell {
  name = "usenet-dl-dev";

  buildInputs = with pkgs'; [
    # Node.js
    nodejs_24
    nodePackages.npm

    # Rust 1.93 toolchain
    rust-bin

    # Build essentials
    pkg-config
    bash
    openssl
    openssl.dev

    # For faster linking
    mold

    # Cargo extensions
    cargo-watch      # Auto-rebuild: cargo watch -x check
    cargo-edit       # cargo add/rm/upgrade
    cargo-nextest    # Faster test runner
    cargo-deny       # Audit dependencies
    cargo-outdated   # Check for outdated deps
    cargo-flamegraph # Performance profiling
    cargo-expand     # Expand macros
    cargo-tarpaulin  # Code coverage

    # Debugging
    gdb
  ];

  # Environment variables
  RUST_BACKTRACE = "1";
  RUST_LOG = "debug";

  # Use mold for faster linking
  RUSTFLAGS = "-C link-arg=-fuse-ld=mold";

  shellHook = ''
    # Ensure npm global installs go to a local directory
    export NPM_CONFIG_PREFIX="$HOME/.npm-global"
    export PATH="$NPM_CONFIG_PREFIX/bin:$PATH"
    mkdir -p "$NPM_CONFIG_PREFIX"

    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    echo " usenet-dl development environment"
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    echo ""
    echo "Rust: $(rustc --version)"
    echo "Node: $(node --version)"
    echo "npm:  $(npm --version)"
    echo ""
    echo "Commands:"
    echo "  cargo build             Build the library"
    echo "  cargo test              Run tests"
    echo "  cargo nextest run       Run tests (faster)"
    echo "  cargo watch -x check    Auto-check on save"
    echo "  cargo clippy            Run lints"
    echo "  cargo fmt               Format code"
    echo "  cargo doc --open        Build and view docs"
    echo ""
  '';
}
