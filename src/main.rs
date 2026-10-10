mod action;
mod app;
mod backend;
mod config;
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
    let (settings, settings_error) = config::Settings::load(&home);
    let mut terminal = ratatui::try_init()?;
    let (tx, mut rx) = mpsc::channel(8);
    let shutdown = Arc::new(AtomicBool::new(false));
    let listener = tokio::task::spawn_blocking({
        let tx = tx.clone();
        let shutdown = shutdown.clone();
        move || event::listen(tx, shutdown)
    });
    let mut state = app::AppState::new(
        home,
        Vec::new(),
        backend::sessions::SessionBackend::default(),
        settings,
        settings_error,
    );
    state.initializing = true;
    let result = state.run(&mut terminal, &mut rx, tx).await;
    ratatui::try_restore()?;
    shutdown.store(true, Ordering::Relaxed);
    listener.await??;
    result
}
