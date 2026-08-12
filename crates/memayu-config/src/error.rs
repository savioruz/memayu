use thiserror::Error;

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ConfigError {
    #[error("missing required field: {field} (set env var {env_var} or add to config file)")]
    MissingField {
        env_var: &'static str,
        field: &'static str,
    },
    #[error("invalid value for {var}: {value:?} ({detail})")]
    Invalid {
        var: &'static str,
        value: String,
        detail: String,
    },
    #[error("failed to read config file {path}: {detail}")]
    File { path: String, detail: String },
    #[error("failed to parse config file {path}: {detail}")]
    Parse { path: String, detail: String },
}
