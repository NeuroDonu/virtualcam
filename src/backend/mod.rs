//! Platform virtual-camera backends.

use crate::error::{Result, VirtualCamError};
use crate::pixel_format::PixelFormat;

/// A platform backend selected explicitly or through [`BackendKind::Auto`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum BackendKind {
    #[default]
    Auto,
    MediaFoundation,
    Obs,
    V4l2,
}

impl BackendKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::MediaFoundation => "media-foundation",
            Self::Obs => "obs",
            Self::V4l2 => "v4l2",
        }
    }
}

impl std::fmt::Display for BackendKind {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl std::str::FromStr for BackendKind {
    type Err = VirtualCamError;

    fn from_str(value: &str) -> Result<Self> {
        match value.to_ascii_lowercase().as_str() {
            "auto" => Ok(Self::Auto),
            "media-foundation" | "mf" | "vcam" => Ok(Self::MediaFoundation),
            "obs" => Ok(Self::Obs),
            "v4l2" | "v4l2loopback" => Ok(Self::V4l2),
            _ => Err(VirtualCamError::BackendNotAvailable(value.to_owned())),
        }
    }
}

#[cfg(all(target_os = "linux", feature = "v4l2"))]
pub mod linux_v4l2;
#[cfg(all(windows, feature = "media-foundation"))]
pub mod windows_media_foundation;
#[cfg(all(windows, feature = "obs"))]
pub mod windows_obs;

/// Backend implemented by one platform transport.
pub trait Backend: Send {
    fn name(&self) -> &'static str;
    fn device(&self) -> &str;
    fn native_format(&self) -> PixelFormat;
    fn native_dimensions(&self) -> (u32, u32);
    fn send(&mut self, frame: &[u8]) -> Result<()>;
    fn close(&mut self) -> Result<()>;
    fn is_open(&self) -> bool;
}

#[derive(Debug, Clone)]
pub struct BackendInfo {
    pub kind: BackendKind,
    pub name: &'static str,
    pub description: &'static str,
    pub default_device: &'static str,
    pub native_format: PixelFormat,
}

pub fn available_backends() -> Vec<BackendInfo> {
    let mut backends = Vec::new();

    #[cfg(windows)]
    {
        #[cfg(feature = "media-foundation")]
        if windows_media_foundation::is_available() {
            backends.push(BackendInfo {
                kind: BackendKind::MediaFoundation,
                name: "media-foundation",
                description: "VCam Windows Media Foundation camera",
                default_device: "VCam",
                native_format: PixelFormat::NV12,
            });
        }

        #[cfg(feature = "obs")]
        if windows_obs::is_available() {
            backends.push(BackendInfo {
                kind: BackendKind::Obs,
                name: "obs",
                description: "OBS Virtual Camera shared-memory transport",
                default_device: "OBS Virtual Camera",
                native_format: PixelFormat::NV12,
            });
        }
    }

    #[cfg(all(target_os = "linux", feature = "v4l2"))]
    if linux_v4l2::is_available() {
        backends.push(BackendInfo {
            kind: BackendKind::V4l2,
            name: "v4l2",
            description: "v4l2loopback mmap streaming",
            default_device: "/dev/video10",
            native_format: PixelFormat::RGB,
        });
    }

    backends
}

pub fn create_backend(
    kind: BackendKind,
    width: u32,
    height: u32,
    fps: f64,
    device: Option<&str>,
) -> Result<Box<dyn Backend>> {
    match kind {
        BackendKind::Auto => create_any_backend(width, height, fps, device),
        #[cfg(all(windows, feature = "media-foundation"))]
        BackendKind::MediaFoundation => Ok(Box::new(
            windows_media_foundation::MediaFoundationBackend::new(width, height, fps, device)?,
        )),
        #[cfg(all(windows, feature = "obs"))]
        BackendKind::Obs => Ok(Box::new(windows_obs::ObsBackend::new(
            width, height, fps, device,
        )?)),
        #[cfg(all(target_os = "linux", feature = "v4l2"))]
        BackendKind::V4l2 => Ok(Box::new(linux_v4l2::V4l2Backend::new(
            width, height, fps, device,
        )?)),
        _ => Err(VirtualCamError::BackendNotAvailable(kind.to_string())),
    }
}

pub fn create_any_backend(
    width: u32,
    height: u32,
    fps: f64,
    device: Option<&str>,
) -> Result<Box<dyn Backend>> {
    let mut errors = Vec::new();
    for info in available_backends() {
        match create_backend(info.kind, width, height, fps, device) {
            Ok(backend) => return Ok(backend),
            Err(error) => errors.push(format!("{}: {error}", info.name)),
        }
    }

    if errors.is_empty() {
        #[cfg(all(target_os = "linux", feature = "v4l2"))]
        {
            Err(VirtualCamError::V4l2LoopbackNotFound)
        }

        #[cfg(not(all(target_os = "linux", feature = "v4l2")))]
        {
            Err(VirtualCamError::NoDeviceFound)
        }
    } else {
        Err(VirtualCamError::InitializationFailed(errors.join("; ")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_names_roundtrip() {
        for kind in [
            BackendKind::Auto,
            BackendKind::MediaFoundation,
            BackendKind::Obs,
            BackendKind::V4l2,
        ] {
            assert_eq!(kind.as_str().parse::<BackendKind>().unwrap(), kind);
        }
        assert_eq!(
            "mf".parse::<BackendKind>().unwrap(),
            BackendKind::MediaFoundation
        );
        assert_eq!(
            "v4l2loopback".parse::<BackendKind>().unwrap(),
            BackendKind::V4l2
        );
    }
}
