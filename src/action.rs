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
    OpenWindowPrefix,
    WindowDigit(usize),
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
    ProjectsCached {
        repositories: Option<Vec<crate::backend::Repository>>,
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
            KeyCode::Char('o') => Self::OpenWindowPrefix,
            KeyCode::Char(digit @ '0'..='9') => Self::WindowDigit(digit as usize - '0' as usize),
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

    #[test]
    fn window_shortcut_recognizes_prefix_and_zero_based_digits() {
        assert!(matches!(Action::from_key(KeyCode::Char('o')), Some(Action::OpenWindowPrefix)));
        for digit in 0..=9 {
            assert!(matches!(Action::from_key(KeyCode::Char(char::from(b'0' + digit))),
                Some(Action::WindowDigit(index)) if index == digit as usize));
        }
    }
}
