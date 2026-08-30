//! Main Camera struct and public API

use crate::backend::{self, Backend, BackendKind, ShutdownHandle};
use crate::error::{Result, VirtualCamError};
use crate::image_formats::{convert_frame, rgb24_to_nv12_scaled_into};
use crate::pixel_format::PixelFormat;
use crate::util::{FpsCounter, FrameTimer};

#[cfg(feature = "gpu")]
use cudarc::driver::{CudaSlice, CudaStream, PinnedHostSlice};

/// Builder for creating a Camera with custom configuration
pub struct CameraBuilder {
    width: u32,
    height: u32,
    fps: f64,
    format: PixelFormat,
    device: Option<String>,
    backend: BackendKind,
    print_fps: bool,
}

impl CameraBuilder {
    /// Create a new camera builder with required parameters
    pub fn new(width: u32, height: u32, fps: f64) -> Self {
        Self {
            width,
            height,
            fps,
            format: PixelFormat::RGB,
            device: None,
            backend: BackendKind::Auto,
            print_fps: false,
        }
    }

    /// Set the input pixel format (default: RGB)
    pub fn format(mut self, format: PixelFormat) -> Self {
        self.format = format;
        self
    }

    /// Set the virtual camera device name
    pub fn device(mut self, device: impl Into<String>) -> Self {
        self.device = Some(device.into());
        self
    }

    /// Select a backend. `Auto` prefers Media Foundation on Windows 11,
    /// falls back to OBS on Windows 10/11, and uses V4L2 on Linux.
    pub fn backend(mut self, backend: BackendKind) -> Self {
        self.backend = backend;
        self
    }

    /// Enable FPS printing to stdout
    pub fn print_fps(mut self, enabled: bool) -> Self {
        self.print_fps = enabled;
        self
    }

    /// Build and open the camera
    pub fn build(self) -> Result<Camera> {
        Camera::new_with_options(
            self.width,
            self.height,
            self.fps,
            self.format,
            self.device.as_deref(),
            Some(self.backend.as_str()),
            self.print_fps,
        )
    }
}

/// Virtual camera for sending video frames
pub struct Camera {
    width: u32,
    height: u32,
    fps: f64,
    format: PixelFormat,
    backend_name: String,

    backend: Box<dyn Backend>,
    fps_counter: FpsCounter,
    frame_timer: FrameTimer,

    frames_sent: u64,
    print_fps: bool,
    last_fps_print: std::time::Instant,

    // Conversion buffer
    conversion_buffer: Vec<u8>,
    rgb_buffer: Vec<u8>,
    #[cfg(feature = "gpu")]
    pinned_frame: Option<PinnedHostSlice<u8>>,
}

impl Camera {
    /// Create a new camera with default settings
    ///
    /// # Arguments
    /// * `width` - Frame width in pixels
    /// * `height` - Frame height in pixels
    /// * `fps` - Target frames per second
    ///
    /// # Example
    /// ```no_run
    /// use virtualcam::Camera;
    ///
    /// let mut cam = Camera::new(1280, 720, 30.0)?;
    /// # Ok::<(), virtualcam::error::VirtualCamError>(())
    /// ```
    pub fn new(width: u32, height: u32, fps: f64) -> Result<Self> {
        Self::new_with_options(width, height, fps, PixelFormat::RGB, None, None, false)
    }

    /// Create a camera with full options
    pub fn new_with_options(
        width: u32,
        height: u32,
        fps: f64,
        format: PixelFormat,
        device: Option<&str>,
        backend_name: Option<&str>,
        print_fps: bool,
    ) -> Result<Self> {
        // Validate parameters
        if width == 0 || height == 0 {
            return Err(VirtualCamError::InvalidDimensions(width, height));
        }
        if (format.requires_even_width() && width % 2 != 0)
            || (format.requires_even_height() && height % 2 != 0)
        {
            return Err(VirtualCamError::InvalidDimensions(width, height));
        }

        if fps <= 0.0 || fps > 1000.0 {
            return Err(VirtualCamError::InvalidFps(fps));
        }

        // Create backend
        let kind = backend_name.unwrap_or("auto").parse::<BackendKind>()?;
        let backend = backend::create_backend(kind, width, height, fps, device)?;

        let backend_name_str = backend.name().to_string();

        log::info!(
            "Camera created: {}x{} @ {} fps, format: {}, backend: {}",
            width,
            height,
            fps,
            format,
            backend_name_str
        );

        Ok(Self {
            width,
            height,
            fps,
            format,
            backend_name: backend_name_str,
            backend,
            fps_counter: FpsCounter::new(fps),
            frame_timer: FrameTimer::new(fps),
            frames_sent: 0,
            print_fps,
            last_fps_print: std::time::Instant::now(),
            conversion_buffer: Vec::new(),
            rgb_buffer: Vec::new(),
            #[cfg(feature = "gpu")]
            pinned_frame: None,
        })
    }

