use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("network: {0}")]
    Network(#[from] reqwest::Error),

    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("json: {0}")]
    Json(#[from] serde_json::Error),

    #[error("checksum mismatch: expected {expected}, got {actual}")]
    Checksum { expected: String, actual: String },

    #[error("unsupported platform: {0}")]
    UnsupportedPlatform(String),

    #[error("manifest missing platform '{0}'")]
    ManifestMissingPlatform(String),

    #[error("install failed: {0}")]
    Install(String),

    #[error("all mirrors failed")]
    AllMirrorsFailed,

    #[error("{0}")]
    Other(String),
}

impl serde::Serialize for AppError {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

pub type Result<T> = std::result::Result<T, AppError>;
