set shell := [ "bash", "-euo", "pipefail", "-c" ]
set script-interpreter := [ "bash", "-euo", "pipefail" ]

default:
  @just --list

# Nettoyer le workspace
clean:
  cargo clean

# Format + Clippy
lint:
  cargo fmt --all
  cargo clippy --all-targets --all-features -- -D warnings

# Tests unitaires + d’intégration
test:
  cargo test --all-targets --all-features

dry-run:
  cargo run -- --data-dir "$DATA_DIR" --dry-run

run:
  cargo run -- --data-dir "$DATA_DIR" --db-url "$DATABASE_URL"

# Build release natif
build-native:
  cargo build --release

# Build global (les deux variantes)
build: build-native

# CI = fmt + clippy + tests + builds
ci: lint test build