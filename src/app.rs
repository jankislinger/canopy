use crate::{
    action::Action,
    backend::sessions::{Session, SessionBackend},
    backend::{Project, ProjectKind, Repository, git::GitStatus},
    config::{DisplayMode, Settings},
    ui,
};
use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use ratatui::{DefaultTerminal, widgets::ListState};
use std::{
    collections::{BTreeSet, HashMap, HashSet},
    io::Read,
    path::{Path, PathBuf},
};
use tokio::{
    sync::mpsc::{Receiver, Sender},
    time::{self, Duration},
};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Confirmation {
    StopSession,
}

#[derive(Clone, Copy)]
enum CommandKind {
    Test,
    Lint,
}

impl CommandKind {
    fn title(self) -> &'static str {
        match self {
            Self::Test => "Test",
            Self::Lint => "Lint",
        }
    }

    fn recipe(self) -> &'static str {
        match self {
            Self::Test => "test",
            Self::Lint => "lint",
        }
    }
}

pub struct CommandPopup {
    pub title: String,
    pub output: String,
    pub scroll: u16,
    pub follow: bool,
    pub success: Option<bool>,
}

pub struct AppState {
    pub repositories: Vec<Repository>,
    pub home: PathBuf,
    pub selected: PathBuf,
    pub project_list_state: ListState,
    pub expanded: HashSet<PathBuf>,
    pub show_hidden: bool,
    pub settings: Settings,
    pub error_popup: Option<String>,
    pub loading: bool,
    pub completed: usize,
    pub spinner: usize,
    pub status: String,
    pub sessions: Vec<Session>,
    pub session_backend: SessionBackend,
    pub confirmation: Option<Confirmation>,
    pub project_scan_generation: u64,
    pub command_popup: Option<CommandPopup>,
    pub command_generation: u64,
    pub git_status: Option<GitStatus>,
    pub git_status_cache: HashMap<PathBuf, GitStatus>,
    pub git_status_loading: bool,
    pub git_status_generation: u64,
    pub sessions_refresh_generation: u64,
    pub sessions_refresh_pending: bool,
}
impl AppState {
    /// Creates application state with the discovered tree expanded.
    pub fn new(
        home: PathBuf,
        repositories: Vec<Repository>,
        session_backend: SessionBackend,
        settings: Settings,
        settings_error: Option<String>,
    ) -> Self {
        let expanded = initial_expanded(&home, &repositories, &settings);
        let selected = rows(&home, &repositories, &expanded, &settings, false)
            .first()
            .map(|r| r.path.clone())
            .unwrap_or_else(|| home.clone());
        Self {
            repositories,
            home,
            selected,
            project_list_state: ListState::default(),
            expanded,
            show_hidden: false,
            settings,
            error_popup: settings_error,
            loading: false,
            completed: 0,
            spinner: 0,
            status: String::new(),
            sessions: session_backend.list().unwrap_or_default(),
            session_backend,
            confirmation: None,
            project_scan_generation: 0,
            command_popup: None,
            command_generation: 0,
            git_status: None,
            git_status_cache: HashMap::new(),
            git_status_loading: false,
            git_status_generation: 0,
            sessions_refresh_generation: 0,
            sessions_refresh_pending: false,
        }
    }
}
/// Runs the application event loop until the user quits.
pub async fn run(
    t: &mut DefaultTerminal,
    rx: &mut Receiver<Action>,
    tx: Sender<Action>,
    mut s: AppState,
) -> color_eyre::Result<()> {
    request_git_status(&mut s, tx.clone());
    let mut tick = time::interval(Duration::from_millis(150));
    let mut sessions_tick = time::interval(Duration::from_secs(1));
    sessions_tick.set_missed_tick_behavior(time::MissedTickBehavior::Skip);
    sessions_tick.tick().await;
    loop {
        t.draw(|f| ui::draw(f, &mut s))?;
        tokio::select! {
            action = rx.recv() => if s.error_popup.is_some() {
                match action {
                    Some(Action::Quit | Action::Cancel | Action::Open) => s.error_popup = None,
                    Some(action) => { let _ = handle_background_action(&mut s, action, tx.clone()); }
                    None => break,
                }
            } else { match (s.command_popup.as_ref(), action) {
                (None, Some(action)) => match action {
                    Action::Quit => break,
                    Action::Up => {
                        select(&mut s, -1);
                        request_git_status(&mut s, tx.clone());
                    }
                    Action::Down => {
                        select(&mut s, 1);
                        request_git_status(&mut s, tx.clone());
                    }
                    Action::Left => {
                        if has_child(&s) { s.expanded.remove(&s.selected); }
                    }
                    Action::Right => {
                        if has_child(&s) { s.expanded.insert(s.selected.clone()); }
                    }
                    Action::ToggleHidden => toggle_hidden(&mut s, tx.clone()),
                    Action::Open => activate(&mut s),
                    Action::Start => start(&mut s),
                    Action::Stop => request_stop(&mut s),
                    Action::Confirm => match s.confirmation.take() {
                        Some(Confirmation::StopSession) => stop(&mut s),
                        None => {}
                    },
                    Action::Cancel => {
                        s.confirmation = None;
                        s.status.clear();
                    }
                    Action::Test => run_command(&mut s, CommandKind::Test, tx.clone()),
                    Action::Lint => run_command(&mut s, CommandKind::Lint, tx.clone()),
                    Action::Refresh => refresh_projects(&mut s, tx.clone()),
                    action => { let _ = handle_background_action(&mut s, action, tx.clone()); }
                },
                (Some(_), Some(action)) => match action {
                    Action::Quit | Action::Cancel => {
                        s.command_popup = None;
                        s.command_generation += 1;
                        s.confirmation = None;
                        s.status.clear();
                    }
                    Action::Up => scroll_popup(&mut s, -1),
                    Action::Down => scroll_popup(&mut s, 1),
                    action => { let _ = handle_background_action(&mut s, action, tx.clone()); }
                },
                (_, None) => break,
            }},
            _ = sessions_tick.tick() => refresh_sessions_background(&mut s, tx.clone()),
            _ = tick.tick(), if s.loading => s.spinner = (s.spinner + 1) % 6,
        }
    }
    Ok(())
}

