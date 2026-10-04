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
    ToggleHidden,
    Open,
    Refresh,
    Start,
    Stop,
    Confirm,
    Cancel,
    Resize,
    Test,
    Lint,
    CommandOutput {
        generation: u64,
        output: String,
    },
    CommandFinished {
        generation: u64,
        title: String,
        output: String,
        success: bool,
    },
    GitStatusLoaded {
        generation: u64,
        path: std::path::PathBuf,
        result: Result<crate::backend::git::GitStatus, String>,
    },
    SessionsLoaded {
        generation: u64,
        result: Result<Vec<crate::backend::sessions::Session>, String>,
    },
    ProjectsLoaded {
        generation: u64,
        result: Result<Vec<crate::backend::Repository>, String>,
    },
}

impl Action {
    /// Converts a terminal key into an application action.
    ///
    /// ```
    /// use crossterm::event::KeyCode;
    /// use canopy::action::Action;
    ///
    /// assert!(matches!(Action::from_key(KeyCode::Char('q')), Some(Action::Quit)));
    /// assert!(Action::from_key(KeyCode::Char('z')).is_none());
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
            KeyCode::Char('H') => Self::ToggleHidden,
            KeyCode::Enter => Self::Open,
            KeyCode::Char('s') => Self::Start,
            KeyCode::Char('x') => Self::Stop,
            KeyCode::Char('y') => Self::Confirm,
            KeyCode::Char('n') | KeyCode::Esc => Self::Cancel,
            KeyCode::Char('t') => Self::Test,
            KeyCode::Char('l') => Self::Lint,
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::Action;
    use crossterm::event::KeyCode;

    #[test]
    fn capital_h_toggles_hidden_entries() {
        assert!(matches!(Action::from_key(KeyCode::Char('H')), Some(Action::ToggleHidden)));
    }
}
