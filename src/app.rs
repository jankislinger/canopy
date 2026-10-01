use crate::{
    action::Action,
    backend::sessions::{Session, SessionBackend},
    backend::{Project, ProjectKind, Repository},
    ui,
};
use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use ratatui::DefaultTerminal;
use std::{
    collections::{BTreeSet, HashSet},
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
    pub expanded: HashSet<PathBuf>,
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
}
impl AppState {
    /// Creates application state with the discovered tree expanded.
    pub fn new(
        home: PathBuf,
        repositories: Vec<Repository>,
        session_backend: SessionBackend,
    ) -> Self {
        let expanded = all_dirs(&home, &repositories);
        let selected = rows(&home, &repositories, &expanded)
            .first()
            .map(|r| r.path.clone())
            .unwrap_or_else(|| home.clone());
        Self {
            repositories,
            home,
            selected,
            expanded,
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
    let mut tick = time::interval(Duration::from_millis(150));
    loop {
        t.draw(|f| ui::draw(f, &s))?;
        tokio::select! {
            action = rx.recv() => match action {
                Some(Action::Quit) if s.command_popup.is_none() => break,
                Some(Action::Quit) => { s.command_popup = None; s.command_generation += 1; }
                None => {
                    if s.command_popup.is_none() { break; }
                }
                Some(Action::Up) if s.command_popup.is_some() => scroll_popup(&mut s, -1),
                Some(Action::Down) if s.command_popup.is_some() => scroll_popup(&mut s, 1),
                Some(Action::Up) => select(&mut s, -1),
                Some(Action::Down) => select(&mut s, 1),
                Some(Action::Left) => {
                    if s.command_popup.is_none() && has_child(&s) {
                        s.expanded.remove(&s.selected);
                    }
                }
                Some(Action::Right) => {
                    if s.command_popup.is_none() && has_child(&s) {
                        s.expanded.insert(s.selected.clone());
                    }
                }
                Some(Action::Open) => activate(&mut s),
                Some(Action::Start) => start(&mut s),
                Some(Action::Stop) => request_stop(&mut s),
                Some(Action::Confirm) => match s.confirmation.take() {
                    Some(Confirmation::StopSession) => stop(&mut s),
                    None => {}
                }
                Some(Action::Cancel) => {
                    s.confirmation = None;
                    s.command_popup = None;
                    s.command_generation += 1;
                    s.status.clear();
                }
                Some(Action::Resize) => {}
                Some(Action::Test) => run_command(&mut s, false, tx.clone()),
                Some(Action::Lint) => run_command(&mut s, true, tx.clone()),
                Some(Action::CommandFinished { generation, title, output, success }) if generation == s.command_generation => {
                    if let Some(popup) = s.command_popup.as_mut() {
                        popup.title = title;
                        popup.output = output;
                        popup.success = Some(success);
                        if popup.follow { popup.scroll = bottom_scroll(&popup.output); }
                    } else {
                        s.command_popup = Some(CommandPopup { title, output, scroll: 0, follow: true, success: Some(success) });
                    }
                }
                Some(Action::CommandOutput { generation, output }) if generation == s.command_generation => {
                    if let Some(popup) = s.command_popup.as_mut() {
                        popup.output.push_str(&output);
                        if popup.follow { popup.scroll = bottom_scroll(&popup.output); }
                    }
                }
                Some(Action::CommandFinished { .. }) => {}
                Some(Action::Refresh) => {
                    s.project_scan_generation += 1;
                    let generation = s.project_scan_generation;
                    let home = s.home.clone();
                    let result_tx = tx.clone();
                    tokio::spawn(async move {
                        let result = tokio::task::spawn_blocking(move || {
                            crate::backend::discover_repositories(&home)
                        })
                        .await
                        .map_err(|error| error.to_string())
                        .and_then(|result| result.map_err(|error| error.to_string()));
                        let _ = result_tx.send(Action::ProjectsLoaded { generation, result }).await;
                    });
                    s.status = "Scanning projects...".into();
                }
                Some(Action::ProjectsLoaded { generation, result: Ok(repositories) }) if generation == s.project_scan_generation => {
                    let previously_expanded = s.expanded.clone();
                    s.repositories = repositories;
                    let available = all_dirs(&s.home, &s.repositories);
                    s.expanded = previously_expanded
                        .into_iter()
                        .filter(|path| available.contains(path))
                        .collect();
                    s.expanded.insert(s.home.clone());
                    let visible = rows(&s.home, &s.repositories, &s.expanded);
                    if !visible.iter().any(|row| row.path == s.selected) {
                        s.selected = visible
                            .first()
                            .map(|row| row.path.clone())
                            .unwrap_or_else(|| s.home.clone());
                    }
                    s.status = "Projects refreshed".into();
                }
                Some(Action::ProjectsLoaded { generation, result: Err(error) }) if generation == s.project_scan_generation => {
                    s.status = format!("Project scan failed: {error}");
                }
                Some(Action::ProjectsLoaded { .. }) => {}
                Some(Action::Wait) => { s.loading = true; tokio::spawn(wait(tx.clone())); }
                Some(Action::Done) => { s.loading = false; s.completed += 1; }
                Some(Action::CommandOutput { .. }) => {}
            },
            _ = tick.tick(), if s.loading => s.spinner = (s.spinner + 1) % 6,
        }
    }
    Ok(())
}

/// Sends a completion action after the demonstration delay.
async fn wait(tx: Sender<Action>) {
    time::sleep(Duration::from_secs(2)).await;
    let _ = tx.send(Action::Done).await;
}

fn refresh_sessions(state: &mut AppState) {
    match state.session_backend.list() {
        Ok(sessions) => state.sessions = sessions,
        Err(error) => state.status = error.to_string(),
    }
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
        match state.session_backend.create(&state.selected) {
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
        let result = if state.sessions.iter().any(|session| session.name == name) {
            state.session_backend.switch_to(&name)
        } else {
            state
                .session_backend
                .create(&state.selected)
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

fn run_command(state: &mut AppState, lint: bool, tx: Sender<Action>) {
    let Some(project) = project(&state.repositories, &state.selected) else {
        return;
    };
    let path = project.path.clone();
    let kinds = project.kinds.clone();
    let title = if lint { "Lint" } else { "Test" }.to_string();
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
            execute_commands_streaming(&path, &kinds, lint, generation, output_tx)
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

fn execute_commands_streaming(
    path: &Path,
    kinds: &[ProjectKind],
    lint: bool,
    generation: u64,
    tx: Sender<Action>,
) -> (String, bool) {
    let mut commands = Vec::new();
    if kinds.contains(&ProjectKind::Python) {
        commands.push(if lint {
            vec!["uv", "run", "ruff", "format", "--check"]
        } else {
            vec!["uv", "run", "pytest"]
        });
        if lint {
            commands.push(vec!["uv", "run", "ruff", "check"]);
        }
    }
    if kinds.contains(&ProjectKind::Rust) {
        if lint {
            commands.push(vec!["cargo", "fmt", "--check"]);
            commands.push(vec!["cargo", "clippy"]);
        } else {
            commands.push(vec!["cargo", "test"]);
        }
    }
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
}

/// Produces the visible filesystem-tree rows for the discovered projects.
pub fn rows(home: &Path, repos: &[Repository], expanded: &HashSet<PathBuf>) -> Vec<Row> {
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
            if p != home
                && !p.ancestors().skip(1).all(|a| !a.starts_with(home) || expanded.contains(a))
            {
                return None;
            }
            let depth =
                p.strip_prefix(home).map(|x| x.components().count()).unwrap_or(1).saturating_sub(1);
            Some(Row { path: p.clone(), depth, kinds: project(repos, &p).map(|x| x.kinds.clone()) })
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
    let r = rows(&s.home, &s.repositories, &s.expanded);
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
