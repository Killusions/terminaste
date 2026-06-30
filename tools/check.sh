#!/usr/bin/env sh
set -eu
cargo run -p terminaste-tools -- assets
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