/// Handles asynchronous events that can arrive with or without an open popup.
fn handle_background_action(
    state: &mut AppState,
    action: Action,
    tx: Sender<Action>,
) -> Result<(), Action> {
    match action {
        Action::Resize => {}
        Action::CommandFinished { generation, title, output, success } => {
            if generation == state.command_generation {
                if let Some(popup) = state.command_popup.as_mut() {
                    popup.title = title;
                    popup.output = output;
                    popup.success = Some(success);
                    if popup.follow {
                        popup.scroll = bottom_scroll(&popup.output);
                    }
                } else {
                    state.command_popup = Some(CommandPopup {
                        title,
                        output,
                        scroll: 0,
                        follow: true,
                        success: Some(success),
                    });
                }
            }
        }
        Action::CommandOutput { generation, output } => {
            if generation == state.command_generation
                && let Some(popup) = state.command_popup.as_mut()
            {
                popup.output.push_str(&output);
                if popup.follow {
                    popup.scroll = bottom_scroll(&popup.output);
                }
            }
        }
        Action::GitStatusLoaded { generation, path, result } => {
            if generation == state.git_status_generation && path == state.selected {
                state.git_status_loading = false;
                if let Ok(status) = result {
                    state.git_status_cache.insert(path, status.clone());
                    state.git_status = Some(status);
                }
            }
        }
        Action::SessionsLoaded { generation, result } => {
            if generation == state.sessions_refresh_generation {
                state.sessions_refresh_pending = false;
                if let Ok(sessions) = result
                    && sessions != state.sessions
                {
                    state.sessions = sessions;
                }
            }
        }
        Action::ProjectsLoaded { generation, result } => {
            if generation == state.project_scan_generation {
                match result {
                    Ok(repositories) => {
                        let previously_expanded = state.expanded.clone();
                        state.repositories = repositories;
                        let available = all_dirs(&state.home, &state.repositories);
                        state.expanded = previously_expanded
                            .into_iter()
                            .filter(|path| available.contains(path))
                            .collect();
                        state.expanded.insert(state.home.clone());
                        let visible = rows(
                            &state.home,
                            &state.repositories,
                            &state.expanded,
                            &state.settings,
                            state.show_hidden,
                        );
                        if !visible.iter().any(|row| row.path == state.selected) {
                            state.selected = visible
                                .first()
                                .map(|row| row.path.clone())
                                .unwrap_or_else(|| state.home.clone());
                        }
                        request_git_status(state, tx.clone());
                        state.status = "Projects refreshed".into();
                    }
                    Err(error) => state.status = format!("Project scan failed: {error}"),
                }
            }
        }
        Action::Wait => {
            state.loading = true;
            tokio::spawn(wait(tx));
        }
        Action::Done => {
            state.loading = false;
            state.completed += 1;
        }
        action => return Err(action),
    }
    Ok(())
}

