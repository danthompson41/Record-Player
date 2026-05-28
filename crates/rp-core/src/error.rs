use thiserror::Error;

#[derive(Error, Debug)]
pub enum RecordPlayerError {
    #[error("Audio device error: {0}")]
    AudioDevice(String),

    #[error("File not found: {0}")]
    FileNotFound(String),

    #[error("Unsupported format: {0}")]
    UnsupportedFormat(String),

    #[error("Decoding error: {0}")]
    Decoding(String),

    #[error("Plugin error: {0}")]
    Plugin(String),

    #[error("Database error: {0}")]
    Database(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, RecordPlayerError>;
