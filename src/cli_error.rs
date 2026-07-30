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
