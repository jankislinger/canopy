# Canopy

`canopy` is a terminal project manager built with Rust and Ratatui. It is intended to grow into a terminal-native alternative to opening many projects in separate IDE windows.

## Installation

```bash
cargo install --git https://github.com/jankislinger/canopy.git
```

## Current functionality

The application currently:

- scans `$HOME` for Git repositories;
- detects nested Python and Rust projects using `pyproject.toml` and `Cargo.toml`;
- displays a filesystem-style project tree with Git/Python/Rust indicators;
- supports arrow-key navigation and directory expansion/collapse;
- creates persistent tmux project sessions with editor, agent, and terminal windows;
- switches to the selected project's last active tmux window with `Enter` (creating its session if needed);
- shows running/attached sessions and their windows in Details, refreshed every second;
- lists the selected project's Just recipes, including parameters and descriptions, in Details when it has a justfile;
- opens a floating Just recipe picker with `:`; fuzzy-search names and descriptions, select with arrows, then press `Enter` to stream its output like test/lint commands;
- opens a window with `o` followed by its actual tmux index (`0`–`9`), for example `o1` for window 1; `Esc` cancels the prefix;
- starts sessions with `s` and stops them with `x` followed by confirmation;
- refreshes discovery with `r` and exits with `q`;
- includes an `Experiments` panel for testing behavior.

Run Canopy inside tmux to switch between project sessions. The window shortcut uses the indices displayed in Details and only opens existing sessions/windows. It respects tmux's configured starting index and gaps: `o1` means window 1, while `o0` means window 0. Details shows each window's name, an `*` for the active window, pane count, and the active pane's command. See [DESIGN.md](DESIGN.md) for the planned direction.

Select a project with a justfile and press `:` to open the recipe picker. Typing fuzzy-searches recipe names and descriptions. Recipes stay in alphabetical order as you type. An exact recipe-name match moves the highlight without moving the recipe, so `:test` followed by `Enter` runs `just test`. Without an exact name, a name prefix highlights the first alphabetical recipe starting with that text, so typing `t` highlights `test` on this project. After arrow navigation, partial matches preserve your selection; description-only matches also preserve the active recipe while it still matches. `Up`/`Down` override the highlight, and `Enter` runs the selected recipe. If filtering removes the active recipe, selection moves to the nearest surviving recipe above it in the previous displayed list, or the first match if none survives above. With no matches, Enter leaves the picker open. Press `Tab` to edit arguments separately (for example `staging` or `"hello world"`), and Tab again to return to the filter. `Left`/`Right`, `Home`/`End`, `Backspace`, and `Delete` edit the focused field; `Ctrl-U` clears it; `Esc` cancels. Recipes run in the selected project using its own justfile. Closing the output popup with `Esc` cancels a running command, just as for test and lint.

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