/// Starts a background scan of the project tree.
fn refresh_projects(state: &mut AppState, tx: Sender<Action>) {
    state.project_scan_generation += 1;
    let generation = state.project_scan_generation;
    let home = state.home.clone();
    let skipped_dirs = state.settings.skipped_dirs.clone();
    tokio::spawn(async move {
        let result = tokio::task::spawn_blocking(move || {
            crate::backend::discover_repositories_with_skipped_dirs(&home, &skipped_dirs)
        })
        .await
        .map_err(|error| error.to_string())
        .and_then(|result| result.map_err(|error| error.to_string()));
        let _ = tx.send(Action::ProjectsLoaded { generation, result }).await;
    });
    state.status = "Scanning projects...".into();
}

/// Sends a completion action after the demonstration delay.
async fn wait(tx: Sender<Action>) {
    time::sleep(Duration::from_secs(2)).await;
    let _ = tx.send(Action::Done).await;
}

fn refresh_sessions(state: &mut AppState) {
    state.sessions_refresh_generation += 1;
    state.sessions_refresh_pending = false;
    match state.session_backend.list() {
        Ok(sessions) if sessions != state.sessions => state.sessions = sessions,
        Ok(_) => {}
        Err(error) => state.status = error.to_string(),
    }
}

/// Refreshes tmux sessions without blocking the application event loop.
fn refresh_sessions_background(state: &mut AppState, tx: Sender<Action>) {
    if state.sessions_refresh_pending {
        return;
    }
    state.sessions_refresh_generation += 1;
    state.sessions_refresh_pending = true;
    let generation = state.sessions_refresh_generation;
    let backend = state.session_backend.clone();
    tokio::spawn(async move {
        let result =
            tokio::task::spawn_blocking(move || backend.list().map_err(|error| error.to_string()))
                .await
                .map_err(|error| error.to_string())
                .and_then(|result| result);
        let _ = tx.send(Action::SessionsLoaded { generation, result }).await;
    });
}

fn request_git_status(state: &mut AppState, tx: Sender<Action>) {
    state.git_status_generation += 1;
    let generation = state.git_status_generation;
    let path = state.selected.clone();
    if project(&state.repositories, &path).is_none() {
        state.git_status = None;
        state.git_status_loading = false;
        return;
    }
    let cached = state.git_status_cache.get(&path).cloned();
    state.git_status = cached.clone();
    state.git_status_loading = cached.is_none();
    let load_path = path.clone();
    tokio::spawn(async move {
        let result =
            tokio::task::spawn_blocking(move || crate::backend::git::load_status(&load_path))
                .await
                .map_err(|error| error.to_string())
                .and_then(|result| result);
        let _ = tx.send(Action::GitStatusLoaded { generation, path, result }).await;
    });
}

