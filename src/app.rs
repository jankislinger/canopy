use crate::{
    action::Action,
    backend::{Project, ProjectKind, Repository},
    ui,
};
use ratatui::DefaultTerminal;
use std::{
    collections::{BTreeSet, HashSet},
    path::{Path, PathBuf},
    process::Command,
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
}
impl AppState {
    /// Creates application state with the discovered tree expanded.
    pub fn new(home: PathBuf, repositories: Vec<Repository>) -> Self {
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
        tokio::select! {a=rx.recv()=>match a{Some(Action::Quit)|None=>break,Some(Action::Up)=>select(&mut s,-1),Some(Action::Down)=>select(&mut s,1),Some(Action::Left)=>{if has_child(&s){s.expanded.remove(&s.selected);}},Some(Action::Right)=>{if has_child(&s){s.expanded.insert(s.selected.clone());}},Some(Action::Open)=>{if project(&s.repositories,&s.selected).is_some(){ratatui::try_restore()?;Command::new("nvim").current_dir(&s.selected).status()?;*t=ratatui::try_init()?}},Some(Action::Refresh)=>{let r=crate::backend::discover_repositories(&s.home)?;s=AppState::new(s.home.clone(),r);s.status="Refreshed".into()},Some(Action::Wait)=>{s.loading=true;tokio::spawn(wait(tx.clone()));},Some(Action::Done)=>{s.loading=false;s.completed+=1;}},_=tick.tick(),if s.loading=>s.spinner=(s.spinner+1)%6}
    }
    Ok(())
}

/// Sends a completion action after the demonstration delay.
async fn wait(tx: Sender<Action>) {
    time::sleep(Duration::from_secs(2)).await;
    let _ = tx.send(Action::Done).await;
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
                .unwrap_or(0);
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
