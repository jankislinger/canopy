use crossterm::event::KeyCode;

#[derive(Debug)]
pub enum Action {
    Quit,
    Wait,
    Done,
    Up,
    Down,
    Left,
    Right,
    Open,
    Refresh,
}

impl Action {
    /// Converts a terminal key into an application action.
    ///
    /// ```
    /// use crossterm::event::KeyCode;
    /// use learning_tui::action::Action;
    ///
    /// assert!(matches!(Action::from_key(KeyCode::Char('q')), Some(Action::Quit)));
    /// assert!(Action::from_key(KeyCode::Char('x')).is_none());
    /// ```
    pub fn from_key(code: KeyCode) -> Option<Self> {
        Some(match code {
            KeyCode::Char('q') => Self::Quit,
            KeyCode::Char('w') => Self::Wait,
            KeyCode::Char('r') => Self::Refresh,
            KeyCode::Up => Self::Up,
            KeyCode::Down => Self::Down,
            KeyCode::Left => Self::Left,
            KeyCode::Right => Self::Right,
            KeyCode::Enter => Self::Open,
            _ => return None,
        })
    }
}
