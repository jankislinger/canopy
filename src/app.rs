use crate::backend::runner::{self, CommandCancellation};
use crate::{
    action::Action,
    backend::{Project, ProjectKind, Repository, git::GitStatus},
    backend::{
        commands::CommandKind,
        sessions::{Session, SessionBackend},
    },
    config::{DisplayMode, Settings},
    ui,
};
use ratatui::{DefaultTerminal, widgets::ListState};
use std::{
    collections::{BTreeSet, HashMap, HashSet},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};
use tokio::{
    sync::mpsc::{Receiver, Sender},
    time::{self, Duration},
};
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Confirmation {
    StopSession { name: String, path: PathBuf },
}

pub struct CommandPopup {
    pub title: String,
    pub output: String,
    pub scroll: u16,
    pub follow: bool,
    pub success: Option<bool>,
}

impl CommandPopup {
    fn bottom_offset(&self) -> u16 {
        self.output.lines().count().saturating_sub(18).min(u16::MAX as usize) as u16
    }

    fn scroll_to_bottom(&mut self) {
        self.scroll = self.bottom_offset();
    }

    fn scroll(&mut self, delta: i16) {
        if delta.is_negative() {
            self.scroll = self.scroll.saturating_sub(delta.unsigned_abs());
            self.follow = false;
        } else {
            let bottom = self.bottom_offset();
            self.scroll = self.scroll.saturating_add(delta as u16).min(bottom);
            self.follow = self.scroll == bottom;
        }
    }
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
    pub initializing: bool,
    pub project_scan_pending: bool,
    pub command_popup: Option<CommandPopup>,
    pub command_generation: u64,
    pub command_cancellation: Option<CommandCancellation>,
    pub git_status: Option<GitStatus>,
    pub git_status_cache: HashMap<PathBuf, GitStatus>,
    pub git_status_loading: bool,
    pub git_status_generation: u64,
    pub sessions_refresh_generation: u64,
    pub sessions_refresh_pending: bool,
}
impl AppState {
    /// Creates application state using the configured tree display modes.
    pub fn new(
        home: PathBuf,
        repositories: Vec<Repository>,
        session_backend: SessionBackend,
        settings: Settings,
        settings_error: Option<String>,
    ) -> Self {
        let mut state = Self {
            selected: home.clone(),
            repositories,
            home,
            project_list_state: ListState::default(),
            expanded: HashSet::new(),
            show_hidden: false,
            settings,
            error_popup: settings_error,
            loading: false,
            completed: 0,
            spinner: 0,
            status: String::new(),
            sessions: Vec::new(),
            session_backend,
            confirmation: None,
            project_scan_generation: 0,
            initializing: false,
            project_scan_pending: false,
            command_popup: None,
            command_generation: 0,
            command_cancellation: None,
            git_status: None,
            git_status_cache: HashMap::new(),
            git_status_loading: false,
            git_status_generation: 0,
            sessions_refresh_generation: 0,
            sessions_refresh_pending: false,
        };
        state.expanded = state.initially_expanded_directories();
        state.selected = state
            .visible_rows()
            .first()
            .map(|row| row.path.clone())
            .unwrap_or_else(|| state.home.clone());
        state
    }
    /// Runs the application event loop until the user quits.
    pub async fn run(
        mut self,
        terminal: &mut DefaultTerminal,
        rx: &mut Receiver<Action>,
        tx: Sender<Action>,
    ) -> color_eyre::Result<()> {
        self.request_sessions_refresh(tx.clone());
        if self.initializing {
            let home = self.home.clone();
            let skipped_dirs = self.settings.skipped_dirs.clone();
            let cache_tx = tx.clone();
            tokio::spawn(async move {
                let repositories = tokio::task::spawn_blocking(move || {
                    crate::backend::cache::load(&home, &skipped_dirs)
                })
                .await
                .unwrap_or_default();
                let _ = cache_tx.send(Action::ProjectsCached { repositories }).await;
            });
        } else {
            self.request_git_status(tx.clone());
        }
        let mut tick = time::interval(Duration::from_millis(150));
        let mut sessions_tick = time::interval(Duration::from_secs(1));
        sessions_tick.set_missed_tick_behavior(time::MissedTickBehavior::Skip);
        sessions_tick.tick().await;
        let startup_logo_deadline = time::Instant::now() + Duration::from_secs(2);
        let mut startup_logo_visible = self.initializing;
        loop {
            terminal.draw(|f| {
                if startup_logo_visible {
                    ui::draw_loading(f);
                } else {
                    ui::draw(f, &mut self);
                }
            })?;
            tokio::select! {
                _ = time::sleep_until(startup_logo_deadline), if startup_logo_visible => {
                    startup_logo_visible = false;
                }
                action = rx.recv() => if self.initializing || startup_logo_visible {
                    match action {
                        Some(Action::Quit) | None => break,
                        Some(action) => { let _ = self.handle_background_action(action, tx.clone()); }
                    }
                } else if self.error_popup.is_some() {
                    match action {
                        Some(Action::Quit | Action::Cancel | Action::Open) => self.error_popup = None,
                        Some(action) => { let _ = self.handle_background_action(action, tx.clone()); }
                        None => break,
                    }
                } else if self.confirmation.is_some() {
                    match action {
                        Some(action) => self.handle_confirmation_action(action, tx.clone()),
                        None => break,
                    }
                } else { match (self.command_popup.as_ref(), action) {
                    (None, Some(action)) => match action {
                        Action::Quit => break,
                        Action::Up => {
                            self.move_selection(-1);
                            self.request_git_status(tx.clone());
                        }
                        Action::Down => {
                            self.move_selection(1);
                            self.request_git_status(tx.clone());
                        }
                        Action::Left => {
                            if self.selected_has_children() { self.expanded.remove(&self.selected); }
                        }
                        Action::Right => {
                            if self.selected_has_children() { self.expanded.insert(self.selected.clone()); }
                        }
                        Action::ToggleHidden => self.toggle_hidden(tx.clone()),
                        Action::Open => self.activate_session(),
                        Action::Start => self.start_session(),
                        Action::Stop => self.request_stop_session(),
                        Action::Confirm => {},
                        Action::Cancel => {
                            self.confirmation = None;
                            self.status.clear();
                        }
                        Action::Test => self.run_command(CommandKind::Test, tx.clone()),
                        Action::Lint => self.run_command(CommandKind::Lint, tx.clone()),
                        Action::Refresh => self.request_project_scan(tx.clone()),
                        action => { let _ = self.handle_background_action(action, tx.clone()); }
                    },
                    (Some(popup), Some(Action::Test)) if popup.success.is_some() => {
                        self.run_command(CommandKind::Test, tx.clone());
                    }
                    (Some(popup), Some(Action::Lint)) if popup.success.is_some() => {
                        self.run_command(CommandKind::Lint, tx.clone());
                    }
                    (Some(_), Some(action)) => match action {
                        Action::Quit | Action::Cancel => {
                            self.close_command_popup();
                            self.confirmation = None;
                            self.status.clear();
                        }
                        Action::Up => if let Some(popup) = self.command_popup.as_mut() { popup.scroll(-1); },
                        Action::Down => if let Some(popup) = self.command_popup.as_mut() { popup.scroll(1); },
                        action => { let _ = self.handle_background_action(action, tx.clone()); }
                    },
                    (_, None) => break,
                }},
                _ = sessions_tick.tick() => self.request_sessions_refresh(tx.clone()),
                _ = tick.tick(), if self.loading => self.spinner = (self.spinner + 1) % 6,
            }
        }
        Ok(())
    }

