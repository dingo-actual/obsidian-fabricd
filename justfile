set shell := ["zsh", "-cu"]

default:
    @just --list

fmt:
    cargo fmt --all

fmt-check:
    cargo fmt --all -- --check

check:
    cargo check --workspace --all-targets

lint:
    cargo clippy --workspace --all-targets -- -D warnings

test:
    # --no-tests=pass: an empty suite is not a failure.
    cargo nextest run --workspace --no-tests=pass
    cargo test --workspace --doc

audit:
    cargo audit

ci: fmt-check check lint test audit