fn session_name(state: &AppState) -> Option<String> {
    if project(&state.repositories, &state.selected).is_some() {
        Some(crate::backend::sessions::session_name(&state.selected))
    } else {
        None
    }
}
fn start(state: &mut AppState) {
    if let Some(name) = session_name(state) {
        let editor = state.settings.editor_for(&state.selected);
        let agent = state.settings.agent_for(&state.selected);
        match state.session_backend.create(&state.selected, editor, agent) {
            Ok(_) => {
                refresh_sessions(state);
                state.status = format!("Started {name}");
            }
            Err(error) => state.status = error.to_string(),
        }
    }
}
fn activate(state: &mut AppState) {
    if let Some(name) = session_name(state) {
        let editor = state.settings.editor_for(&state.selected);
        let agent = state.settings.agent_for(&state.selected);
        let result = if state.sessions.iter().any(|session| session.name == name) {
            state.session_backend.switch_to(&name)
        } else {
            state
                .session_backend
                .create(&state.selected, editor, agent)
                .and_then(|_| state.session_backend.switch_to(&name))
        };
        match result {
            Ok(()) => state.status = format!("Switched to {name}"),
            Err(error) => state.status = error.to_string(),
        }
        refresh_sessions(state);
    }
}
fn request_stop(state: &mut AppState) {
    if session_name(state)
        .and_then(|name| state.sessions.iter().find(|session| session.name == name).map(|_| name))
        .is_some()
    {
        state.confirmation = Some(Confirmation::StopSession);
        state.status = "Stop session? y/n".into();
    }
}
fn stop(state: &mut AppState) {
    if let Some(name) = session_name(state) {
        match state.session_backend.stop(&name) {
            Ok(()) => {
                refresh_sessions(state);
                state.status = format!("Stopped {name}");
            }
            Err(error) => state.status = error.to_string(),
        }
    }
    state.confirmation = None;
}

fn run_command(state: &mut AppState, command: CommandKind, tx: Sender<Action>) {
    let Some(project) = project(&state.repositories, &state.selected) else {
        return;
    };
    let path = project.path.clone();
    let kinds = project.kinds.clone();
    let title = command.title().to_string();
    state.command_generation += 1;
    let generation = state.command_generation;
    state.command_popup = Some(CommandPopup {
        title: title.clone(),
        output: "Running...\n".into(),
        scroll: 0,
        follow: true,
        success: None,
    });
    let output_tx = tx.clone();
    tokio::spawn(async move {
        let result = tokio::task::spawn_blocking(move || {
            execute_commands_streaming(&path, &kinds, command, generation, output_tx)
        })
        .await
        .unwrap_or_else(|error| (format!("Command task failed: {error}"), false));
        let _ = tx
            .send(Action::CommandFinished {
                generation,
                title,
                output: result.0,
                success: result.1,
            })
            .await;
    });
}

fn scroll_popup(state: &mut AppState, delta: i16) {
    if let Some(popup) = state.command_popup.as_mut() {
        if delta.is_negative() {
            popup.scroll = popup.scroll.saturating_sub(delta.unsigned_abs());
            popup.follow = false;
        } else {
            let bottom = bottom_scroll(&popup.output);
            popup.scroll = popup.scroll.saturating_add(delta as u16).min(bottom);
            popup.follow = popup.scroll == bottom;
        }
    }
}

fn bottom_scroll(output: &str) -> u16 {
    output.lines().count().saturating_sub(18).min(u16::MAX as usize) as u16
}