    /// Handles asynchronous events that can arrive with or without an open popup.
    fn handle_background_action(
        &mut self,
        action: Action,
        tx: Sender<Action>,
    ) -> Result<(), Action> {
        match action {
            Action::Resize => {}
            Action::CommandFinished { generation, title, output, success } => {
                if generation == self.command_generation {
                    self.command_cancellation = None;
                    if let Some(popup) = self.command_popup.as_mut() {
                        popup.title = title;
                        popup.output = output;
                        popup.success = Some(success);
                        if popup.follow {
                            popup.scroll_to_bottom();
                        }
                    } else {
                        self.command_popup = Some(CommandPopup {
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
                if generation == self.command_generation
                    && let Some(popup) = self.command_popup.as_mut()
                {
                    popup.output.push_str(&output);
                    if popup.follow {
                        popup.scroll_to_bottom();
                    }
                }
            }
            Action::GitStatusLoaded { generation, path, result } => {
                if generation == self.git_status_generation && path == self.selected {
                    self.git_status_loading = false;
                    if let Ok(status) = result {
                        self.git_status_cache.insert(path, status.clone());
                        self.git_status = Some(status);
                    }
                }
            }
            Action::SessionsLoaded { generation, result } => {
                if generation == self.sessions_refresh_generation {
                    self.sessions_refresh_pending = false;
                    if let Ok(sessions) = result
                        && sessions != self.sessions
                    {
                        self.sessions = sessions;
                    }
                }
            }
            Action::ProjectsCached { repositories } => {
                if let Some(repositories) = repositories {
                    self.repositories = repositories;
                    self.expanded = self.initially_expanded_directories();
                    self.selected = self
                        .visible_rows()
                        .first()
                        .map(|row| row.path.clone())
                        .unwrap_or_else(|| self.home.clone());
                    self.initializing = false;
                    self.request_git_status(tx.clone());
                }
                self.request_project_scan(tx);
            }
            Action::ProjectsLoaded { generation, result } => {
                if generation == self.project_scan_generation {
                    self.project_scan_pending = false;
                    let initial = self.initializing;
                    self.initializing = false;
                    match result {
                        Ok(repositories) => {
                            let previously_expanded = self.expanded.clone();
                            self.repositories = repositories;
                            let available = self.tree_directories();
                            self.expanded = previously_expanded
                                .into_iter()
                                .filter(|path| available.contains(path))
                                .collect();
                            if initial {
                                self.expanded = self.initially_expanded_directories();
                            }
                            self.expanded.insert(self.home.clone());
                            let visible = self.visible_rows();
                            if !visible.iter().any(|row| row.path == self.selected) {
                                self.selected = visible
                                    .first()
                                    .map(|row| row.path.clone())
                                    .unwrap_or_else(|| self.home.clone());
                            }
                            self.request_git_status(tx.clone());
                            self.status = "Projects refreshed".into();
                        }
                        Err(error) => self.status = format!("Project scan failed: {error}"),
                    }
                }
            }
            Action::Wait => {
                self.loading = true;
                tokio::spawn(wait(tx));
            }
            Action::Done => {
                self.loading = false;
                self.completed += 1;
            }
            action => return Err(action),
        }
        Ok(())
    }

    /// Starts a background scan of the project tree.
    fn request_project_scan(&mut self, tx: Sender<Action>) {
        if self.project_scan_pending {
            return;
        }
        self.project_scan_pending = true;
        self.project_scan_generation += 1;
        let generation = self.project_scan_generation;
        let home = self.home.clone();
        let skipped_dirs = self.settings.skipped_dirs.clone();
        tokio::spawn(async move {
            let result = tokio::task::spawn_blocking(move || {
                let repositories = Repository::discover(&home, &skipped_dirs)?;
                let _ = crate::backend::cache::save(&home, &skipped_dirs, &repositories);
                Ok::<_, color_eyre::Report>(repositories)
            })
            .await
            .map_err(|error| error.to_string())
            .and_then(|result| result.map_err(|error| error.to_string()));
            let _ = tx.send(Action::ProjectsLoaded { generation, result }).await;
        });
        self.status = "Scanning projects...".into();
    }

    fn refresh_sessions(&mut self) {
        self.sessions_refresh_generation += 1;
        self.sessions_refresh_pending = false;
        match self.session_backend.list() {
            Ok(sessions) if sessions != self.sessions => self.sessions = sessions,
            Ok(_) => {}
            Err(error) => self.status = error.to_string(),
        }
    }

    /// Refreshes tmux sessions without blocking the application event loop.
    fn request_sessions_refresh(&mut self, tx: Sender<Action>) {
        if self.sessions_refresh_pending {
            return;
        }
        self.sessions_refresh_generation += 1;
        self.sessions_refresh_pending = true;
        let generation = self.sessions_refresh_generation;
        let backend = self.session_backend.clone();
        tokio::spawn(async move {
            let result = tokio::task::spawn_blocking(move || {
                backend.list().map_err(|error| error.to_string())
            })
            .await
            .map_err(|error| error.to_string())
            .and_then(|result| result);
            let _ = tx.send(Action::SessionsLoaded { generation, result }).await;
        });
    }

    fn request_git_status(&mut self, tx: Sender<Action>) {
        self.git_status_generation += 1;
        let generation = self.git_status_generation;
        let path = self.selected.clone();
        if self.project_at(&path).is_none() {
            self.git_status = None;
            self.git_status_loading = false;
            return;
        }
        let cached = self.git_status_cache.get(&path).cloned();
        self.git_status = cached.clone();
        self.git_status_loading = cached.is_none();
        let load_path = path.clone();
        tokio::spawn(async move {
            let result = tokio::task::spawn_blocking(move || GitStatus::load(&load_path))
                .await
                .map_err(|error| error.to_string())
                .and_then(|result| result);
            let _ = tx.send(Action::GitStatusLoaded { generation, path, result }).await;
        });
    }

    fn selected_session_name(&self) -> Option<String> {
        if self.selected_project().is_some() {
            Some(Session::name_for_path(&self.selected))
        } else {
            None
        }
    }

    fn start_session(&mut self) {
        if let Some(name) = self.selected_session_name() {
            let editor = self.settings.editor_for(&self.selected);
            let agent = self.settings.agent_for(&self.selected);
            match self.session_backend.create(&self.selected, editor, agent) {
                Ok(_) => {
                    self.refresh_sessions();
                    self.status = format!("Started {name}");
                }
                Err(error) => self.status = error.to_string(),
            }
        }
    }

    fn activate_session(&mut self) {
        if let Some(name) = self.selected_session_name() {
            let editor = self.settings.editor_for(&self.selected);
            let agent = self.settings.agent_for(&self.selected);
            let result = if self.sessions.iter().any(|session| session.name == name) {
                self.session_backend.switch_to(&name)
            } else {
                self.session_backend
                    .create(&self.selected, editor, agent)
                    .and_then(|_| self.session_backend.switch_to(&name))
            };
            match result {
                Ok(()) => self.status = format!("Switched to {name}"),
                Err(error) => self.status = error.to_string(),
            }
            self.refresh_sessions();
        }
    }

    fn request_stop_session(&mut self) {
        if let Some(name) = self.selected_session_name()
            && self.sessions.iter().any(|session| session.name == name)
        {
            self.confirmation =
                Some(Confirmation::StopSession { name, path: self.selected.clone() });
            self.status = "Stop session? y/n".into();
        }
    }

    fn handle_confirmation_action(&mut self, action: Action, tx: Sender<Action>) {
        match action {
            Action::Confirm => {
                if let Some(Confirmation::StopSession { name, .. }) = self.confirmation.take() {
                    self.stop_session(&name);
                }
            }
            Action::Cancel | Action::Quit => {
                self.confirmation = None;
                self.status.clear();
            }
            action => {
                let _ = self.handle_background_action(action, tx);
            }
        }
    }

    fn stop_session(&mut self, name: &str) {
        match self.session_backend.stop(name) {
            Ok(()) => {
                self.refresh_sessions();
                self.status = format!("Stopped {name}");
            }
            Err(error) => self.status = error.to_string(),
        }
    }

    fn close_command_popup(&mut self) {
        self.command_cancellation = None;
        self.command_popup = None;
        self.command_generation += 1;
    }

    fn run_command(&mut self, command: CommandKind, tx: Sender<Action>) {
        let Some(project) = self.selected_project() else {
            return;
        };
        let project = project.clone();
        let title = command.title().to_string();
        self.command_generation += 1;
        let generation = self.command_generation;
        self.command_popup = Some(CommandPopup {
            title: title.clone(),
            output: "Running...\n".into(),
            scroll: 0,
            follow: true,
            success: None,
        });
        let cancellation = CommandCancellation::new();
        let cancelled = cancellation.flag();
        self.command_cancellation = Some(cancellation);
        let output_tx = tx.clone();
        tokio::spawn(async move {
            let result = tokio::task::spawn_blocking(move || {
                execute_commands_streaming(&project, command, generation, output_tx, &cancelled)
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

    /// Reports whether the selected directory contains a discovered project below it.
    fn selected_has_children(&self) -> bool {
        self.repositories
            .iter()
            .flat_map(|r| &r.projects)
            .any(|p| p.path != self.selected && p.path.starts_with(&self.selected))
    }

    /// Moves selection through the currently visible tree rows.
    fn move_selection(&mut self, delta: i32) {
        let rows = self.visible_rows();
        if let Some(index) = rows.iter().position(|row| row.path == self.selected) {
            let next = (index as i32 + delta).clamp(0, rows.len() as i32 - 1) as usize;
            self.selected = rows[next].path.clone();
        }
    }

    fn toggle_hidden(&mut self, tx: Sender<Action>) {
        self.show_hidden = !self.show_hidden;
        let visible = self.visible_rows();
        if !visible.iter().any(|row| row.path == self.selected) {
            let mut ancestor = self.selected.parent();
            self.selected = std::iter::from_fn(|| {
                let path = ancestor?;
                ancestor = path.parent();
                Some(path.to_path_buf())
            })
            .find(|path| visible.iter().any(|row| row.path == *path))
            .or_else(|| visible.first().map(|row| row.path.clone()))
            .unwrap_or_else(|| self.home.clone());
            self.request_git_status(tx);
        }
    }

    /// Produces the currently visible rows of the project tree.
    pub fn visible_rows(&self) -> Vec<Row> {
        let home = self.home.as_path();
        let set: BTreeSet<_> = self.tree_directories().into_iter().collect();
        set.into_iter()
            .filter_map(|p| {
                if p == home {
                    return None;
                }
                let is_hidden = self.settings.display_for(&p) == DisplayMode::Hidden;
                let has_hidden_ancestor = p
                    .ancestors()
                    .take_while(|ancestor| *ancestor != home)
                    .any(|ancestor| self.settings.display_for(ancestor) == DisplayMode::Hidden);
                if !self.show_hidden && (is_hidden || has_hidden_ancestor) {
                    return None;
                }
                if p != home
                    && !p
                        .ancestors()
                        .skip(1)
                        .all(|a| !a.starts_with(home) || self.expanded.contains(a))
                {
                    return None;
                }
                let depth = p
                    .strip_prefix(home)
                    .map(|x| x.components().count())
                    .unwrap_or(1)
                    .saturating_sub(1);
                Some(Row {
                    path: p.clone(),
                    depth,
                    kinds: self.project_at(&p).map(|x| x.kinds().to_vec()),
                    hidden: is_hidden,
                })
            })
            .collect()
    }

    /// Finds the project metadata associated with a path.
    fn project_at(&self, path: &Path) -> Option<&Project> {
        self.repositories
            .iter()
            .flat_map(|repo| &repo.projects)
            .find(|project| project.path == path)
    }

    fn selected_project(&self) -> Option<&Project> {
        self.project_at(&self.selected)
    }

    /// Collects every directory needed to render the project tree.
    fn tree_directories(&self) -> HashSet<PathBuf> {
        let mut directories = HashSet::new();
        for project in self.repositories.iter().flat_map(|repo| &repo.projects) {
            for path in project.path.ancestors().take_while(|path| path.starts_with(&self.home)) {
                directories.insert(path.to_path_buf());
            }
        }
        directories
    }

    fn initially_expanded_directories(&self) -> HashSet<PathBuf> {
        let mut expanded = self.tree_directories();
        for path in expanded.clone() {
            match self.settings.display_for(&path) {
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
}

/// Sends a completion action after the demonstration delay.
async fn wait(tx: Sender<Action>) {
    time::sleep(Duration::from_secs(2)).await;
    let _ = tx.send(Action::Done).await;
}

fn execute_commands_streaming(
    project: &Project,
    command: CommandKind,
    generation: u64,
    tx: Sender<Action>,
    cancelled: &AtomicBool,
) -> (String, bool) {
    let commands = project.commands(command);
    if commands.is_empty() {
        return ("No test or lint command is defined for this project.".into(), false);
    }
    let mut output = String::new();
    let mut success = true;
    for command in commands {
        if cancelled.load(Ordering::Relaxed) {
            return (output, false);
        }
        let header = format!("$ {}\n", command.join(" "));
        output.push_str(&header);
        if !send_command_output(&tx, generation, header, cancelled) {
            return (output, false);
        }
        match runner::execute(&project.path, &command, cancelled, |chunk| {
            send_command_output(&tx, generation, chunk, cancelled)
        }) {
            Ok((command_output, command_success)) => {
                output.push_str(&command_output);
                success &= command_success;
            }
            Err(error) => {
                let text = format!("failed to start command: {error}\n\n");
                output.push_str(&text);
                success = false;
                if !send_command_output(&tx, generation, text, cancelled) {
                    return (output, false);
                }
            }
        }
    }
    (output, success)
}

fn send_command_output(
    tx: &Sender<Action>,
    generation: u64,
    output: String,
    cancelled: &AtomicBool,
) -> bool {
    let mut action = Action::CommandOutput { generation, output };
    loop {
        if cancelled.load(Ordering::Relaxed) {
            return false;
        }
        match tx.try_send(action) {
            Ok(()) => return true,
            Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => return false,
            Err(tokio::sync::mpsc::error::TrySendError::Full(pending)) => {
                action = pending;
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }
}

pub struct Row {
    pub path: PathBuf,
    pub depth: usize,
    pub kinds: Option<Vec<ProjectKind>>,
    pub hidden: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::DirectoryOverride;

    fn tree() -> (PathBuf, Vec<Repository>, PathBuf, PathBuf) {
        let home = PathBuf::from("/home/test");
        let repository = home.join("my-repo");
        let intermediate = repository.join("python_libs");
        let project = intermediate.join("my-lib");
        let repositories = vec![Repository {
            path: repository.clone(),
            projects: vec![
                Project::new(repository.clone(), vec![ProjectKind::Git], None),
                Project::new(project.clone(), vec![ProjectKind::Python], None),
            ],
        }];
        (home, repositories, intermediate, project)
    }

    #[tokio::test]
    async fn initial_scan_opens_configured_tree_and_recovers_from_failure() {
        let (home, repositories, _, _) = tree();
        let mut state =
            AppState::new(home, Vec::new(), SessionBackend::default(), Settings::default(), None);
        state.initializing = true;
        state.project_scan_pending = true;
        state.project_scan_generation = 1;
        let (tx, _rx) = tokio::sync::mpsc::channel(8);
        state
            .handle_background_action(
                Action::ProjectsLoaded { generation: 0, result: Ok(Vec::new()) },
                tx.clone(),
            )
            .unwrap();
        assert!(state.initializing);
        state
            .handle_background_action(
                Action::ProjectsLoaded { generation: 1, result: Ok(repositories) },
                tx.clone(),
            )
            .unwrap();
        assert!(!state.initializing);
        assert!(!state.project_scan_pending);
        assert_eq!(state.expanded, state.initially_expanded_directories());
        assert!(!state.repositories.is_empty());
        state.initializing = true;
        state
            .handle_background_action(
                Action::ProjectsLoaded { generation: 1, result: Err("unavailable".into()) },
                tx,
            )
            .unwrap();
        assert!(!state.initializing);
        assert!(state.status.contains("unavailable"));
    }

    #[cfg(unix)]
    #[test]
    fn confirmation_blocks_navigation_and_stops_the_original_session() {
        use std::{
            fs,
            os::unix::fs::PermissionsExt,
            time::{SystemTime, UNIX_EPOCH},
        };
        let temp = std::env::temp_dir().join(format!(
            "canopy-confirm-test-{}",
            SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
        ));
        fs::create_dir_all(&temp).unwrap();
        let executable = temp.join("tmux");
        fs::write(&executable, "#!/bin/sh\nif [ \"$1\" = kill-session ]; then printf '%s' \"$3\" > \"$0.stopped\"; fi\n").unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();
        let (home, repositories, _, project) = tree();
        let mut state = AppState::new(
            home,
            repositories,
            SessionBackend::new(executable),
            Settings::default(),
            None,
        );
        state.sessions.push(Session {
            name: "my-repo".into(),
            attached_clients: 0,
            working_directory: state.selected.clone(),
        });
        state.request_stop_session();
        let original = state.selected.clone();
        let (tx, _rx) = tokio::sync::mpsc::channel(8);
        state.handle_confirmation_action(Action::Down, tx.clone());
        state.handle_confirmation_action(Action::Start, tx.clone());
        assert_eq!(state.selected, original);
        assert_eq!(
            state.confirmation,
            Some(Confirmation::StopSession { name: "my-repo".into(), path: original })
        );
        // A background refresh may change selection even while the prompt is open.
        state.selected = project;
        state.handle_confirmation_action(Action::Confirm, tx);
        assert_eq!(fs::read_to_string(temp.join("tmux.stopped")).unwrap(), "my-repo");
        assert!(state.confirmation.is_none());
        fs::remove_dir_all(temp).unwrap();
    }

    #[test]
    fn cancelled_output_delivery_does_not_block_on_a_full_action_channel() {
        let (tx, _rx) = tokio::sync::mpsc::channel(1);
        tx.try_send(Action::Resize).unwrap();
        let cancellation = CommandCancellation::new();
        let flag = cancellation.flag();
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            done_tx.send(send_command_output(&tx, 1, "output".into(), &flag)).unwrap();
        });
        drop(cancellation);
        assert!(!done_rx.recv_timeout(Duration::from_secs(2)).unwrap());
    }

    #[test]
    fn closing_popup_cancels_the_command_and_rejects_late_output() {
        let (home, repositories, _, _) = tree();
        let mut state = AppState::new(
            home,
            repositories,
            SessionBackend::new("/nonexistent-canopy-test-tmux"),
            Settings::default(),
            None,
        );
        let cancellation = CommandCancellation::new();
        let flag = cancellation.flag();
        state.command_cancellation = Some(cancellation);
        state.command_popup = Some(CommandPopup {
            title: "Test".into(),
            output: String::new(),
            scroll: 0,
            follow: true,
            success: None,
        });
        state.close_command_popup();
        assert!(flag.load(Ordering::Relaxed));
        let (tx, _rx) = tokio::sync::mpsc::channel(8);
        state
            .handle_background_action(
                Action::CommandFinished {
                    generation: 0,
                    title: "Test".into(),
                    output: "late".into(),
                    success: true,
                },
                tx,
            )
            .unwrap();
        assert!(state.command_popup.is_none());
    }

    #[test]
    fn collapsed_override_starts_closed() {
        let (home, repositories, intermediate, _) = tree();
        let mut settings = Settings::default();
        settings.directory_overrides.insert(
            intermediate.clone(),
            DirectoryOverride { display: Some(DisplayMode::Collapsed), ..Default::default() },
        );

        let state = AppState::new(
            home.clone(),
            repositories,
            SessionBackend::new("/nonexistent-canopy-test-tmux"),
            settings,
            None,
        );
        let expanded = state.expanded;
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
        let mut state = AppState::new(
            home,
            repositories,
            SessionBackend::new("/nonexistent-canopy-test-tmux"),
            settings,
            None,
        );
        let concealed = state.visible_rows();
        assert!(!concealed.iter().any(|row| row.path == intermediate || row.path == project));

        state.show_hidden = true;
        let revealed = state.visible_rows();
        assert!(revealed.iter().any(|row| row.path == intermediate && row.hidden));
        assert!(revealed.iter().any(|row| row.path == project && !row.hidden));
    }
}
