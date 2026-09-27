use crate::action::Action;
use crossterm::event::{self, Event};
use tokio::sync::mpsc::Sender;

/// Reads terminal key events and sends recognized actions to the application.
///
/// This blocks until quit is received or the action channel is closed.
pub fn listen(action_tx: Sender<Action>) -> color_eyre::Result<()> {
    loop {
        if let Event::Key(key) = event::read()? {
            if let Some(action) = Action::from_key(key.code) {
                let quit = matches!(action, Action::Quit);
                action_tx
                    .blocking_send(action)
                    .map_err(|_| color_eyre::eyre::eyre!("action receiver closed"))?;
                if quit {
                    break;
                }
            }
        }
    }
    Ok(())
}
