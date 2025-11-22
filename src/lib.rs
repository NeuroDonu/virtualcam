//! # VirtualCam
//!
//! A Rust library for sending video frames to virtual cameras.
//!
//! Supports OBS Virtual Camera and Unity Video Capture on Windows.
//!
//! ## Quick Start
//!
//! ```no_run
//! use virtualcam::{Camera, PixelFormat};
//!
//! fn main() -> virtualcam::error::Result<()> {
//!     // Create a camera with RGB input
//!     let mut cam = Camera::builder(1280, 720, 30.0)
//!         .format(PixelFormat::RGB)
//!         .build()?;
//!
//!     println!("Using device: {}", cam.device());
//!
//!     // Send frames
//!     let mut frame = vec![0u8; 1280 * 720 * 3];
//!
//!     for i in 0..300 {
//!         // Generate gradient
//!         let hue = (i % 100) as f32 / 100.0;
//!         let (r, g, b) = hsv_to_rgb(hue, 1.0, 1.0);
//!
//!         for pixel in frame.chunks_exact_mut(3) {
//!             pixel[0] = r;
//!             pixel[1] = g;
//!             pixel[2] = b;
//!         }
//!
//!         cam.send(&frame)?;
//!         cam.sleep_until_next_frame();
//!     }
//!
//!     Ok(())
//! }
//!
//! fn hsv_to_rgb(h: f32, s: f32, v: f32) -> (u8, u8, u8) {
//!     let i = (h * 6.0) as i32;
//!     let f = h * 6.0 - i as f32;
//!     let p = v * (1.0 - s);
//!     let q = v * (1.0 - f * s);
//!     let t = v * (1.0 - (1.0 - f) * s);
//!
//!     let (r, g, b) = match i % 6 {
//!         0 => (v, t, p),
//!         1 => (q, v, p),
//!         2 => (p, v, t),
//!         3 => (p, q, v),
//!         4 => (t, p, v),
//!         _ => (v, p, q),
//!     };
//!
//!     ((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8)
//! }
//! ```
//!
//! ## Pixel Formats
//!
//! The library supports multiple input pixel formats:
//!
//! - `RGB` - 3 bytes per pixel (Red, Green, Blue)
//! - `BGR` - 3 bytes per pixel (Blue, Green, Red) - OpenCV default
//! - `RGBA` - 4 bytes per pixel with alpha
//! - `GRAY` - 1 byte per pixel grayscale
//! - `I420` - YUV 4:2:0 planar
//! - `NV12` - YUV 4:2:0 semi-planar
//! - `YUYV` - YUV 4:2:2 packed
//! - `UYVY` - YUV 4:2:2 packed
//!
//! ## Backends
//!
//! Currently supported backends:
//!
//! - **OBS Virtual Camera** (Windows) - Native format: NV12
//! - **Unity Video Capture** (Windows) - Native format: RGBA
//!
//! The library automatically converts your input format to the backend's native format.

pub mod backend;
pub mod camera;
pub mod error;
pub mod image_formats;
pub mod pixel_format;
pub mod util;

// Re-exports for convenient access
pub use camera::{available_backends, Camera, CameraBuilder};
pub use error::{Result, VirtualCamError};
pub use pixel_format::PixelFormat;

/// Prelude module for convenient imports
pub mod prelude {
    pub use crate::camera::{available_backends, Camera, CameraBuilder};
    pub use crate::error::{Result, VirtualCamError};
    pub use crate::pixel_format::PixelFormat;
}
