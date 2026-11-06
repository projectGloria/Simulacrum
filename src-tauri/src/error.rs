use serde::{Serialize, Serializer};

/// Every failure the frontend can see. Messages are user-facing, so they say what to do,
/// not which Rust function failed.
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("Could not open \"{path}\": {source}")]
    Open { path: String, source: std::io::Error },

    #[error("\"{path}\" is not a supported image (PNG, JPG, WEBP or BMP).")]
    UnsupportedFormat { path: String },

    #[error("\"{path}\" could not be decoded — the file may be corrupt. ({detail})")]
    Decode { path: String, detail: String },

    #[error("Image is too small ({w}×{h}). The square crop must be at least {min}×{min} px.")]
    TooSmall { w: u32, h: u32, min: u32 },

    #[error("Could not write \"{path}\": {source}")]
    Write { path: String, source: std::io::Error },

    #[error("Image encoding failed: {0}")]
    Encode(String),

    #[error("Invalid settings: {0}")]
    InvalidSettings(String),

    #[error("That image is no longer loaded. Re-add it and try again.")]
    UnknownItem,

    #[error("Internal error: {0}")]
    Internal(String),
}

impl AppError {
    /// Turns OS error codes into something a non-developer can act on.
    pub fn write(path: &std::path::Path, source: std::io::Error) -> Self {
        AppError::Write { path: path.display().to_string(), source }
    }
}

// Tauri serializes command errors to the JS promise rejection. A plain string keeps the
// frontend trivial: `catch (e) { show(String(e)) }`.
impl Serialize for AppError {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

pub type AppResult<T> = Result<T, AppError>;
