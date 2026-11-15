# Canopy

Canopy is a terminal project manager. Browse your Git repositories, open a persistent workspace for each project, and move between your editor, coding agent, shell, and Git tools through tmux.

## Getting started

Install Canopy with Rust and Cargo:

```bash
cargo install --git https://github.com/jankislinger/canopy.git
```

For project workspaces, install `tmux` and make your editor and agent commands available on your shell's `PATH`. The defaults are `nvim` and `codex`; the Git window uses `lazygit`. See [Editors](#editors) and [Agents](#agents) to change the commands. Running recipes requires `just`, and language-specific test and lint commands require the corresponding project tools.

Start Canopy inside a tmux session:

```bash
tmux new-session -s workspace
canopy
```

Use `Up` and `Down` to select a project, then press `Enter` to create its workspace and switch to it. Use tmux's session chooser (by default, prefix then `s`) to return to the session containing Canopy. Project sessions keep running when you quit Canopy with `q`.

By default, Canopy discovers repositories under `$HOME`. Pass a directory to limit discovery:

```bash
canopy ~/Projects
canopy .
```

Use `canopy --help` for command-line options and `canopy --version` to print the build commit.

## Projects and discovery

A project is a directory you can open as its own workspace. Canopy discovers Git repositories and identifies nested Python and Rust projects by their `pyproject.toml` and `Cargo.toml` files. A directory can have multiple kinds, shown as Git, Python, and Rust indicators in the tree.

The tree groups projects by filesystem location. Use `Up`/`Down` to move and `Left`/`Right` to collapse or expand directories. Press `/` to fuzzy-search projects, select a result with `Up`/`Down`, and press `Enter` to select it in the tree. `Esc` closes search.

