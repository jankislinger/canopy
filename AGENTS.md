# Repository instructions

Before committing any changes, run all of these Cargo verification commands from the repository root:

```sh
cargo fmt --check
cargo check --all-targets
cargo build --all-targets
cargo test --all-targets
cargo clippy --all-targets --all-features -- -D warnings
```

All commands must complete successfully before creating a commit.
