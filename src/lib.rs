//! Cross-platform virtual-camera output.
//!
//! `Camera` provides one API over three independent transports:
//!
//! - Windows 11 Media Foundation through the bundled VCam runtime;
//! - OBS Virtual Camera on Windows 10 and Windows 11;
//! - mmap streaming to `v4l2loopback` on Linux.
//!
//! ```no_run
//! use virtualcam::{Camera, PixelFormat};
//!
//! let mut camera = Camera::builder(1280, 720, 30.0)
//!     .format(PixelFormat::RGB)
//!     .build()?;
//! camera.send(&vec![0; 1280 * 720 * 3])?;
//! # Ok::<(), virtualcam::VirtualCamError>(())
//! ```

pub mod backend;
pub mod camera;
pub mod error;
pub mod image_formats;
pub mod pixel_format;
pub mod util;

pub use backend::{Backend, BackendInfo, BackendKind, ShutdownHandle};
pub use camera::{Camera, CameraBuilder, available_backends};
pub use error::{Result, VirtualCamError};
pub use pixel_format::PixelFormat;

pub mod prelude {
    pub use crate::backend::{Backend, BackendInfo, BackendKind, ShutdownHandle};
    pub use crate::camera::{Camera, CameraBuilder, available_backends};
    pub use crate::error::{Result, VirtualCamError};
    pub use crate::pixel_format::PixelFormat;
}
