use super::USAGE;
use crate::native_cli::NativeCliError;
use std::fmt;

#[derive(Debug)]
pub enum CliError {
    Usage(String),
    Core(higher_graphen_core::CoreError),
    Native(NativeCliError),
    Json(serde_json::Error),
}

impl CliError {
    pub(crate) fn usage(message: impl Into<String>) -> Self {
        Self::Usage(message.into())
    }

    /// The stable, machine-readable classification for this refusal. One
    /// match over the variants that already exist, exactly like
    /// `NativeCliError::error_code`, which `Native` delegates to rather
    /// than re-deciding.
    pub(crate) fn error_code(&self) -> &'static str {
        match self {
            Self::Usage(_) => "usage",
            Self::Core(error) => error.code(),
            Self::Native(error) => error.error_code(),
            Self::Json(_) => "invalid",
        }
    }

    /// Structured recovery data alongside the message, when there is any.
    /// Delegates to `NativeCliError::refusal_data` for `Native`; the
    /// top-level variants (a command segment error, a JSON parse failure
    /// before any command was even identified) carry none.
    pub(crate) fn refusal_data(&self) -> Option<serde_json::Value> {
        match self {
            Self::Native(error) => error.refusal_data(),
            _ => None,
        }
    }

    /// The refusal's message alone, without `Display`'s appended usage
    /// block. The full usage text belongs in a human reading `--format
    /// text` on a terminal, not repeated inside every JSON refusal's
    /// `message` field.
    pub(crate) fn refusal_message(&self) -> String {
        match self {
            Self::Usage(message) => message.clone(),
            other => other.to_string(),
        }
    }
}

impl From<higher_graphen_core::CoreError> for CliError {
    fn from(error: higher_graphen_core::CoreError) -> Self {
        Self::Core(error)
    }
}

impl From<NativeCliError> for CliError {
    fn from(error: NativeCliError) -> Self {
        Self::Native(error)
    }
}

impl From<serde_json::Error> for CliError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Usage(message) => write!(formatter, "{message}\n{USAGE}"),
            Self::Core(error) => write!(formatter, "{error}"),
            Self::Native(error) => write!(formatter, "{error}"),
            Self::Json(error) => write!(formatter, "{error}"),
        }
    }
}