fn project_commands(
    path: &Path,
    kinds: &[ProjectKind],
    command: CommandKind,
) -> Vec<Vec<&'static str>> {
    let declaration = format!("{}:", command.recipe());
    let has_recipe = std::fs::read_to_string(path.join("justfile"))
        .is_ok_and(|contents| contents.lines().any(|line| line.starts_with(&declaration)));
    if has_recipe {
        return vec![vec!["just", command.recipe()]];
    }

    let (python, rust) = match command {
        CommandKind::Test => (vec![vec!["uv", "run", "pytest"]], vec![vec!["cargo", "test"]]),
        CommandKind::Lint => (
            vec![
                vec!["uv", "run", "ruff", "format", "--check"],
                vec!["uv", "run", "ruff", "check"],
            ],
            vec![vec!["cargo", "fmt", "--check"], vec!["cargo", "clippy"]],
        ),
    };

    let mut commands = Vec::new();
    if kinds.contains(&ProjectKind::Python) {
        commands.extend(python);
    }
    if kinds.contains(&ProjectKind::Rust) {
        commands.extend(rust);
    }
    commands
}

fn execute_commands_streaming(
    path: &Path,
    kinds: &[ProjectKind],
    command: CommandKind,
    generation: u64,
    tx: Sender<Action>,
) -> (String, bool) {
    let commands = project_commands(path, kinds, command);
    if commands.is_empty() {
        return ("No test or lint command is defined for this project.".into(), false);
    }
    let mut output = String::new();
    let mut success = true;
    for command in commands {
        let header = format!("$ {}\n", command.join(" "));
        output.push_str(&header);
        let _ = tx.blocking_send(Action::CommandOutput { generation, output: header });
        match execute_command_in_pty(path, &command, generation, &tx) {
            Ok((command_output, command_success)) => {
                output.push_str(&command_output);
                success &= command_success;
            }
            Err(error) => {
                let text = format!("failed to start command: {error}\n\n");
                output.push_str(&text);
                success = false;
                let _ = tx.blocking_send(Action::CommandOutput { generation, output: text });
            }
        }
    }
    (output, success)
}

fn execute_command_in_pty(
    path: &Path,
    command: &[&str],
    generation: u64,
    tx: &Sender<Action>,
) -> Result<(String, bool), Box<dyn std::error::Error + Send + Sync>> {
    let pty_system = native_pty_system();
    let pair =
        pty_system.openpty(PtySize { rows: 24, cols: 120, pixel_width: 0, pixel_height: 0 })?;
    let mut builder = CommandBuilder::new(command[0]);
    builder.args(&command[1..]);
    builder.cwd(path);
    let mut child = pair.slave.spawn_command(builder)?;
    drop(pair.slave);

    let mut reader = pair.master.try_clone_reader()?;
    let mut command_output = String::new();
    let mut buffer = [0_u8; 4096];
    loop {
        let bytes_read = reader.read(&mut buffer)?;
        if bytes_read == 0 {
            break;
        }
        let chunk = String::from_utf8_lossy(&buffer[..bytes_read]).into_owned();
        command_output.push_str(&chunk);
        let _ = tx.blocking_send(Action::CommandOutput { generation, output: chunk });
    }

    let status = child.wait()?;
    Ok((command_output, status.success()))
}

pub struct Row {
    pub path: PathBuf,
    pub depth: usize,
    pub kinds: Option<Vec<ProjectKind>>,
    pub hidden: bool,
}

/// Produces the visible filesystem-tree rows for the discovered projects.
pub fn rows(
    home: &Path,
    repos: &[Repository],
    expanded: &HashSet<PathBuf>,
    settings: &Settings,
    show_hidden: bool,
) -> Vec<Row> {
    let mut set = BTreeSet::new();
    for r in repos {
        for p in &r.projects {
            let mut x = p.path.clone();
            while x.starts_with(home) {
                set.insert(x.clone());
                if x == home {
                    break;
                }
                let Some(y) = x.parent() else { break };
                x = y.to_path_buf()
            }
        }
    }
    set.into_iter()
        .filter_map(|p| {
            if p == home {
                return None;
            }
            let is_hidden = settings.display_for(&p) == DisplayMode::Hidden;
            let has_hidden_ancestor = p
                .ancestors()
                .take_while(|ancestor| *ancestor != home)
                .any(|ancestor| settings.display_for(ancestor) == DisplayMode::Hidden);
            if !show_hidden && (is_hidden || has_hidden_ancestor) {
                return None;
            }
            if p != home
                && !p.ancestors().skip(1).all(|a| !a.starts_with(home) || expanded.contains(a))
            {
                return None;
            }
            let depth =
                p.strip_prefix(home).map(|x| x.components().count()).unwrap_or(1).saturating_sub(1);
            Some(Row {
                path: p.clone(),
                depth,
                kinds: project(repos, &p).map(|x| x.kinds.clone()),
                hidden: is_hidden,
            })
        })
        .collect()
}

