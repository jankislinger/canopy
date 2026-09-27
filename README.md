# learning-tui

`learning-tui` is a terminal project manager built with Rust and Ratatui. It is intended to grow into a terminal-native alternative to opening many projects in separate IDE windows.

## Current functionality

The application currently:

- scans `$HOME` for Git repositories;
- detects direct child Python and Rust projects using `pyproject.toml` and `Cargo.toml`;
- displays a filesystem-style project tree with Git/Python/Rust indicators;
- supports arrow-key navigation and directory expansion/collapse;
- opens selected projects in `nvim` with `Enter`;
- refreshes discovery with `r` and exits with `q`;
- includes an `Experiments` panel for testing behavior.

The project is currently a local project browser. tmux-backed persistent workspaces, session switching, configurable scan roots, and project-specific commands are planned but not implemented yet. See [DESIGN.md](DESIGN.md) for the planned direction.

## Development

Run formatting, compilation checks, and tests with:

```bash
cargo fmt
cargo check
cargo test
```

Run the application with:

```bash
cargo run
```

The current discovery root is the user’s `$HOME` directory. The application expects `nvim` to be available when opening a project.
