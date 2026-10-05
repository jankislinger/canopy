#[derive(Clone, Copy)]
pub enum CommandKind {
    Test,
    Lint,
}

impl CommandKind {
    pub fn title(self) -> &'static str {
        match self {
            Self::Test => "Test",
            Self::Lint => "Lint",
        }
    }

    pub(super) fn recipe(self) -> &'static str {
        match self {
            Self::Test => "test",
            Self::Lint => "lint",
        }
    }
}
