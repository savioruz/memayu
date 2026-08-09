use thiserror::Error;

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ConfigError {
    #[error("missing required env var {0}")]
    Missing(&'static str),
    #[error("invalid value for {var}: {value:?} ({detail})")]
    Invalid {
        var: &'static str,
        value: String,
        detail: String,
    },
}
