use crate::{
    action::Action,
    backend::sessions::{Session, SessionBackend},
    backend::{Project, ProjectKind, Repository},
    ui,
};
use ratatui::DefaultTerminal;
use std::{
    collections::{BTreeSet, HashSet},
    path::{Path, PathBuf},
};
use tokio::{
    sync::mpsc::{Receiver, Sender},
    time::{self, Duration},
};
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
    pub stop_confirmation: bool,
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
            stop_confirmation: false,
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
        tokio::select! {a=rx.recv()=>match a{Some(Action::Quit)|None=>break,Some(Action::Up)=>select(&mut s,-1),Some(Action::Down)=>select(&mut s,1),Some(Action::Left)=>{if has_child(&s){s.expanded.remove(&s.selected);}},Some(Action::Right)=>{if has_child(&s){s.expanded.insert(s.selected.clone());}},Some(Action::Open)=>activate(&mut s),Some(Action::Start)=>start(&mut s),Some(Action::Stop)=>request_stop(&mut s),Some(Action::Confirm)=>stop(&mut s),Some(Action::Cancel)=>{s.stop_confirmation=false;s.status.clear()},Some(Action::Refresh)=>{let home=s.home.clone();let result_tx=tx.clone();tokio::spawn(async move{let result=tokio::task::spawn_blocking(move||crate::backend::discover_repositories(&home)).await.map_err(|error|error.to_string()).and_then(|result|result.map_err(|error|error.to_string()));let _=result_tx.send(Action::ProjectsLoaded(result)).await;});s.status="Scanning projects...".into()},Some(Action::ProjectsLoaded(Ok(repositories)))=>{s.repositories=repositories;s.expanded=all_dirs(&s.home,&s.repositories);let visible=rows(&s.home,&s.repositories,&s.expanded);if !visible.iter().any(|row|row.path==s.selected){s.selected=visible.first().map(|row|row.path.clone()).unwrap_or_else(||s.home.clone())}s.status="Projects refreshed".into()},Some(Action::ProjectsLoaded(Err(error)))=>s.status=format!("Project scan failed: {error}"),Some(Action::Wait)=>{s.loading=true;tokio::spawn(wait(tx.clone()));},Some(Action::Done)=>{s.loading=false;s.completed+=1;}},_=tick.tick(),if s.loading=>s.spinner=(s.spinner+1)%6}
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
        .and_then(|name| {
            state
                .sessions
                .iter()
                .find(|session| session.name == name)
                .map(|_| name)
        })
        .is_some()
    {
        state.stop_confirmation = true;
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
    state.stop_confirmation = false;
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
                && !p
                    .ancestors()
                    .skip(1)
                    .all(|a| !a.starts_with(home) || expanded.contains(a))
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
                kinds: project(repos, &p).map(|x| x.kinds.clone()),
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
    let r = rows(&s.home, &s.repositories, &s.expanded);
    if let Some(i) = r.iter().position(|x| x.path == s.selected) {
        s.selected = r[(i as i32 + d).clamp(0, r.len() as i32 - 1) as usize]
            .path
            .clone()
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
