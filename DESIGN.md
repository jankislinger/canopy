# Project Workspace Manager Design

## Vision

This application should become a local project workspace manager rather than reimplementing an IDE. It should provide one place to discover projects, start their development environments, and switch between them. Neovim remains the editor and tmux remains the process and workspace supervisor.

The target workflow is similar to switching between PyCharm or RustRover projects, but with terminal-native workspaces:

- one tmux session or window per project;
- Neovim opened in the project directory;
- optional shell, test, and watcher panes;
- quick switching between active projects.

## Current State

The TUI currently:

- scans `$HOME` for Git repositories;
- skips hidden directories and common generated/dependency directories;
- detects repository roots, direct child Python projects (`pyproject.toml`), and Rust projects (`Cargo.toml`);
- combines indicators when a directory is multiple project types;
- displays a filesystem-style project tree;
- supports arrow-key navigation and expansion/collapse;
- opens selected projects in `nvim`;
- supports manual refresh with `r` and quitting with `q`;
- retains the original `Experiments` panel and loading demonstration;
- has basic backend discovery tests.

The current implementation does not yet manage persistent editor sessions, tmux sessions, project commands, or configuration.

## Workspace Model

Each detected project should eventually have:

- canonical project path;
- display name and project type indicators;
- parent Git repository;
- stable tmux session name;
- session status (`attached`, `running`, or `stopped`);
- optional last-used timestamp;
- optional project-specific commands and layout.

Use the canonical path to derive a stable, collision-resistant session name, for example:

```text
learning-tui--home-jan-project
```

Git worktrees should remain a future hierarchy layer between a repository and its coding projects. They are not part of the first session implementation.

## tmux Integration

The project manager should run inside tmux and use tmux as the workspace backend.

When the user activates a project:

1. Check whether its tmux session exists.
2. Create the session if necessary, with the project directory as its working directory.
3. Start `nvim` in the project directory.
4. Optionally create a standard layout containing an editor pane, shell pane, and test/watch pane.
5. Switch the current tmux client to that session.

The manager should use tmux commands for switching and lifecycle management rather than embedding long-running editor or shell processes itself.

The project tree should show session state, for example:

```text
▾ ~/Projects
  [Git,Python] api-service       ● attached
  [Git,Rust]   compiler           ○ stopped
  [Git]        experiments        ● running
```

Suggested controls:

- `Enter`: switch to or create the project session;
- `s`: start a session without switching, if useful;
- `a`: attach or switch to the selected session;
- `x`: stop/kill the selected session after confirmation;
- `r`: refresh projects and session state;
- `n`: create a new project workspace, if project creation is added;
- `q`: quit the manager without stopping sessions.

The exact keymap should remain centralized in `action.rs` and should not be hard-coded in the UI.

## Module Direction

The current module structure should evolve toward:

```text
src/
├── main.rs
├── app.rs
├── action.rs
├── event.rs
├── ui.rs
└── backend/
    ├── mod.rs
    ├── projects.rs
    └── sessions.rs
```

Responsibilities:

- `main.rs`: startup, terminal lifecycle, and dependency wiring;
- `app.rs`: state, actions, selection, and coordination;
- `action.rs`: application actions and key bindings;
- `event.rs`: terminal input and event delivery;
- `ui.rs`: Ratatui rendering only;
- `backend/projects.rs`: project discovery and project metadata;
- `backend/sessions.rs`: tmux session commands and status queries.

The UI should not execute tmux commands directly. The session backend should expose typed operations and return useful errors for display in the status area.

## Implementation Roadmap

Build the workspace capability in this order:

1. Add a session backend that can list, create, switch to, and stop tmux sessions.
2. Add session status to the project model and tree rows.
3. Make `Enter` switch to or create the selected project session.
4. Add a standard tmux layout with Neovim and a shell.
5. Add session lifecycle controls and confirmation prompts.
6. Add configuration and persistence.
7. Add project-specific actions such as test, run, format, and build.

## Configuration

The first session implementation can use these defaults:

- `$HOME` as the discovery root;
- `nvim` as the editor;
- one tmux session per project;
- one editor pane and one shell pane;
- no project-specific startup commands.

Once the session model is stable, add a configuration file for:

- scan roots;
- ignored directories;
- editor command;
- session naming;
- default pane layout;
- project-specific startup commands.

## Design Constraints

- Do not embed Neovim or long-running terminal processes inside the TUI.
- Keep discovery and tmux operations testable without requiring an interactive terminal.
- Make session names deterministic and safe for tmux.
- Treat missing tmux or nvim as recoverable user-facing errors.
- Preserve project tree navigation independently from session state.
- Keep worktree support deferred until repository and project sessions are reliable.
