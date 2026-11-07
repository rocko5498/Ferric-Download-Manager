use thiserror::Error;

#[derive(Error, Debug, serde::Serialize, serde::Deserialize)]
pub enum FerricError {
    #[error("invalid URL: {0}")]
    InvalidUrl(String),

    #[error("scheme not allowed: {0}")]
    SchemeNotAllowed(String),

    #[error("SSRF: blocked private/loopback address: {0}")]
    SsrfBlocked(String),

    #[error("path traversal attempt: {raw}")]
    PathTraversal { raw: String },

    #[error("invalid save path: {0}")]
    InvalidSavePath(String),

    #[error("extension not authorized: {0}")]
    UnauthorizedExtension(String),

    #[error("daemon not running")]
    DaemonNotRunning,

    #[error("database error: {0}")]
    Database(String),

    #[error("IO error: {0}")]
    Io(String),

    #[error("HTTP error {status}: {message}")]
    Http { status: u16, message: String },

    #[error("chunk {index} failed: {reason}")]
    ChunkFailed { index: usize, reason: String },

    #[error("server rejected Range request (no Accept-Ranges or returned 200 for Range)")]
    RangeRejected,

    #[error("the server sent a web page instead of the file (a verification page, a login page or an expired link); open the link in your browser to download it there")]
    WebPageInsteadOfFile,

    #[error("file changed on server (ETag/Last-Modified mismatch) — cannot resume")]
    FileChanged,

    #[error("rate limited by server (429)")]
    RateLimited,

    #[error("file not found on server (404)")]
    NotFound,

    #[error("access denied by server (403)")]
    Forbidden,

    #[error("signed URL expired — re-capture the link to resume")]
    SignedUrlExpired,

    #[error("download cancelled")]
    Cancelled,

    #[error("IPC error: {0}")]
    Ipc(String),

    #[error("serialization error: {0}")]
    Serialization(String),
}

impl FerricError {
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        match self {
            FerricError::Http { status, .. } => *status >= 500,
            FerricError::Io(_) | FerricError::ChunkFailed { .. } => true,
            _ => false,
        }
    }

    pub fn from_status(status: u16, body: impl Into<String>) -> Self {
        FerricError::Http {
            status,
            message: body.into(),
        }
    }
}

// Conversion from std::io::Error (can't use #[from] with serde, so manual)
impl From<std::io::Error> for FerricError {
    fn from(e: std::io::Error) -> Self {
        FerricError::Io(e.to_string())
    }
}
