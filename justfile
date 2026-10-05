# List available recipes
_list:
    @just --list

# Check formatting and compilation, and run Clippy with warnings treated as errors
lint:
    cargo fmt --check
    cargo check --all-targets
    cargo clippy --all-targets --all-features -- -D warnings

# Run tests for all targets
test:
    cargo test --all-targets

# Build all targets, then run lint checks and tests
check:
    cargo build --all-targets
    @just lint
    @just test