    /// Create a camera using the builder pattern
    pub fn builder(width: u32, height: u32, fps: f64) -> CameraBuilder {
        CameraBuilder::new(width, height, fps)
    }

    /// Send a frame to the virtual camera
    ///
    /// # Arguments
    /// * `frame` - Frame data as a byte slice. Must match the configured format and dimensions.
    ///
    /// # Example
    /// ```no_run
    /// use virtualcam::Camera;
    ///
    /// let mut cam = Camera::new(640, 480, 30.0)?;
    /// let frame = vec![0u8; 640 * 480 * 3]; // RGB frame
    /// cam.send(&frame)?;
    /// # Ok::<(), virtualcam::error::VirtualCamError>(())
    /// ```
    pub fn send(&mut self, frame: &[u8]) -> Result<()> {
        // Validate frame size
        let expected_size = self.format.frame_size(self.width, self.height);
        if frame.len() != expected_size {
            return Err(VirtualCamError::FrameSizeMismatch {
                expected: expected_size,
                actual: frame.len(),
            });
        }

        // Convert and, for fixed-format backends, scale to native dimensions.
        let native_format = self.backend.native_format();
        let (native_width, native_height) = self.backend.native_dimensions();
        let same_dimensions = (self.width, self.height) == (native_width, native_height);
        let frame_to_send = if self.format == native_format && same_dimensions {
            frame
        } else if native_format == PixelFormat::NV12 && !same_dimensions {
            let rgb = if self.format == PixelFormat::RGB {
                frame
            } else {
                self.rgb_buffer = convert_frame(
                    frame,
                    self.format,
                    PixelFormat::RGB,
                    self.width,
                    self.height,
                );
                if self.rgb_buffer.is_empty() {
                    return Err(VirtualCamError::UnsupportedFormat(format!(
                        "{} -> RGB",
                        self.format
                    )));
                }
                &self.rgb_buffer
            };
            rgb24_to_nv12_scaled_into(
                rgb,
                self.width,
                self.height,
                &mut self.conversion_buffer,
                native_width,
                native_height,
            );
            &self.conversion_buffer
        } else if same_dimensions {
            self.conversion_buffer =
                convert_frame(frame, self.format, native_format, self.width, self.height);
            if self.conversion_buffer.is_empty() {
                return Err(VirtualCamError::UnsupportedFormat(format!(
                    "{} -> {}",
                    self.format, native_format
                )));
            }
            &self.conversion_buffer
        } else {
            return Err(VirtualCamError::UnsupportedFormat(format!(
                "scaling {} -> {} is not implemented",
                self.format, native_format
            )));
        };

        self.backend.send(frame_to_send)?;
        self.record_frame();
        Ok(())
    }

    /// Send a frame already encoded in the backend's native format and size.
    pub fn send_native(&mut self, frame: &[u8]) -> Result<()> {
        let (width, height) = self.backend.native_dimensions();
        let expected = self.backend.native_format().frame_size(width, height);
        if frame.len() != expected {
            return Err(VirtualCamError::FrameSizeMismatch {
                expected,
                actual: frame.len(),
            });
        }
        self.backend.send(frame)?;
        self.record_frame();
        Ok(())
    }

