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
    cargo nextest run --workspace
    cargo test --workspace --doc

audit:
    cargo audit

ci: fmt-check check lint test audit

