//! Virtual camera backends

use crate::error::Result;
use crate::pixel_format::PixelFormat;

#[cfg(windows)]
pub mod windows_obs;
#[cfg(windows)]
pub mod windows_unity;

#[cfg(target_os = "linux")]
pub mod linux_v4l2;

#[cfg(feature = "cuda")]
pub mod cuda_convert;

/// Backend trait that all virtual camera implementations must implement
pub trait Backend: Send {
    /// Get the name of this backend
    fn name(&self) -> &'static str;

    /// Get the device name in use
    fn device(&self) -> &str;

    /// Get the native pixel format used by this backend
    fn native_format(&self) -> PixelFormat;

    /// Send a frame to the virtual camera
    /// Frame must be in the format returned by `native_format()`
    fn send(&mut self, frame: &[u8]) -> Result<()>;

    /// Close the backend and release all resources
    fn close(&mut self) -> Result<()>;

    /// Check if the backend is still open
    fn is_open(&self) -> bool;
}

/// Information about an available backend
#[derive(Debug, Clone)]
pub struct BackendInfo {
    /// Backend name (e.g., "obs", "unitycapture")
    pub name: &'static str,
    /// Human-readable description
    pub description: &'static str,
    /// Default device name
    pub default_device: &'static str,
    /// Supported pixel formats
    pub native_format: PixelFormat,
}

/// Get list of available backends on this platform
pub fn available_backends() -> Vec<BackendInfo> {
    let mut backends = Vec::new();

    #[cfg(windows)]
    {
        // OBS Virtual Camera - primary backend
        #[cfg(feature = "obs")]
        if windows_obs::is_available() {
            backends.push(BackendInfo {
                name: "obs",
                description: "OBS Virtual Camera",
                default_device: "OBS Virtual Camera",
                native_format: PixelFormat::NV12,
            });
        }

        // Unity Capture - alternative backend
        #[cfg(feature = "unity-capture")]
        {
            backends.push(BackendInfo {
                name: "unitycapture",
                description: "Unity Video Capture",
                default_device: "Unity Video Capture",
                native_format: PixelFormat::RGBA,
            });
        }
    }

    #[cfg(target_os = "linux")]
    {
        // V4L2 Loopback - Linux backend
        #[cfg(feature = "v4l2")]
        if linux_v4l2::is_available() {
            backends.push(BackendInfo {
                name: "v4l2loopback",
                description: "V4L2 Loopback",
                default_device: "/dev/video0",
                native_format: PixelFormat::I420,
            });
        }
    }

    backends
}

/// Create a backend by name
pub fn create_backend(
    name: &str,
    width: u32,
    height: u32,
    fps: f64,
    device: Option<&str>,
) -> Result<Box<dyn Backend>> {
    match name {
        #[cfg(all(windows, feature = "obs"))]
        "obs" => {
            let backend = windows_obs::ObsBackend::new(width, height, fps, device)?;
            Ok(Box::new(backend))
        }
        #[cfg(all(windows, feature = "unity-capture"))]
        "unitycapture" => {
            let backend = windows_unity::UnityCaptureBackend::new(width, height, fps, device)?;
            Ok(Box::new(backend))
        }
        #[cfg(all(target_os = "linux", feature = "v4l2"))]
        "v4l2loopback" | "v4l2" => {
            let backend = linux_v4l2::V4l2Backend::new(width, height, fps, device)?;
            Ok(Box::new(backend))
        }
        _ => Err(crate::error::VirtualCamError::BackendNotAvailable(
            name.to_string(),
        )),
    }
}

/// Try to create any available backend
pub fn create_any_backend(
    width: u32,
    height: u32,
    fps: f64,
    device: Option<&str>,
) -> Result<Box<dyn Backend>> {
    let mut errors = Vec::new();

    for backend_info in available_backends() {
        match create_backend(backend_info.name, width, height, fps, device) {
            Ok(backend) => return Ok(backend),
            Err(e) => errors.push((backend_info.name, e)),
        }
    }

    if errors.is_empty() {
        Err(crate::error::VirtualCamError::NoDeviceFound)
    } else {
        let error_msg = errors
            .iter()
            .map(|(name, e)| format!("{}: {}", name, e))
            .collect::<Vec<_>>()
            .join("; ");
        Err(crate::error::VirtualCamError::InitializationFailed(
            error_msg,
        ))
    }
}