    /// Download a device-resident native frame through one reusable pinned
    /// allocation, then publish it through the selected Windows backend.
    #[cfg(feature = "gpu")]
    pub fn send_native_gpu(
        &mut self,
        stream: &std::sync::Arc<CudaStream>,
        frame: &CudaSlice<u8>,
    ) -> Result<()> {
        let (width, height) = self.backend.native_dimensions();
        let expected = self.backend.native_format().frame_size(width, height);
        if frame.len() != expected {
            return Err(VirtualCamError::FrameSizeMismatch {
                expected,
                actual: frame.len(),
            });
        }
        if self.pinned_frame.is_none() {
            self.pinned_frame = Some(
                unsafe { stream.context().alloc_pinned::<u8>(expected) }
                    .map_err(|error| VirtualCamError::SendFailed(error.to_string()))?,
            );
        }
        let pinned = self
            .pinned_frame
            .as_mut()
            .expect("pinned frame initialized");
        stream
            .memcpy_dtoh(
                frame,
                pinned
                    .as_mut_slice()
                    .map_err(|error| VirtualCamError::SendFailed(error.to_string()))?,
            )
            .map_err(|error| VirtualCamError::SendFailed(error.to_string()))?;
        stream
            .synchronize()
            .map_err(|error| VirtualCamError::SendFailed(error.to_string()))?;
        let host = pinned
            .as_slice()
            .map_err(|error| VirtualCamError::SendFailed(error.to_string()))?;
        self.backend.send(host)?;
        self.record_frame();
        Ok(())
    }

    fn record_frame(&mut self) {
        // Update counters
        self.frames_sent += 1;
        self.fps_counter.tick();
        self.frame_timer.begin_frame();

        // Print FPS if enabled
        if self.print_fps && self.last_fps_print.elapsed().as_secs() >= 1 {
            println!("FPS: {:.1}", self.fps_counter.fps());
            self.last_fps_print = std::time::Instant::now();
        }
    }

    /// Sleep until the next frame is due
    ///
    /// Call this after `send()` to maintain the target frame rate.
    ///
    /// # Example
    /// ```no_run
    /// use virtualcam::Camera;
    ///
    /// let mut cam = Camera::new(640, 480, 30.0)?;
    /// loop {
    ///     let frame = vec![0u8; 640 * 480 * 3];
    ///     cam.send(&frame)?;
    ///     cam.sleep_until_next_frame();
    /// }
    /// # Ok::<(), virtualcam::error::VirtualCamError>(())
    /// ```
    pub fn sleep_until_next_frame(&mut self) {
        self.frame_timer.sleep_until_next_frame();
    }

    /// Close the camera and release resources
    pub fn close(&mut self) -> Result<()> {
        self.backend.close()
    }

    /// Return a cloneable signal when the selected backend needs an external
    /// wake-up before its output worker can be joined.
    pub fn shutdown_handle(&self) -> Option<ShutdownHandle> {
        self.backend.shutdown_handle()
    }

    /// Get the frame width
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Get the frame height
    pub fn height(&self) -> u32 {
        self.height
    }

    /// Get the target FPS
    pub fn fps(&self) -> f64 {
        self.fps
    }

    /// Get the input pixel format
    pub fn format(&self) -> PixelFormat {
        self.format
    }

    /// Get the native format used by the backend
    pub fn native_format(&self) -> PixelFormat {
        self.backend.native_format()
    }

    pub fn native_dimensions(&self) -> (u32, u32) {
        self.backend.native_dimensions()
    }

    /// Get the backend name
    pub fn backend(&self) -> &str {
        &self.backend_name
    }

    /// Get the device name
    pub fn device(&self) -> &str {
        self.backend.device()
    }

    /// Get the number of frames sent
    pub fn frames_sent(&self) -> u64 {
        self.frames_sent
    }

    /// Get the current measured FPS
    pub fn current_fps(&self) -> f64 {
        self.fps_counter.fps()
    }

    /// Check if the camera is open
    pub fn is_open(&self) -> bool {
        self.backend.is_open()
    }
}

impl Drop for Camera {
    fn drop(&mut self) {
        let _ = self.close();
    }
}

/// List available backends
pub fn available_backends() -> Vec<backend::BackendInfo> {
    backend::available_backends()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_camera_builder() {
        let builder = Camera::builder(1280, 720, 30.0)
            .format(PixelFormat::BGR)
            .print_fps(true);

        assert_eq!(builder.width, 1280);
        assert_eq!(builder.height, 720);
        assert_eq!(builder.fps, 30.0);
    }

    #[test]
    fn test_invalid_dimensions() {
        let result = Camera::new(0, 480, 30.0);
        assert!(result.is_err());
    }

    #[test]
    fn test_invalid_fps() {
        let result = Camera::new(640, 480, 0.0);
        assert!(result.is_err());

        let result = Camera::new(640, 480, -10.0);
        assert!(result.is_err());
    }
}
