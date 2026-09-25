/// All errors produced by the config crate.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("yaml parse error at {path}: {source}")]
    Parse { path: String, source: serde_yaml::Error },

    #[error("validation error: {0}")]
    Validation(String),

    #[error("missing env var '{var}' required by connection '{connection}'")]
    MissingSecret { var: String, connection: String },

    #[error("duplicate connection id '{id}'")]
    DuplicateConnection { id: String },

    #[error("route '{route}' references unknown connection '{connection}'")]
    UnknownConnection { route: String, connection: String },

    #[error("unknown strategy '{strategy}' in route '{route}'")]
    UnknownStrategy { strategy: String, route: String },
}
