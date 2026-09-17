use thiserror::Error;

/// Top-level error type surfaced to the UI layer. Every subsystem maps its
/// own error enum into one of these variants so the frontend has a single,
/// stable shape to render — never a raw Win32 HRESULT or FFmpeg errno.
#[derive(Debug, Error, Clone, serde::Serialize)]
pub enum BrailError {
    #[error("no capturable source found (monitor/window may have disconnected)")]
    CaptureSourceLost,

    #[error("screen capture failed to initialize: {0}")]
    CaptureInitFailed(String),

    #[error("no compatible hardware encoder is available; falling back to software encoding")]
    NoHardwareEncoder,

    #[error("encoder initialization failed: {0}")]
    EncoderInitFailed(String),

    #[error("encoder rejected the requested configuration: {0}")]
    EncoderConfigRejected(String),

    #[error("audio device error: {0}")]
    AudioDeviceError(String),

    #[error("could not open output file at {0}: {1}")]
    OutputFileError(String, String),

    #[error("recording could not be finalized cleanly; a recovery file was written: {0}")]
    FinalizationFailed(String),

    #[error("streaming connection failed: {0}")]
    StreamConnectFailed(String),

    #[error("streaming connection dropped: {0}")]
    StreamDisconnected(String),

    #[error("invalid stream key or server configuration")]
    InvalidStreamCredentials,

    #[error("insufficient upload bandwidth for the selected quality preset")]
    InsufficientBandwidth,

    #[error("disk is full or write failed: {0}")]
    DiskWriteFailed(String),

    #[error("requested configuration is not supported on this hardware: {0}")]
    UnsupportedConfiguration(String),

    #[error("secure storage error: {0}")]
    SecureStorageError(String),

    #[error("hotkey registration failed (may be in use by another application): {0}")]
    HotkeyRegistrationFailed(String),

    #[error("internal error: {0}")]
    Internal(String),
}

pub type BrailResult<T> = Result<T, BrailError>;
