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

# Run file hygiene checks on all files
pre-commit:
    uvx pre-commit run --all-files --show-diff-on-failure --color=always

# Build all targets, then run file hygiene checks, lint checks, and tests
check:
    cargo build --all-targets
    @just pre-commit
    @just lint
    @just test
