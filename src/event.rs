use crate::action::Action;
use crossterm::event::{self, Event};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use tokio::sync::mpsc::Sender;

/// Reads terminal key events and sends recognized actions to the application.
///
/// This blocks until quit is received or the action channel is closed.
pub fn listen(action_tx: Sender<Action>, shutdown: Arc<AtomicBool>) -> color_eyre::Result<()> {
    loop {
        if shutdown.load(Ordering::Relaxed) {
            return Ok(());
        }
        if !event::poll(std::time::Duration::from_millis(100))? {
            continue;
        }
        let action = match event::read()? {
            Event::Key(key) => Action::from_key(key.code),
            Event::Resize(_, _) => Some(Action::Resize),
            _ => None,
        };
        if let Some(action) = action {
            action_tx
                .blocking_send(action)
                .map_err(|_| color_eyre::eyre::eyre!("action receiver closed"))?;
        }
    }
}