Press `r` to refresh discovery after adding projects or changing project files. Dot-prefixed directories and configured skipped directory names are excluded from scanning. You can also hide or initially collapse particular directories through the [configuration file](#configuration-file). Press `H` to toggle entries hidden by those display settings for the current run.

## Sessions and windows

Each project workspace is a persistent tmux session rooted in that project's directory. Creating a session opens four windows:

| Window | What runs there |
| --- | --- |
| `editor` | Your configured editor, default `nvim` |
| `agent` | Your configured coding agent, default `codex` |
| `terminal` | A shell |
| `git` | `lazygit` |

Press `Enter` to switch to the selected project's last active window, creating its session if necessary. Press `s` to start a session without switching. While windows are opening, the project shows as starting; switching waits until every window has opened or failed. Window failures are reported, and Canopy still attempts to open the remaining windows.

Details shows the selected project's session status and windows, refreshed every second. Each window lists its tmux index, name, pane count, and active pane's command; `*` marks the active window. To open a particular window, press `o` followed by its displayed index from `0` to `9`. For example, `o1` opens window 1. These are actual tmux indices, including your configured starting index and gaps. This shortcut only opens existing windows. Press `Esc` to cancel the prefix.

Press `a` to open the active-session picker, use `Up`/`Down` to select a project, and press `Enter` to switch once it has finished starting. Press `Esc` to close the picker.

To stop the selected project's session, press `x` and confirm with `y`; `n` or `Esc` cancels. Stopping a session closes its windows and the processes running in them.

## Agents

Canopy starts a coding agent in the project's `agent` window when it creates a session. The default command is `codex`. Install and configure your chosen agent separately, including any authentication it requires.

Set `commands.agent` in the [configuration file](#configuration-file) to choose a command or add arguments:

```json
{
  "commands": {
    "agent": "codex"
  }
}
```

The command runs through tmux's shell with the project directory as its working directory. Interact with the agent directly in that window, and use `o` followed by its index in Details to return to it. The session preserves the running agent when you switch projects or quit Canopy.

Use `directory_overrides` to choose a different agent command for a particular project or a parent directory such as `~/Projects`. Projects beneath that directory inherit the command; a closer override takes precedence. Command changes apply when creating a new session; restart an existing session to use them.

## Editors

The `editor` window starts `nvim` by default. Set `commands.editor` to your preferred terminal editor command, including any arguments:

```json
{
  "commands": {
    "editor": "nvim --clean"
  }
}
```

Like agents, editors run in the project directory through tmux's shell. You can override the editor for individual project paths in the configuration file. Existing sessions keep their current editor process.

## Tests and linting

Select a project and press `t` to run tests or `l` to run lint checks. Canopy streams command output in a popup. Press `Esc` to close it; closing while a command is running cancels it.

If the project's justfile defines `test` or `lint`, the corresponding shortcut runs `just test` or `just lint`. Otherwise, Canopy uses the detected project kinds:

| Project kind | Tests (`t`) | Lint (`l`) |
| --- | --- | --- |
| Python | `uv run pytest` | `uv run ruff format --check`, then `uv run ruff check` |
| Rust | `cargo test` | `cargo fmt --check`, then `cargo clippy` |

Projects with both kinds use both sets of commands. A Git-only project needs the corresponding Just recipe to use these shortcuts. Commands run in the selected project's directory.

## Just recipes

When a project has a justfile, Details lists its recipes, parameters, and descriptions. Press `:` to open the recipe picker. Type to fuzzy-search names and descriptions, select with `Up`/`Down`, and press `Enter` to run the highlighted recipe in that project.

For example, type `:test` and press `Enter` to run `just test`. Recipes remain alphabetical while filtering. An exact name selects that recipe; otherwise, a name prefix selects the first alphabetical match. Arrow navigation lets you choose another match, and description-only matches preserve your selection while it still matches. If filtering removes your selection, the highlight moves to the nearest surviving recipe above it in the previous list, or the first match. With no matches, `Enter` leaves the picker open.

Press `Tab` to edit arguments separately, such as `staging` or `"hello world"`, and `Tab` again to return to the filter. Both fields support `Left`/`Right`, `Home`/`End`, `Backspace`, and `Delete`; `Ctrl-U` clears the focused field. Press `Esc` to cancel the picker.

Output streams in the same popup used for tests and linting. Closing it with `Esc` cancels a running recipe.

## Configuration file

Canopy reads `~/.config/canopy/settings.json` at startup. To customize Canopy, create the directory and settings file, then use the example below as a starting point:

```bash
mkdir -p ~/.config/canopy
nvim ~/.config/canopy/settings.json
```

Omit settings you do not need; omitted values keep their defaults. Restart Canopy after editing the file.

To use a different file, pass `--config-file`:

```bash
canopy --config-file ./settings.json ~/Projects
```

The discovery directory does not change the default config location or how `~/` resolves in settings. A missing default file uses defaults. An unreadable or invalid default file reports an error in the UI and uses defaults; an explicit `--config-file` that cannot be loaded causes Canopy to exit. Unknown settings are rejected.

```json
{
  "extra_skipped_dirs": ["vendor"],
  "commands": {
    "editor": "nvim --clean",
    "agent": "codex"
  },
  "directory_overrides": {
    "~/Projects": { "commands": { "agent": "claude" } },
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

`skipped_dirs` replaces the default skipped directory names; `extra_skipped_dirs` adds to them. They cannot be used together. Both are arrays of directory names. The default names are `target`, `node_modules`, `.venv`, `__pycache__`, `.cache`, and `build`. Skipped directories are excluded from discovery; `H` does not reveal them.

Directory override keys must be absolute paths or start with `~/`; `~` is also accepted for the home directory. Editor and agent commands inherit from parent directories, even when those directories are not projects. Each command uses the closest override that defines it, then falls back to the global command. For example, a project that overrides only its editor still inherits its agent from a parent. In the example above, projects under `~/Projects` use `claude`, while `~/Projects/special` uses `codex --full-auto`.

The `display` setting applies to the directory itself and controls how its subtree appears. It accepts:

- `hidden`: conceal the directory and its subtree. Press `H` to show hidden entries for the current run; hidden rows appear dimmed.
- `collapsed`: discover its children, but start with them collapsed. Select the directory and press `Right` to expand it.
- `expanded`: start with its children visible. This is the default.

Set the global editor and agent command strings under `commands`. A directory override can replace either command under its own `commands` object. Commands run through tmux's shell when Canopy creates a project session. Settings affect newly created sessions; they do not change commands in sessions that are already running.

## Updates

The startup screen shows the commit Canopy was built from, for example `Build ef12345` (with `-dirty` for builds with uncommitted changes). In the background, Canopy asks GitHub (using `curl`) whether `master` has commits this build lacks. When it does, the Commands panel shows `New version available (N commits, abc1234)  u: upgrade`. Pressing `u` runs `cargo install --locked --git https://github.com/jankislinger/canopy.git --branch master` in the output popup. After it succeeds, the panel shows `Upgraded: restart Canopy to apply`; quit and start Canopy again to use the new version. A build that already contains `master`, or is ahead of it, shows no hint.

The result is cached for 8 hours in `~/.cache/canopy/update.json` and is tied to the build commit, so installing a different version always triggers a fresh check. A binary running from its cargo target directory (for example via `cargo run`) is a dev build. It keeps its cache in that directory (`target/debug/canopy-update.json`), and it only shows how many commits `master` is ahead; `u` does nothing for dev builds.

## Development

Run the application from source with:

```bash
cargo run
```

Run the repository's full verification recipe with:

```bash
just check
```

This builds all targets, runs file hygiene checks, checks formatting and compilation, runs Clippy, and runs tests. See [DESIGN.md](DESIGN.md) for the planned direction. The UI also includes an Experiments panel for testing behavior.