/// Returns the display label for a project kind.
pub fn kind(k: &ProjectKind) -> &'static str {
    match k {
        ProjectKind::Git => "Git",
        ProjectKind::Python => "Python",
        ProjectKind::Rust => "Rust",
    }
}

/// Finds the project metadata associated with a path.
fn project<'a>(r: &'a [Repository], p: &Path) -> Option<&'a Project> {
    r.iter().flat_map(|x| &x.projects).find(|x| x.path == p)
}

/// Reports whether the selected directory contains a discovered project below it.
fn has_child(s: &AppState) -> bool {
    s.repositories
        .iter()
        .flat_map(|r| &r.projects)
        .any(|p| p.path != s.selected && p.path.starts_with(&s.selected))
}

/// Moves selection through the currently visible tree rows.
fn select(s: &mut AppState, d: i32) {
    let r = rows(&s.home, &s.repositories, &s.expanded, &s.settings, s.show_hidden);
    if let Some(i) = r.iter().position(|x| x.path == s.selected) {
        s.selected = r[(i as i32 + d).clamp(0, r.len() as i32 - 1) as usize].path.clone()
    }
}

/// Collects every directory needed to render the project tree.
fn all_dirs(h: &Path, r: &[Repository]) -> HashSet<PathBuf> {
    let mut s = HashSet::new();
    for x in r.iter().flat_map(|r| &r.projects) {
        let mut p = x.path.clone();
        while p.starts_with(h) {
            s.insert(p.clone());
            if p == h {
                break;
            }
            let Some(q) = p.parent() else { break };
            p = q.to_path_buf()
        }
    }
    s
}

fn initial_expanded(
    home: &Path,
    repositories: &[Repository],
    settings: &Settings,
) -> HashSet<PathBuf> {
    let mut expanded = all_dirs(home, repositories);
    for path in expanded.clone() {
        match settings.display_for(&path) {
            DisplayMode::Hidden | DisplayMode::Expanded => {
                expanded.insert(path);
            }
            DisplayMode::Collapsed => {
                expanded.remove(&path);
            }
        }
    }
    expanded
}

