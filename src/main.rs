mod action;
mod app;
mod backend;
mod cli;
mod config;
mod event;
mod fuzzy;
mod ui;

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use tokio::sync::mpsc;

#[tokio::main]
/// Initializes the terminal, starts input handling, and runs the application.
async fn main() -> color_eyre::Result<()> {
    let invocation = match cli::parse(std::env::args().skip(1)) {
        cli::Command::Run(invocation) => invocation,
        cli::Command::Help => {
            print!("{}", cli::HELP);
            return Ok(());
        }
        cli::Command::Version => {
            println!(
                "{}",
                cli::version_line(env!("CANOPY_BUILD_SHA"), env!("CANOPY_BUILD_DIRTY") == "true")
            );
            return Ok(());
        }
        cli::Command::Error(message) => {
            eprint!("{message}\n{}", cli::HELP);
            std::process::exit(2);
        }
    };
    color_eyre::install()?;
    let user_home = backend::home_dir()?;
    let root = match invocation.root {
        Some(raw) => {
            let path = cli::expand_home(&raw, &user_home);
            let path = path
                .canonicalize()
                .map_err(|error| color_eyre::eyre::eyre!("{}: {error}", path.display()))?;
            if !path.is_dir() {
                return Err(color_eyre::eyre::eyre!("{} is not a directory", path.display()));
            }
            path
        }
        None => user_home.clone(),
    };
    let (settings, settings_error) = match invocation.config_file {
        Some(raw) => {
            let path = cli::expand_home(&raw, &user_home);
            (
                config::Settings::load_path(&path, &user_home)
                    .map_err(|error| color_eyre::eyre::eyre!("{error}"))?,
                None,
            )
        }
        None => config::Settings::load(&user_home),
    };
    let mut terminal = ratatui::try_init()?;
    let (tx, mut rx) = mpsc::channel(8);
    let shutdown = Arc::new(AtomicBool::new(false));
    let listener = tokio::task::spawn_blocking({
        let tx = tx.clone();
        let shutdown = shutdown.clone();
        move || event::listen(tx, shutdown)
    });
    let mut state = app::AppState::new(
        root,
        Vec::new(),
        backend::sessions::SessionBackend::default(),
        settings,
        settings_error,
    );
    state.user_home = user_home;
    state.build = backend::update::BuildInfo::current();
    state.initializing = true;
    state.tmux_warning = std::env::var_os("TMUX").is_none();
    if !state.tmux_warning
        && let Err(error) = state.session_backend.prepare_host_session()
    {
        state.status = error.to_string();
    }
    let result = state.run(&mut terminal, &mut rx, tx).await;
    ratatui::try_restore()?;
    shutdown.store(true, Ordering::Relaxed);
    listener.await??;
    result
}
