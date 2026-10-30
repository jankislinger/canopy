use crate::backend::runner::{self, CommandCancellation};
use crate::{
    action::Action,
    backend::{Project, ProjectKind, Repository, git::GitStatus},
    backend::{
        commands::CommandKind,
        sessions::{Session, SessionBackend},
        update::{BuildInfo, UpdateInfo},
    },
    config::{DisplayMode, Settings},
    ui,
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
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
const UPGRADE_TITLE: &str = "Upgrade";

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

/// fzf-style project search state.
#[derive(Default)]
pub struct FuzzySearch {
    pub query: String,
    pub selected: usize,
}

/// Popup that lists projects with a running tmux session.
pub struct ActiveSessions {
    pub selected: usize,
}

/// A project whose tmux session is currently running.
pub struct ActiveSessionProject {
    pub path: PathBuf,
    pub display: String,
    pub attached_clients: usize,
    pub window_count: usize,
}

pub struct FuzzyResult {
    pub path: PathBuf,
    pub display: String,
    pub positions: Vec<usize>,
}

pub struct RecipePrompt {
    pub path: PathBuf,
    pub justfile: PathBuf,
    pub generation: u64,
    pub listing: Option<String>,
    pub input: String,
    pub cursor: usize,
    pub list_state: ListState,
    pub selected: Option<usize>,
    pub manually_selected: bool,
    pub arguments: String,
    pub argument_cursor: usize,
    pub editing_arguments: bool,
    pub error: Option<String>,
}

impl RecipePrompt {
    pub fn new(path: PathBuf, justfile: PathBuf, generation: u64, listing: Option<String>) -> Self {
        let mut prompt = Self {
            path,
            justfile,
            generation,
            listing,
            input: String::new(),
            cursor: 0,
            list_state: ListState::default(),
            selected: None,
            manually_selected: false,
            arguments: String::new(),
            argument_cursor: 0,
            editing_arguments: false,
            error: None,
        };
        prompt.reconcile_selection();
        prompt
    }

    pub fn filtered_recipes(&self) -> Vec<(usize, crate::backend::just::Recipe)> {
        let mut matches: Vec<_> =
            crate::backend::just::Recipe::parse_listing(self.listing.as_deref().unwrap_or(""))
                .into_iter()
                .enumerate()
                .filter(|(_, recipe)| recipe.matches(&self.input))
                .collect();
        matches.sort_by(|(_, left), (_, right)| left.name.cmp(&right.name));
        matches
    }

    fn reconcile_selection(&mut self) {
        let matches = self.filtered_recipes();
        if self.selected.is_some_and(|selected| matches.iter().any(|(index, _)| *index == selected))
        {
            return;
        }
        self.selected = self
            .selected
            .and_then(|selected| {
                matches.iter().filter(|(index, _)| *index < selected).map(|(index, _)| *index).max()
            })
            .or_else(|| matches.first().map(|(index, _)| *index));
    }

    fn filter_changed(&mut self, previous_matches: &[(usize, crate::backend::just::Recipe)]) {
        let matches = self.filtered_recipes();
        // Exact names win; otherwise a prefix selects the first alphabetical
        // match until the user takes control with arrows.
        if self.input.trim().is_empty() {
            self.manually_selected = false;
        }
        if let Some((index, _)) =
            matches.iter().find(|(_, recipe)| recipe.is_exact_match(&self.input))
        {
            self.selected = Some(*index);
            self.manually_selected = false;
            return;
        }
        if !self.manually_selected
            && let Some((index, _)) =
                matches.iter().find(|(_, recipe)| recipe.is_name_prefix(&self.input))
        {
            self.selected = Some(*index);
            return;
        }
        if self.selected.is_some_and(|selected| matches.iter().any(|(index, _)| *index == selected))
        {
            return;
        }
        self.selected = self
            .selected
            .and_then(|selected| {
                let position = previous_matches.iter().position(|(index, _)| *index == selected)?;
                previous_matches[..position]
                    .iter()
                    .rev()
                    .find(|(index, _)| matches.iter().any(|(survivor, _)| survivor == index))
                    .map(|(index, _)| *index)
            })
            .or_else(|| matches.first().map(|(index, _)| *index));
    }

    fn move_recipe(&mut self, delta: isize) {
        self.reconcile_selection();
        let matches = self.filtered_recipes();
        if let Some(position) = matches.iter().position(|(index, _)| Some(*index) == self.selected)
        {
            let next = position.saturating_add_signed(delta).min(matches.len() - 1);
            self.selected = Some(matches[next].0);
            self.manually_selected = true;
        }
    }

    fn update_listing(&mut self, listing: Option<String>) {
        let previous_matches = self.filtered_recipes();
        let selected_name = self
            .filtered_recipes()
            .into_iter()
            .find(|(index, _)| Some(*index) == self.selected)
            .map(|(_, recipe)| recipe.name);
        self.listing = listing;
        if let Some(name) = selected_name {
            self.selected =
                crate::backend::just::Recipe::parse_listing(self.listing.as_deref().unwrap_or(""))
                    .iter()
                    .position(|recipe| recipe.name == name);
        }
        if self.selected.is_none() {
            self.filter_changed(&previous_matches);
        } else {
            self.reconcile_selection();
        }
    }
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
    pub tmux_warning: bool,
    pub loading: bool,
    pub completed: usize,
    pub spinner: usize,
    pub status: String,
    pub sessions: Vec<Session>,
    pub open_window_pending: bool,
    pub session_backend: SessionBackend,
    pub confirmation: Option<Confirmation>,
    pub project_scan_generation: u64,
    pub initializing: bool,
    pub project_scan_pending: bool,
    pub command_popup: Option<CommandPopup>,
    pub recipe_prompt: Option<RecipePrompt>,
    pub fuzzy: Option<FuzzySearch>,
    pub active_sessions: Option<ActiveSessions>,
    pub command_generation: u64,
    pub command_cancellation: Option<CommandCancellation>,
    pub git_status: Option<GitStatus>,
    pub just_listing: Option<String>,
    pub git_status_cache: HashMap<PathBuf, GitStatus>,
    pub git_status_loading: bool,
    pub git_status_generation: u64,
    pub sessions_refresh_generation: u64,
    pub sessions_refresh_pending: bool,
    pub build: BuildInfo,
    pub update: Option<UpdateInfo>,
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
            tmux_warning: false,
            loading: false,
            completed: 0,
            spinner: 0,
            status: String::new(),
            sessions: Vec::new(),
            open_window_pending: false,
            session_backend,
            confirmation: None,
            project_scan_generation: 0,
            initializing: false,
            project_scan_pending: false,
            command_popup: None,
            recipe_prompt: None,
            fuzzy: None,
            active_sessions: None,
            command_generation: 0,
            command_cancellation: None,
            git_status: None,
            just_listing: None,
            git_status_cache: HashMap::new(),
            git_status_loading: false,
            git_status_generation: 0,
            sessions_refresh_generation: 0,
            sessions_refresh_pending: false,
            build: BuildInfo::default(),
            update: None,
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
            self.request_selected_details(tx.clone());
        }
        if !self.build.sha.is_empty() {
            let home = self.home.clone();
            let build = self.build.clone();
            let update_tx = tx.clone();
            tokio::spawn(async move {
                let update = tokio::task::spawn_blocking(move || {
                    crate::backend::update::check(&home, &build)
                })
                .await
                .unwrap_or_default();
                let _ = update_tx.send(Action::UpdateChecked { update }).await;
            });
        }
        let mut tick = time::interval(Duration::from_millis(150));
        let mut sessions_tick = time::interval(Duration::from_secs(1));
        sessions_tick.set_missed_tick_behavior(time::MissedTickBehavior::Skip);
        sessions_tick.tick().await;
        let startup_logo_deadline = time::Instant::now() + Duration::from_secs(1);
        let mut startup_logo_visible = self.initializing;
        // Actions pulled from the channel while coalescing navigation keys.
        let mut stash: Option<Action> = None;
        // When set, selected-project details are loaded once this instant passes.
        let mut details_deadline: Option<time::Instant> = None;
        loop {
            terminal.draw(|f| {
                if startup_logo_visible {
                    ui::draw_loading(f, &self.build);
                } else {
                    ui::draw(f, &mut self);
                }
            })?;
            tokio::select! {
                _ = time::sleep_until(startup_logo_deadline), if startup_logo_visible => {
                    startup_logo_visible = false;
                }
                _ = async { time::sleep_until(details_deadline.unwrap()).await },
                    if details_deadline.is_some() =>
                {
                    details_deadline = None;
                    self.request_selected_details(tx.clone());
                }
                action = async {
                    match stash.take() {
                        Some(action) => Some(action),
                        None => rx.recv().await,
                    }
                } => {
                    let action = match action {
                        Some(Action::Key(key)) => {
                            if self.recipe_prompt.is_some() {
                                self.handle_recipe_key(key, tx.clone());
                                continue;
                            }
                            if self.fuzzy.is_some() {
                                self.handle_fuzzy_key(key, tx.clone());
                                continue;
                            }
                            if self.active_sessions.is_some() {
                                self.handle_active_sessions_key(key, tx.clone());
                                continue;
                            }
                            let Some(action) = Action::from_key(key.code) else { continue; };
                            if matches!(action, Action::Up | Action::Down)
                                && self.can_coalesce_navigation()
                            {
                                // Merge queued Up/Down presses into one move so a key-repeat
                                // backlog cannot keep scrolling after the key is released.
                                let mut delta = nav_step(&action);
                                while let Ok(next) = rx.try_recv() {
                                    match next {
                                        Action::Key(key)
                                            if matches!(
                                                Action::from_key(key.code),
                                                Some(Action::Up | Action::Down)
                                            ) =>
                                        {
                                            delta += nav_step(&Action::from_key(key.code).unwrap());
                                        }
                                        other => {
                                            stash = Some(other);
                                            break;
                                        }
                                    }
                                }
                                if let Some(popup) = self.command_popup.as_mut() {
                                    popup.scroll(delta.clamp(i16::MIN as i32, i16::MAX as i32) as i16);
                                } else {
                                    self.move_selection(delta);
                                    self.preview_selected_details();
                                    details_deadline = Some(
                                        time::Instant::now() + Duration::from_millis(120),
                                    );
                                }
                                continue;
                            }
                            Some(action)
                        }
                        action => action,
                    };
                    if self.initializing || startup_logo_visible {
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
                } else if self.tmux_warning {
                    match action {
                        Some(Action::Quit) | None => break,
                        Some(Action::Cancel | Action::Open) => self.tmux_warning = false,
                        Some(action) => { let _ = self.handle_background_action(action, tx.clone()); }
                    }
                } else if self.confirmation.is_some() {
                    match action {
                        Some(action) => self.handle_confirmation_action(action, tx.clone()),
                        None => break,
                    }
                } else { match (self.command_popup.as_ref(), action) {
                    (None, Some(action)) => {
                        let Some(action) = self.handle_window_shortcut(action) else { continue; };
                        match action {
                        Action::Quit => break,
                        Action::Up => {
                            self.move_selection(-1);
                            self.request_selected_details(tx.clone());
                        }
                        Action::Down => {
                            self.move_selection(1);
                            self.request_selected_details(tx.clone());
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
                        Action::Upgrade => self.upgrade(tx.clone()),
                        Action::JustPrompt => self.open_recipe_prompt(tx.clone()),
                        Action::Search => self.fuzzy = Some(FuzzySearch::default()),
                        Action::ActiveSessions => self.open_active_sessions(),
                        Action::Refresh => self.request_project_scan(tx.clone()),
                        action => { let _ = self.handle_background_action(action, tx.clone()); }
                    }},
                    (Some(popup), Some(Action::Test)) if popup.success.is_some() => {
                        self.run_command(CommandKind::Test, tx.clone());
                    }
                    (Some(popup), Some(Action::Lint)) if popup.success.is_some() => {
                        self.run_command(CommandKind::Lint, tx.clone());
                    }
                    (Some(popup), Some(Action::JustPrompt)) if popup.success.is_some() => {
                        self.close_command_popup();
                        self.open_recipe_prompt(tx.clone());
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
                }}},
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
            Action::UpdateChecked { update } => self.update = update,
            Action::CommandFinished { generation, title, mut output, success } => {
                if success && title == UPGRADE_TITLE {
                    output.push_str("\nRestart canopy to use the new version.\n");
                }
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
            Action::JustRecipesLoaded { generation, path, result } => {
                if let Some(prompt) = self.recipe_prompt.as_mut()
                    && generation == prompt.generation
                    && path == prompt.path
                {
                    let listing = match &result {
                        Ok(Some(listing)) => Some(listing.clone()),
                        Ok(None) => {
                            prompt.error = Some("No justfile in this project".into());
                            Some(String::new())
                        }
                        Err(error) => {
                            prompt.error = Some(format!("Could not list recipes: {error}"));
                            Some(String::new())
                        }
                    };
                    prompt.update_listing(listing);
                }
                if generation == self.git_status_generation && path == self.selected {
                    self.just_listing = match result {
                        Ok(listing) => listing,
                        Err(error) => Some(format!("  Could not list recipes: {error}")),
                    };
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
                    self.request_selected_details(tx.clone());
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
                            self.request_selected_details(tx.clone());
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

    /// True when Up/Down are plain navigation, with no modal state intercepting them.
    fn can_coalesce_navigation(&self) -> bool {
        !self.initializing
            && self.error_popup.is_none()
            && !self.tmux_warning
            && self.confirmation.is_none()
            && !self.open_window_pending
    }

    /// Cheaply shows cached details for the selection and invalidates in-flight loads.
    fn preview_selected_details(&mut self) {
        self.git_status_generation += 1;
        self.just_listing = None;
        if self.project_at(&self.selected).is_none() {
            self.git_status = None;
            self.git_status_loading = false;
            return;
        }
        let cached = self.git_status_cache.get(&self.selected).cloned();
        self.git_status_loading = cached.is_none();
        self.git_status = cached;
    }

    fn request_selected_details(&mut self, tx: Sender<Action>) {
        self.git_status_generation += 1;
        let generation = self.git_status_generation;
        let path = self.selected.clone();
        self.just_listing = None;
        if self.project_at(&path).is_none() {
            self.git_status = None;
            self.git_status_loading = false;
            return;
        }
        let cached = self.git_status_cache.get(&path).cloned();
        self.git_status = cached.clone();
        self.git_status_loading = cached.is_none();
        let just_path = path.clone();
        let just_tx = tx.clone();
        tokio::spawn(async move {
            let load_path = just_path.clone();
            let result =
                tokio::task::spawn_blocking(move || crate::backend::just::list_recipes(&load_path))
                    .await
                    .map_err(|error| error.to_string())
                    .and_then(|result| result);
            let _ = just_tx
                .send(Action::JustRecipesLoaded { generation, path: just_path, result })
                .await;
        });
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

    pub fn selected_session(&self) -> Option<&Session> {
        let name = self.selected_session_name()?;
        self.sessions.iter().find(|session| session.name == name)
    }

    fn handle_window_shortcut(&mut self, action: Action) -> Option<Action> {
        if self.open_window_pending {
            match action {
                Action::WindowDigit(index) => {
                    self.open_window_pending = false;
                    self.activate_window(index);
                    return None;
                }
                Action::Resize
                | Action::Done
                | Action::SessionsLoaded { .. }
                | Action::GitStatusLoaded { .. }
                | Action::JustRecipesLoaded { .. }
                | Action::ProjectsCached { .. }
                | Action::ProjectsLoaded { .. }
                | Action::UpdateChecked { .. }
                | Action::CommandOutput { .. }
                | Action::CommandFinished { .. } => {}
                _ => {
                    self.open_window_pending = false;
                    self.status.clear();
                }
            }
        }
        match action {
            Action::OpenWindowPrefix => {
                if self.selected_session().is_some() {
                    self.open_window_pending = true;
                    self.status = "Open window: press tmux index 0–9 (Esc cancels)".into();
                } else {
                    self.status = "No running session for this project; Enter creates one".into();
                }
                None
            }
            Action::WindowDigit(_) => None,
            action => Some(action),
        }
    }

    fn activate_window(&mut self, index: usize) {
        let Some(session) = self.selected_session() else {
            self.status = "No running session for this project".into();
            return;
        };
        let Some(window) = session.windows.iter().find(|window| window.index == index) else {
            self.status = format!("No window {index} in {}", session.name);
            return;
        };
        self.status = match self.session_backend.switch_to_window(&session.name, &window.id) {
            Ok(()) => format!("Switched to {}:{index} ({})", session.name, window.name),
            Err(error) => error.to_string(),
        };
        self.refresh_sessions();
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

    fn open_recipe_prompt(&mut self, tx: Sender<Action>) {
        if self.selected_project().is_none() {
            self.status = "Select a project to run Just recipes".into();
            return;
        }
        let Some(justfile) = crate::backend::just::justfile(&self.selected) else {
            self.status = "No justfile in this project".into();
            return;
        };
        let listing = self
            .just_listing
            .clone()
            .filter(|listing| !listing.trim_start().starts_with("Could not list recipes:"));
        self.request_selected_details(tx);
        self.recipe_prompt = Some(RecipePrompt::new(
            self.selected.clone(),
            justfile,
            self.git_status_generation,
            listing,
        ));
    }

    /// Ranks projects (by path relative to Home) against the current query.
    pub fn fuzzy_results(&self) -> Vec<FuzzyResult> {
        let Some(search) = &self.fuzzy else { return Vec::new() };
        let mut results: Vec<(i64, FuzzyResult)> = self
            .repositories
            .iter()
            .flat_map(|repo| &repo.projects)
            .filter(|project| {
                self.show_hidden
                    || !project
                        .path
                        .ancestors()
                        .take_while(|a| a.starts_with(&self.home) && *a != self.home)
                        .any(|a| self.settings.display_for(a) == DisplayMode::Hidden)
            })
            .filter_map(|project| {
                let display = project
                    .path
                    .strip_prefix(&self.home)
                    .unwrap_or(&project.path)
                    .display()
                    .to_string();
                let found = crate::fuzzy::fuzzy_match(&search.query, &display)?;
                Some((
                    found.score,
                    FuzzyResult { path: project.path.clone(), display, positions: found.positions },
                ))
            })
            .collect();
        results.sort_by(|(a, left), (b, right)| {
            b.cmp(a)
                .then(left.display.len().cmp(&right.display.len()))
                .then(left.display.cmp(&right.display))
        });
        results.into_iter().map(|(_, result)| result).collect()
    }

    /// Projects that currently have a tmux session, including hidden ones.
    pub fn active_session_projects(&self) -> Vec<ActiveSessionProject> {
        let mut projects: Vec<_> = self
            .repositories
            .iter()
            .flat_map(|repository| &repository.projects)
            .filter_map(|project| {
                let name = Session::name_for_path(&project.path);
                let session = self.sessions.iter().find(|session| session.name == name)?;
                let display = project
                    .path
                    .strip_prefix(&self.home)
                    .unwrap_or(&project.path)
                    .display()
                    .to_string();
                Some(ActiveSessionProject {
                    path: project.path.clone(),
                    display,
                    attached_clients: session.attached_clients,
                    window_count: session.windows.len(),
                })
            })
            .collect();
        projects.sort_by(|left, right| left.display.cmp(&right.display));
        projects
    }

    fn open_active_sessions(&mut self) {
        let selected = self
            .active_session_projects()
            .iter()
            .position(|project| project.path == self.selected)
            .unwrap_or(0);
        self.active_sessions = Some(ActiveSessions { selected });
    }

    fn handle_active_sessions_key(&mut self, key: KeyEvent, tx: Sender<Action>) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if key.code == KeyCode::Esc
            || key.code == KeyCode::Char('q')
            || (ctrl && matches!(key.code, KeyCode::Char('c' | 'g')))
        {
            self.active_sessions = None;
            return;
        }
        let count = self.active_session_projects().len();
        let Some(popup) = self.active_sessions.as_mut() else { return };
        if count == 0 {
            return;
        }
        popup.selected = popup.selected.min(count - 1);
        match key.code {
            KeyCode::Up | KeyCode::BackTab => popup.selected = popup.selected.saturating_sub(1),
            KeyCode::Down | KeyCode::Tab => {
                popup.selected = (popup.selected + 1).min(count - 1);
            }
            KeyCode::Enter => {}
            _ => return,
        }
        if key.code != KeyCode::Enter {
            return;
        }
        let index = self.active_sessions.as_ref().map(|popup| popup.selected).unwrap_or(0);
        let path =
            self.active_session_projects().into_iter().nth(index).map(|project| project.path);
        self.active_sessions = None;
        if let Some(path) = path {
            let concealed = path
                .ancestors()
                .take_while(|ancestor| ancestor.starts_with(&self.home) && *ancestor != self.home)
                .any(|ancestor| self.settings.display_for(ancestor) == DisplayMode::Hidden);
            if concealed {
                self.show_hidden = true;
            }
            self.select_path(path, tx);
            self.activate_session();
        }
    }

    fn handle_fuzzy_key(&mut self, key: KeyEvent, tx: Sender<Action>) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let count = self.fuzzy_results().len();
        let Some(search) = self.fuzzy.as_mut() else { return };
        match key.code {
            KeyCode::Esc => self.fuzzy = None,
            KeyCode::Char('c' | 'g') if ctrl => self.fuzzy = None,
            KeyCode::Enter => {
                let index = search.selected;
                let target = self.fuzzy_results().into_iter().nth(index);
                self.fuzzy = None;
                if let Some(target) = target {
                    self.select_path(target.path, tx);
                }
            }
            KeyCode::Up | KeyCode::BackTab => search.selected = search.selected.saturating_sub(1),
            KeyCode::Char('p' | 'k') if ctrl => search.selected = search.selected.saturating_sub(1),
            KeyCode::Down | KeyCode::Tab => {
                search.selected = (search.selected + 1).min(count.saturating_sub(1));
            }
            KeyCode::Char('n' | 'j') if ctrl => {
                search.selected = (search.selected + 1).min(count.saturating_sub(1));
            }
            KeyCode::Char('u') if ctrl => {
                search.query.clear();
                search.selected = 0;
            }
            KeyCode::Char('w') if ctrl => {
                let trimmed = search.query.trim_end().len();
                let start = search.query[..trimmed].rfind(' ').map_or(0, |i| i + 1);
                search.query.truncate(start);
                search.selected = 0;
            }
            KeyCode::Backspace => {
                search.query.pop();
                search.selected = 0;
            }
            KeyCode::Char(character)
                if !key.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                search.query.push(character);
                search.selected = 0;
            }
            _ => {}
        }
    }

    /// Selects a project, expanding its ancestors so it is visible in the tree.
    fn select_path(&mut self, path: PathBuf, tx: Sender<Action>) {
        for ancestor in path.ancestors().skip(1).take_while(|a| a.starts_with(&self.home)) {
            self.expanded.insert(ancestor.to_path_buf());
        }
        self.selected = path;
        self.request_selected_details(tx);
    }

    fn handle_recipe_key(&mut self, key: KeyEvent, tx: Sender<Action>) {
        if key.code == KeyCode::Esc
            || (key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL))
        {
            self.recipe_prompt = None;
            return;
        }
        if key.code == KeyCode::Enter {
            self.submit_recipe(tx);
            return;
        }
        let Some(prompt) = self.recipe_prompt.as_mut() else {
            return;
        };
        prompt.error = None;
        let previous_matches = prompt.filtered_recipes();
        let previous_input = prompt.input.clone();
        match key.code {
            KeyCode::Up => {
                prompt.move_recipe(-1);
                return;
            }
            KeyCode::Down => {
                prompt.move_recipe(1);
                return;
            }
            KeyCode::Tab | KeyCode::BackTab => {
                prompt.editing_arguments = !prompt.editing_arguments;
                return;
            }
            _ => {}
        }
        let (input, cursor) = if prompt.editing_arguments {
            (&mut prompt.arguments, &mut prompt.argument_cursor)
        } else {
            (&mut prompt.input, &mut prompt.cursor)
        };
        match key.code {
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                input.clear();
                *cursor = 0;
            }
            KeyCode::Home => *cursor = 0,
            KeyCode::End => *cursor = input.len(),
            KeyCode::Left => {
                *cursor = input[..*cursor]
                    .char_indices()
                    .next_back()
                    .map(|(index, _)| index)
                    .unwrap_or(0);
            }
            KeyCode::Right => {
                if let Some(character) = input[*cursor..].chars().next() {
                    *cursor += character.len_utf8();
                }
            }
            KeyCode::Backspace => {
                if let Some((index, _)) = input[..*cursor].char_indices().next_back() {
                    input.drain(index..*cursor);
                    *cursor = index;
                }
            }
            KeyCode::Delete => {
                if let Some(character) = input[*cursor..].chars().next() {
                    input.drain(*cursor..*cursor + character.len_utf8());
                }
            }
            KeyCode::Char(character)
                if !key.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                input.insert(*cursor, character);
                *cursor += character.len_utf8();
            }
            _ => {}
        }
        if prompt.input != previous_input {
            prompt.filter_changed(&previous_matches);
        }
    }

    fn submit_recipe(&mut self, tx: Sender<Action>) {
        let Some(prompt) = self.recipe_prompt.as_mut() else {
            return;
        };
        prompt.reconcile_selection();
        let Some((_, recipe)) = prompt
            .filtered_recipes()
            .into_iter()
            .find(|(index, _)| Some(*index) == prompt.selected)
        else {
            prompt.error = Some("No matching recipe selected".into());
            return;
        };
        let input = format!("{} {}", shell_words::quote(&recipe.name), prompt.arguments);
        let command = match crate::backend::just::recipe_command(&prompt.justfile, &input) {
            Ok(command) => command,
            Err(error) => {
                prompt.error = Some(error);
                return;
            }
        };
        let prompt = self.recipe_prompt.take().unwrap();
        let title = format!("Just: {}", input.trim_end());
        self.run_commands(prompt.path, title, vec![command], tx);
    }

    fn run_command(&mut self, command: CommandKind, tx: Sender<Action>) {
        let Some(project) = self.selected_project() else {
            return;
        };
        let project = project.clone();
        let title = command.title().to_string();
        let commands = project
            .commands(command)
            .iter()
            .map(|command| command.iter().map(|arg| (*arg).to_owned()).collect())
            .collect();
        self.run_commands(project.path, title, commands, tx);
    }

    /// Reinstalls the latest master. Dev builds only show the hint, since installing would
    /// replace the central binary.
    fn upgrade(&mut self, tx: Sender<Action>) {
        if self.update.is_none() || self.build.dev {
            return;
        }
        use crate::backend::update::{BRANCH, REPOSITORY_URL};
        let command = ["cargo", "install", "--locked", "--git", REPOSITORY_URL, "--branch", BRANCH];
        let commands = vec![command.iter().map(|arg| (*arg).to_owned()).collect()];
        self.run_commands(self.home.clone(), UPGRADE_TITLE.into(), commands, tx);
    }

    fn run_commands(
        &mut self,
        path: PathBuf,
        title: String,
        commands: Vec<Vec<String>>,
        tx: Sender<Action>,
    ) {
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
                execute_commands_streaming(&path, &commands, generation, output_tx, &cancelled)
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
            self.request_selected_details(tx);
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

fn nav_step(action: &Action) -> i32 {
    if matches!(action, Action::Up) { -1 } else { 1 }
}

/// Sends a completion action after the demonstration delay.
async fn wait(tx: Sender<Action>) {
    time::sleep(Duration::from_secs(2)).await;
    let _ = tx.send(Action::Done).await;
}

fn execute_commands_streaming(
    path: &Path,
    commands: &[Vec<String>],
    generation: u64,
    tx: Sender<Action>,
    cancelled: &AtomicBool,
) -> (String, bool) {
    if commands.is_empty() {
        return ("No test or lint command is defined for this project.".into(), false);
    }
    let mut output = String::new();
    let mut success = true;
    for command in commands {
        if cancelled.load(Ordering::Relaxed) {
            return (output, false);
        }
        let header = format!("$ {}\n", shell_words::join(command));
        output.push_str(&header);
        if !send_command_output(&tx, generation, header, cancelled) {
            return (output, false);
        }
        let args: Vec<_> = command.iter().map(String::as_str).collect();
        match runner::execute(path, &args, cancelled, |chunk| {
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
    async fn active_sessions_popup_lists_running_projects_and_selects_one() {
        let (home, repositories, intermediate, project) = tree();
        let repository = home.join("my-repo");
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
        state.selected = repository.clone();
        state.sessions = vec![
            Session {
                name: Session::name_for_path(&repository),
                attached_clients: 0,
                working_directory: repository.clone(),
                windows: vec![],
            },
            Session {
                name: Session::name_for_path(&project),
                attached_clients: 2,
                working_directory: project.clone(),
                windows: vec![crate::backend::sessions::Window {
                    index: 1,
                    id: "@1".into(),
                    name: "editor".into(),
                    active: true,
                    pane_count: 1,
                    command: "nvim".into(),
                    working_directory: project.clone(),
                }],
            },
            Session {
                name: "unrelated".into(),
                attached_clients: 1,
                working_directory: PathBuf::from("/tmp/other"),
                windows: vec![],
            },
        ];
        state.open_active_sessions();
        assert_eq!(
            state
                .active_session_projects()
                .iter()
                .map(|project| project.display.as_str())
                .collect::<Vec<_>>(),
            ["my-repo", "my-repo/python_libs/my-lib"]
        );
        assert_eq!(state.active_sessions.as_ref().unwrap().selected, 0);
        let (tx, _rx) = tokio::sync::mpsc::channel(8);
        state.handle_active_sessions_key(
            KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
            tx.clone(),
        );
        assert_eq!(state.active_sessions.as_ref().unwrap().selected, 1);
        state.handle_active_sessions_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), tx);
        assert!(state.active_sessions.is_none());
        assert_eq!(state.selected, project);
        assert!(state.show_hidden);
    }

    #[test]
    fn window_shortcut_preserves_tmux_indices_and_cancels_on_navigation() {
        let (home, repositories, _, _) = tree();
        let mut state = AppState::new(
            home,
            repositories,
            SessionBackend::new("/nonexistent-canopy-test-tmux"),
            Settings::default(),
            None,
        );
        state.sessions.push(Session {
            name: Session::name_for_path(&state.selected),
            attached_clients: 0,
            working_directory: state.selected.clone(),
            windows: vec![crate::backend::sessions::Window {
                index: 3,
                id: "@42".into(),
                name: "agent".into(),
                active: true,
                pane_count: 1,
                command: "codex".into(),
                working_directory: state.selected.clone(),
            }],
        });
        assert!(state.handle_window_shortcut(Action::OpenWindowPrefix).is_none());
        assert!(state.open_window_pending);
        // Background updates must not consume the prefix.
        assert!(matches!(
            state.handle_window_shortcut(Action::SessionsLoaded {
                generation: 0,
                result: Ok(Vec::new()),
            }),
            Some(Action::SessionsLoaded { .. })
        ));
        assert!(state.open_window_pending);
        assert!(matches!(state.handle_window_shortcut(Action::Done), Some(Action::Done)));
        assert!(state.open_window_pending);
        state.handle_window_shortcut(Action::WindowDigit(1));
        assert!(!state.open_window_pending);
        assert!(state.status.starts_with("No window 1"));
        state.handle_window_shortcut(Action::OpenWindowPrefix);
        assert!(matches!(state.handle_window_shortcut(Action::Down), Some(Action::Down)));
        assert!(!state.open_window_pending);
        assert!(state.status.is_empty());
        state.handle_window_shortcut(Action::OpenWindowPrefix);
        assert!(matches!(state.handle_window_shortcut(Action::Cancel), Some(Action::Cancel)));
        assert!(!state.open_window_pending);
        state.sessions.clear();
        state.handle_window_shortcut(Action::OpenWindowPrefix);
        assert!(!state.open_window_pending);
        assert!(state.status.starts_with("No running session"));
    }

    #[test]
    fn just_listing_rejects_stale_results_and_clears_when_justfile_is_missing() {
        let (home, repositories, _, _) = tree();
        let mut state =
            AppState::new(home, repositories, SessionBackend::default(), Settings::default(), None);
        state.git_status_generation = 5;
        state.just_listing = Some("current".into());
        let (tx, _rx) = tokio::sync::mpsc::channel(8);
        for (generation, path) in [(4, state.selected.clone()), (5, state.home.join("other"))] {
            state
                .handle_background_action(
                    Action::JustRecipesLoaded {
                        generation,
                        path,
                        result: Ok(Some("stale".into())),
                    },
                    tx.clone(),
                )
                .unwrap();
            assert_eq!(state.just_listing.as_deref(), Some("current"));
        }
        state
            .handle_background_action(
                Action::JustRecipesLoaded {
                    generation: 5,
                    path: state.selected.clone(),
                    result: Ok(None),
                },
                tx,
            )
            .unwrap();
        assert!(state.just_listing.is_none());
    }

    #[test]
    fn recipe_prompt_edits_text_instead_of_triggering_project_shortcuts() {
        let (home, repositories, _, _) = tree();
        let mut state =
            AppState::new(home, repositories, SessionBackend::default(), Settings::default(), None);
        let selected = state.selected.clone();
        state.recipe_prompt = Some(RecipePrompt::new(
            selected.clone(),
            selected.join("justfile"),
            0,
            Some("  test\n  lint".into()),
        ));
        let (tx, _rx) = tokio::sync::mpsc::channel(8);
        for character in "qtlé".chars() {
            state.handle_recipe_key(
                KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE),
                tx.clone(),
            );
        }
        for code in [
            KeyCode::Left,
            KeyCode::Backspace,
            KeyCode::Char('-'),
            KeyCode::Right,
            KeyCode::Home,
            KeyCode::Delete,
        ] {
            state.handle_recipe_key(KeyEvent::new(code, KeyModifiers::NONE), tx.clone());
        }
        assert_eq!(state.recipe_prompt.as_ref().unwrap().input, "t-é");
        state.handle_recipe_key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE), tx.clone());
        state.handle_recipe_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE), tx.clone());
        assert_eq!(state.recipe_prompt.as_ref().unwrap().input, "t-");
        state.handle_recipe_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE), tx.clone());
        assert_eq!(state.selected, selected);
        assert!(state.recipe_prompt.as_ref().unwrap().selected.is_none());
        state.handle_recipe_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), tx.clone());
        assert!(state.recipe_prompt.as_ref().unwrap().error.is_some());
        assert!(state.command_popup.is_none());
        state.handle_recipe_key(
            KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL),
            tx.clone(),
        );
        state.handle_recipe_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE), tx.clone());
        assert_eq!(state.recipe_prompt.as_ref().unwrap().selected, Some(0));
        state.handle_recipe_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE), tx.clone());
        state.handle_recipe_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE), tx.clone());
        assert_eq!(state.recipe_prompt.as_ref().unwrap().arguments, "x");
        assert!(state.recipe_prompt.as_ref().unwrap().input.is_empty());
        state.handle_recipe_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE), tx);
        assert!(state.recipe_prompt.is_none());
    }

    #[test]
    fn recipe_picker_uses_stable_order_and_name_highlights() {
        let scenarios = [
            (
                "  check # Run tests and lint\n  lint # Lint\n  test # Run tests",
                "te",
                0,
                0,
                Some("test"),
            ),
            ("  check # Run tests and lint\n  test # Run tests", "test", 0, 0, Some("test")),
            (
                "  check # Run tests and lint\n  lint # Lint\n  test # Run tests",
                "t",
                0,
                0,
                Some("test"),
            ),
            ("  check # Run tests\n  test-unit\n  test", "t", 0, 1, Some("test-unit")),
            ("  test-unit\n  test\n  test-integration", "test", 0, 1, Some("test-integration")),
            (
                "  build # Compile application\n  check # Run tests and lint\n  test # Run tests",
                "run tests",
                0,
                0,
                Some("check"),
            ),
            (
                "  alpha # apples\n  bravo # bananas\n  charlie # apples\n  delta # bananas\n  echo # apples",
                "apls",
                3,
                0,
                Some("charlie"),
            ),
            ("  deploy target\n  deploy-preview", "deploy staging", 0, 0, None),
        ];
        for (listing, query, before, after, expected) in scenarios {
            let (home, repositories, _, _) = tree();
            let mut state = AppState::new(
                home,
                repositories,
                SessionBackend::default(),
                Settings::default(),
                None,
            );
            state.recipe_prompt = Some(RecipePrompt::new(
                state.selected.clone(),
                state.selected.join("justfile"),
                0,
                Some(listing.into()),
            ));
            let (tx, _rx) = tokio::sync::mpsc::channel(8);
            let down = KeyEvent::new(KeyCode::Down, KeyModifiers::NONE);
            for _ in 0..before {
                state.handle_recipe_key(down, tx.clone());
            }
            for character in query.chars() {
                state.handle_recipe_key(
                    KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE),
                    tx.clone(),
                );
            }
            for _ in 0..after {
                state.handle_recipe_key(down, tx.clone());
            }
            // Moving the text cursor must not undo a manual recipe selection.
            state.handle_recipe_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE), tx.clone());
            let prompt = state.recipe_prompt.as_ref().unwrap();
            let matches = prompt.filtered_recipes();
            assert!(
                matches.windows(2).all(|pair| pair[0].1.name <= pair[1].1.name),
                "filter must not reorder matches by relevance: {query}"
            );
            let selected = matches.iter().find(|(index, _)| Some(*index) == prompt.selected);
            assert_eq!(
                selected.map(|(_, recipe)| recipe.name.as_str()),
                expected,
                "query: {query}"
            );
            if let Some(expected) = expected {
                assert_eq!(
                    crate::backend::just::recipe_command(
                        &prompt.justfile,
                        &selected.unwrap().1.name
                    )
                    .unwrap()
                    .last()
                    .unwrap(),
                    expected
                );
            } else {
                state.handle_recipe_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), tx);
                assert!(state.command_popup.is_none());
                assert!(state.recipe_prompt.as_ref().unwrap().error.is_some());
            }
        }
    }

    #[test]
    fn exact_match_is_highlighted_when_recipes_finish_loading() {
        let mut prompt = RecipePrompt::new(
            PathBuf::from("/tmp/project"),
            PathBuf::from("/tmp/project/justfile"),
            0,
            None,
        );
        prompt.input = "TEST".into();
        prompt.cursor = prompt.input.len();
        prompt.update_listing(Some("  check # Run tests\n  test # Run tests".into()));
        assert_eq!(
            prompt
                .filtered_recipes()
                .iter()
                .map(|(_, recipe)| recipe.name.as_str())
                .collect::<Vec<_>>(),
            ["check", "test"]
        );
        assert_eq!(prompt.selected, Some(1));
        prompt.move_recipe(-1);
        prompt.update_listing(Some("  check # Run tests\n  test # Run tests".into()));
        assert_eq!(prompt.selected, Some(0)); // Refresh must not undo arrow navigation.
    }

    #[test]
    fn filtering_preserves_selection_or_moves_to_nearest_surviving_recipe_above() {
        let mut prompt = RecipePrompt::new(PathBuf::from("/tmp/project"), PathBuf::from("/tmp/project/justfile"), 0,
            Some("  alpha # apples\n  bravo # bananas\n  charlie # apples\n  delta # bananas\n  echo # apples".into()));
        for _ in 0..3 {
            prompt.move_recipe(1);
        }
        assert_eq!(prompt.selected, Some(3));
        prompt.input = "APLS".into();
        prompt.reconcile_selection();
        assert_eq!(
            prompt.filtered_recipes().iter().map(|(index, _)| *index).collect::<Vec<_>>(),
            [0, 2, 4]
        );
        assert_eq!(prompt.selected, Some(2));
        prompt.input = "apls chrl".into();
        prompt.reconcile_selection();
        assert_eq!(prompt.selected, Some(2));
        prompt.input = "ech".into();
        prompt.reconcile_selection();
        assert_eq!(prompt.selected, Some(4)); // No match above: first surviving match.
        prompt.input.clear();
        prompt.reconcile_selection();
        assert_eq!(prompt.selected, Some(4)); // Broadening preserves the active recipe.
        prompt.input = "no recipe matches this".into();
        prompt.reconcile_selection();
        assert!(prompt.selected.is_none());
        prompt.move_recipe(-1);
        assert!(prompt.selected.is_none());
        prompt.input.clear();
        prompt.reconcile_selection();
        assert_eq!(prompt.selected, Some(0));
        prompt.move_recipe(-1);
        assert_eq!(prompt.selected, Some(0));
        prompt.move_recipe(1);
        assert_eq!(prompt.selected, Some(1));
        prompt.update_listing(Some(
            "  alpha # apples\n  beta # new recipe\n  bravo # bananas".into(),
        ));
        assert_eq!(prompt.selected, Some(2)); // Refreshes preserve recipe identity.
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
            windows: Vec::new(),
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
