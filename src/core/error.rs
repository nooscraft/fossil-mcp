//! Error types for the Fossil toolkit.

use std::path::PathBuf;

/// Result type alias using [`Error`].
pub type Result<T> = std::result::Result<T, Error>;

/// Unified error type for the Fossil toolkit.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("parse error: {message}")]
    Parse {
        message: String,
        file: Option<PathBuf>,
    },

    #[error("unsupported language: {0}")]
    UnsupportedLanguage(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("configuration error: {message}")]
    Config { message: String },

    #[error("analysis error: {message}")]
    Analysis { message: String },

    #[error("rule error: {message}")]
    Rule { message: String },

    #[error("{0}")]
    Other(String),
}

impl Error {
    pub fn parse(message: impl Into<String>) -> Self {
        Error::Parse {
            message: message.into(),
            file: None,
        }
    }

    pub fn parse_in_file(message: impl Into<String>, file: impl Into<PathBuf>) -> Self {
        Error::Parse {
            message: message.into(),
            file: Some(file.into()),
        }
    }

    pub fn config(message: impl Into<String>) -> Self {
        Error::Config {
            message: message.into(),
        }
    }

    pub fn analysis(message: impl Into<String>) -> Self {
        Error::Analysis {
            message: message.into(),
        }
    }

    pub fn rule(message: impl Into<String>) -> Self {
        Error::Rule {
            message: message.into(),
        }
    }

    pub fn other(message: impl Into<String>) -> Self {
        Error::Other(message.into())
    }
}
