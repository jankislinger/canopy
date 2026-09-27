mod action;
mod app;
mod backend;
mod event;
mod ui;

use tokio::sync::mpsc;

#[tokio::main]
/// Initializes the terminal, starts input handling, and runs the application.
async fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;
    let home = backend::home_dir()?;
    let repositories = backend::discover_repositories(&home)?;
    let mut terminal = ratatui::try_init()?;
    let (tx, mut rx) = mpsc::channel(8);
    let listener = tokio::task::spawn_blocking({
        let tx = tx.clone();
        move || event::listen(tx)
    });
    let result = app::run(
        &mut terminal,
        &mut rx,
        tx,
        app::AppState::new(
            home,
            repositories,
            backend::sessions::SessionBackend::default(),
        ),
    )
    .await;
    ratatui::try_restore()?;
    if result.is_ok() {
        listener.await??;
    } else {
        listener.abort();
    }
    result
}
