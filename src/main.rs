mod action;
mod app;
mod backend;
mod event;
mod ui;

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use tokio::sync::mpsc;

#[tokio::main]
/// Initializes the terminal, starts input handling, and runs the application.
async fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;
    let home = backend::home_dir()?;
    let repositories = tokio::task::spawn_blocking({
        let home = home.clone();
        move || backend::discover_repositories(&home)
    })
    .await??;
    let mut terminal = ratatui::try_init()?;
    let (tx, mut rx) = mpsc::channel(8);
    let shutdown = Arc::new(AtomicBool::new(false));
    let listener = tokio::task::spawn_blocking({
        let tx = tx.clone();
        let shutdown = shutdown.clone();
        move || event::listen(tx, shutdown)
    });
    let result = app::run(
        &mut terminal,
        &mut rx,
        tx,
        app::AppState::new(home, repositories, backend::sessions::SessionBackend::default()),
    )
    .await;
    ratatui::try_restore()?;
    shutdown.store(true, Ordering::Relaxed);
    listener.await??;
    result
}
