//! Error types for virtualcam

use thiserror::Error;

/// Result type for virtualcam operations
pub type Result<T> = std::result::Result<T, VirtualCamError>;

/// Errors that can occur when working with virtual cameras
#[derive(Error)]
pub enum VirtualCamError {
    /// No virtual camera device was found
    #[error("No virtual camera backend is available (VCam, OBS, or v4l2loopback)")]
    NoDeviceFound,

    /// Linux has no configured v4l2loopback output device
    #[error(
        "No v4l2loopback output device found. Create one before starting the application, for example: sudo modprobe v4l2loopback video_nr=10 card_label=\"Virtual Camera\" exclusive_caps=1. Run the application itself as your normal user."
    )]
    V4l2LoopbackNotFound,

    /// The specified device was not found
    #[error("Device '{0}' not found")]
    DeviceNotFound(String),

    /// The device is already in use
    #[error("Device '{0}' is already in use")]
    DeviceInUse(String),

    /// The specified backend is not available
    #[error("Backend '{0}' is not available on this platform")]
    BackendNotAvailable(String),

    /// Unsupported pixel format
    #[error("Unsupported pixel format: {0}")]
    UnsupportedFormat(String),

    /// Invalid frame dimensions
    #[error("Invalid frame dimensions: width={0}, height={1}")]
    InvalidDimensions(u32, u32),

    /// Frame size mismatch
    #[error("Frame size mismatch: expected {expected} bytes, got {actual} bytes")]
    FrameSizeMismatch { expected: usize, actual: usize },

    /// Frame shape mismatch
    #[error("Frame shape mismatch: expected {expected:?}, got {actual:?}")]
    FrameShapeMismatch {
        expected: Vec<usize>,
        actual: Vec<usize>,
    },

    /// Invalid FPS value
    #[error("Invalid FPS value: {0}")]
    InvalidFps(f64),

    /// Camera is already closed
    #[error("Camera is already closed")]
    AlreadyClosed,

    /// Failed to initialize camera output
    #[error("Failed to initialize camera output: {0}")]
    InitializationFailed(String),

    /// Failed to send frame
    #[error("Failed to send frame: {0}")]
    SendFailed(String),

    /// Windows-specific error
    #[cfg(windows)]
    #[error("Windows error: {0}")]
    WindowsError(#[from] windows::core::Error),

    /// Registry error
    #[cfg(windows)]
    #[error("Registry error: {0}")]
    RegistryError(String),

    /// Shared memory error
    #[error("Shared memory error: {0}")]
    SharedMemoryError(String),

    /// I/O error
    #[error("I/O error: {0}")]
    IoError(#[from] std::io::Error),

    /// Permission denied
    #[error("Permission denied: {0}")]
    PermissionDenied(String),

    /// Timeout error
    #[error("Operation timed out")]
    Timeout,

    /// Internal error
    #[error("Internal error: {0}")]
    Internal(String),
}

impl std::fmt::Debug for VirtualCamError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(self, formatter)
    }
}

#[cfg(test)]
mod tests {
    use super::VirtualCamError;

    #[test]
    fn missing_loopback_error_is_actionable() {
        let message = VirtualCamError::V4l2LoopbackNotFound.to_string();
        assert!(message.contains("sudo modprobe v4l2loopback"));
        assert!(message.contains("normal user"));
    }
}