fn toggle_hidden(state: &mut AppState, tx: Sender<Action>) {
    state.show_hidden = !state.show_hidden;
    let visible =
        rows(&state.home, &state.repositories, &state.expanded, &state.settings, state.show_hidden);
    if !visible.iter().any(|row| row.path == state.selected) {
        let mut ancestor = state.selected.parent();
        state.selected = std::iter::from_fn(|| {
            let path = ancestor?;
            ancestor = path.parent();
            Some(path.to_path_buf())
        })
        .find(|path| visible.iter().any(|row| row.path == *path))
        .or_else(|| visible.first().map(|row| row.path.clone()))
        .unwrap_or_else(|| state.home.clone());
        request_git_status(state, tx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::DirectoryOverride;
    use std::{
        fs,
        time::{SystemTime, UNIX_EPOCH},
    };

    struct TempProject(PathBuf);

    impl TempProject {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "canopy-commands-test-{}",
                SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TempProject {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn language_commands_keep_the_existing_defaults() {
        let project = TempProject::new();
        let kinds = [ProjectKind::Git, ProjectKind::Python, ProjectKind::Rust];
        assert_eq!(
            project_commands(&project.0, &kinds, CommandKind::Test),
            vec![vec!["uv", "run", "pytest"], vec!["cargo", "test"]]
        );
        assert_eq!(
            project_commands(&project.0, &kinds, CommandKind::Lint),
            vec![
                vec!["uv", "run", "ruff", "format", "--check"],
                vec!["uv", "run", "ruff", "check"],
                vec!["cargo", "fmt", "--check"],
                vec!["cargo", "clippy"],
            ]
        );
    }

    #[test]
    fn justfile_commands_override_language_defaults_and_work_for_git_projects() {
        let project = TempProject::new();
        fs::write(project.0.join("justfile"), "test:\n    true\nlint:\n    true\n").unwrap();
        for kinds in
            [vec![ProjectKind::Git], vec![ProjectKind::Git, ProjectKind::Python, ProjectKind::Rust]]
        {
            assert_eq!(
                project_commands(&project.0, &kinds, CommandKind::Test),
                vec![vec!["just", "test"]]
            );
            assert_eq!(
                project_commands(&project.0, &kinds, CommandKind::Lint),
                vec![vec!["just", "lint"]]
            );
        }
    }

    #[test]
    fn missing_just_recipe_falls_back_to_language_commands() {
        let project = TempProject::new();
        let kinds = [ProjectKind::Rust];
        fs::write(project.0.join("justfile"), "test:\n    true\n# lint:\nlint-extra:\n    true\n")
            .unwrap();
        assert_eq!(
            project_commands(&project.0, &kinds, CommandKind::Test),
            vec![vec!["just", "test"]]
        );
        assert_eq!(
            project_commands(&project.0, &kinds, CommandKind::Lint),
            vec![vec!["cargo", "fmt", "--check"], vec!["cargo", "clippy"],]
        );

        fs::write(project.0.join("justfile"), "lint:\n    true\ntest-extra:\n    true\n").unwrap();
        assert_eq!(
            project_commands(&project.0, &kinds, CommandKind::Test),
            vec![vec!["cargo", "test"]]
        );
        assert_eq!(
            project_commands(&project.0, &kinds, CommandKind::Lint),
            vec![vec!["just", "lint"]]
        );
    }

    fn tree() -> (PathBuf, Vec<Repository>, PathBuf, PathBuf) {
        let home = PathBuf::from("/home/test");
        let repository = home.join("my-repo");
        let intermediate = repository.join("python_libs");
        let project = intermediate.join("my-lib");
        let repositories = vec![Repository {
            path: repository.clone(),
            projects: vec![
                Project { path: repository.clone(), kinds: vec![ProjectKind::Git] },
                Project { path: project.clone(), kinds: vec![ProjectKind::Python] },
            ],
        }];
        (home, repositories, intermediate, project)
    }

    #[test]
    fn collapsed_override_starts_closed() {
        let (home, repositories, intermediate, _) = tree();
        let mut settings = Settings::default();
        settings.directory_overrides.insert(
            intermediate.clone(),
            DirectoryOverride { display: Some(DisplayMode::Collapsed), ..Default::default() },
        );

        let expanded = initial_expanded(&home, &repositories, &settings);
        assert!(!expanded.contains(&intermediate));
        assert!(expanded.contains(&home.join("my-repo")));
    }

    #[test]
    fn hidden_override_hides_subtree_until_revealed() {
        let (home, repositories, intermediate, project) = tree();
        let mut settings = Settings::default();
        settings.directory_overrides.insert(
            intermediate.clone(),
            DirectoryOverride { display: Some(DisplayMode::Hidden), ..Default::default() },
        );
        let expanded = all_dirs(&home, &repositories);

        let concealed = rows(&home, &repositories, &expanded, &settings, false);
        assert!(!concealed.iter().any(|row| row.path == intermediate || row.path == project));

        let revealed = rows(&home, &repositories, &expanded, &settings, true);
        assert!(revealed.iter().any(|row| row.path == intermediate && row.hidden));
        assert!(revealed.iter().any(|row| row.path == project && !row.hidden));
    }
}
