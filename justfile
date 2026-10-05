check:
    cargo fmt --check
    cargo check --all-targets
    cargo build --all-targets
    cargo test --all-targets
    cargo clippy --all-targets --all-features -- -D warnings
