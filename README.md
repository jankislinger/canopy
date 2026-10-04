# Canopy

`canopy` is a terminal project manager built with Rust and Ratatui. It is intended to grow into a terminal-native alternative to opening many projects in separate IDE windows.

## Current functionality

The application currently:

- scans `$HOME` for Git repositories;
- detects nested Python and Rust projects using `pyproject.toml` and `Cargo.toml`;
- displays a filesystem-style project tree with Git/Python/Rust indicators;
- supports arrow-key navigation and directory expansion/collapse;
- opens selected projects in `nvim` with `Enter`;
- refreshes discovery with `r` and exits with `q`;
- includes an `Experiments` panel for testing behavior.

The project is currently a local project browser. tmux-backed persistent workspaces, session switching, configurable scan roots, and project-specific commands are planned but not implemented yet. See [DESIGN.md](DESIGN.md) for the planned direction.

## Configuration

Canopy reads `~/.config/canopy/settings.json` at startup. To customize Canopy, create the directory and settings file, then use the example below as a starting point:

```bash
mkdir -p ~/.config/canopy
nvim ~/.config/canopy/settings.json
```

Omit settings you do not need; omitted values keep their defaults. Restart Canopy after editing the file.

```json
{
  "extra_skipped_dirs": ["vendor"],
  "commands": {
    "editor": "nvim --clean",
    "agent": "codex"
  },
  "directory_overrides": {
    "~/Projects/archived": { "display": "hidden" },
    "~/Projects/monorepo/python_libs": { "display": "collapsed" },
    "~/Projects/special": {
      "display": "expanded",
      "commands": {
        "editor": "nvim --listen /tmp/special.nvim",
        "agent": "codex --full-auto"
      }
    }
  }
}
```

`skipped_dirs` replaces the default skipped directory names; `extra_skipped_dirs` adds to them. They cannot be used together. Both are arrays of directory names.

Directory override keys must be absolute paths or start with `~/`; they match the corresponding directory exactly. `display` accepts:

- `hidden`: conceal the directory and its subtree. Press `H` to show hidden entries for the current run; hidden rows appear dimmed.
- `collapsed`: discover its children, but start with them collapsed. Select the directory and press `Right` to expand it.
- `expanded`: start with its children visible. This is the default.

Set the global editor and agent command strings under `commands`. A directory override can replace either command under its own `commands` object. Commands run through tmux's shell when Canopy creates a project session. Settings affect newly created sessions; they do not change commands in sessions that are already running.

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
