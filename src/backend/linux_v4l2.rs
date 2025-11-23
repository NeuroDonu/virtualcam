//! V4L2 Loopback backend for Linux

use crate::error::{Result, VirtualCamError};
use crate::pixel_format::PixelFormat;
use super::Backend;

use std::collections::HashSet;
use std::ffi::CString;
use std::fs::{File, OpenOptions};
use std::io::Write as IoWrite;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::io::{AsRawFd, RawFd};
use std::sync::Mutex;

use nix::libc;

// Track devices in use globally
lazy_static::lazy_static! {
    static ref ACTIVE_DEVICES: Mutex<HashSet<String>> = Mutex::new(HashSet::new());
}

/// V4L2 ioctl request codes
const VIDIOC_QUERYCAP: libc::c_ulong = 0x80685600;
const VIDIOC_S_FMT: libc::c_ulong = 0xC0D05605;

/// V4L2 buffer type
const V4L2_BUF_TYPE_VIDEO_OUTPUT: u32 = 2;

/// V4L2 capabilities
const V4L2_CAP_VIDEO_OUTPUT: u32 = 0x00000002;

/// V4L2 pixel formats
const V4L2_PIX_FMT_GREY: u32 = fourcc(b"GREY");
const V4L2_PIX_FMT_YUV420: u32 = fourcc(b"YU12");
const V4L2_PIX_FMT_NV12: u32 = fourcc(b"NV12");
const V4L2_PIX_FMT_YUYV: u32 = fourcc(b"YUYV");
const V4L2_PIX_FMT_UYVY: u32 = fourcc(b"UYVY");

/// Expected v4l2loopback driver name
const V4L2_LOOPBACK_DRIVER: &str = "v4l2 loopback";

/// FourCC encoding helper
const fn fourcc(bytes: &[u8; 4]) -> u32 {
    ((bytes[0] as u32) << 0)
        | ((bytes[1] as u32) << 8)
        | ((bytes[2] as u32) << 16)
        | ((bytes[3] as u32) << 24)
}

/// V4L2 capability structure
#[repr(C)]
#[derive(Default)]
struct V4l2Capability {
    driver: [u8; 16],
    card: [u8; 32],
    bus_info: [u8; 32],
    version: u32,
    capabilities: u32,
    device_caps: u32,
    reserved: [u32; 3],
}

/// V4L2 pixel format structure
#[repr(C)]
#[derive(Default, Clone, Copy)]
struct V4l2PixFormat {
    width: u32,
    height: u32,
    pixelformat: u32,
    field: u32,
    bytesperline: u32,
    sizeimage: u32,
    colorspace: u32,
    priv_: u32,
    flags: u32,
    ycbcr_enc: u32,
    quantization: u32,
    xfer_func: u32,
}

/// V4L2 format structure
#[repr(C)]
struct V4l2Format {
    type_: u32,
    fmt: V4l2FormatUnion,
}

/// Union for format (we only use pix)
#[repr(C)]
union V4l2FormatUnion {
    pix: V4l2PixFormat,
    raw_data: [u8; 200],
}

impl Default for V4l2Format {
    fn default() -> Self {
        Self {
            type_: 0,
            fmt: V4l2FormatUnion {
                raw_data: [0; 200],
            },
        }
    }
}

/// V4L2 Loopback backend
pub struct V4l2Backend {
    width: u32,
    height: u32,
    device: String,
    is_open: bool,

    file: Option<File>,
    native_format: PixelFormat,
    frame_size: usize,
}

unsafe impl Send for V4l2Backend {}

impl V4l2Backend {
    /// Create a new V4L2 backend
    pub fn new(
        width: u32,
        height: u32,
        _fps: f64, // V4L2 loopback doesn't use fps - rate determined by send calls
        device: Option<&str>,
    ) -> Result<Self> {
        // Determine native format and V4L2 pixel format
        // Default to I420 which is widely supported
        let native_format = PixelFormat::I420;
        let v4l2_pixfmt = V4L2_PIX_FMT_YUV420;
        let frame_size = native_format.frame_size(width, height);

        let try_open = |device_name: &str| -> Result<File> {
            // Check if already in use
            {
                let active = ACTIVE_DEVICES.lock().unwrap();
                if active.contains(device_name) {
                    return Err(VirtualCamError::DeviceInUse(device_name.to_string()));
                }
            }

            // Open device
            let file = OpenOptions::new()
                .write(true)
                .custom_flags(libc::O_SYNC)
                .open(device_name)
                .map_err(|e| {
                    match e.kind() {
                        std::io::ErrorKind::PermissionDenied => {
                            VirtualCamError::PermissionDenied(format!(
                                "Could not access {} due to missing permissions. \
                                 Did you add your user to the 'video' group? \
                                 Run 'usermod -a -G video {}' and log out and in again.",
                                device_name,
                                std::env::var("USER").unwrap_or_else(|_| "myusername".to_string())
                            ))
                        }
                        std::io::ErrorKind::NotFound => {
                            VirtualCamError::DeviceNotFound(format!(
                                "Device {} does not exist",
                                device_name
                            ))
                        }
                        _ => VirtualCamError::IoError(e),
                    }
                })?;

            let fd = file.as_raw_fd();

            // Query capabilities
            let mut cap = V4l2Capability::default();
            let ret = unsafe { libc::ioctl(fd, VIDIOC_QUERYCAP, &mut cap) };
            if ret == -1 {
                return Err(VirtualCamError::InitializationFailed(format!(
                    "Device capabilities of {} could not be queried",
                    device_name
                )));
            }

            // Check for video output capability
            if (cap.capabilities & V4L2_CAP_VIDEO_OUTPUT) == 0 {
                return Err(VirtualCamError::DeviceNotFound(format!(
                    "Device {} is not a video output device",
                    device_name
                )));
            }

            // Check if it's v4l2loopback
            let driver = String::from_utf8_lossy(&cap.driver)
                .trim_end_matches('\0')
                .to_string();
            if driver != V4L2_LOOPBACK_DRIVER {
                return Err(VirtualCamError::DeviceNotFound(format!(
                    "Device {} is not a v4l2loopback device (driver: {})",
                    device_name, driver
                )));
            }

            Ok(file)
        };

        // Find or open device
        let (device_name, file) = if let Some(dev) = device {
            let file = try_open(dev)?;
            (dev.to_string(), file)
        } else {
            // Search for available device
            let mut found = None;
            for i in 0..100 {
                let device_name = format!("/dev/video{}", i);
                match try_open(&device_name) {
                    Ok(file) => {
                        found = Some((device_name, file));
                        break;
                    }
                    Err(VirtualCamError::DeviceNotFound(_)) => continue,
                    Err(VirtualCamError::DeviceInUse(_)) => continue,
                    Err(e) => return Err(e),
                }
            }

            found.ok_or_else(|| {
                VirtualCamError::NoDeviceFound
            })?
        };

        // Configure format
        let fd = file.as_raw_fd();
        let mut fmt = V4l2Format::default();
        fmt.type_ = V4L2_BUF_TYPE_VIDEO_OUTPUT;

        unsafe {
            fmt.fmt.pix = V4l2PixFormat {
                width,
                height,
                pixelformat: v4l2_pixfmt,
                ..Default::default()
            };
        }

        let ret = unsafe { libc::ioctl(fd, VIDIOC_S_FMT, &mut fmt) };
        if ret == -1 {
            let err = std::io::Error::last_os_error();
            return Err(VirtualCamError::InitializationFailed(format!(
                "Virtual camera device {} could not be configured: {}",
                device_name, err
            )));
        }

        // Track as active
        {
            let mut active = ACTIVE_DEVICES.lock().unwrap();
            active.insert(device_name.clone());
        }

        log::info!(
            "V4L2 Virtual Camera initialized: {}x{} on {}",
            width,
            height,
            device_name
        );

        Ok(Self {
            width,
            height,
            device: device_name,
            is_open: true,
            file: Some(file),
            native_format,
            frame_size,
        })
    }

    /// Create with specific pixel format
    pub fn with_format(
        width: u32,
        height: u32,
        fps: f64,
        device: Option<&str>,
        format: PixelFormat,
    ) -> Result<Self> {
        let v4l2_pixfmt = match format {
            PixelFormat::GRAY => V4L2_PIX_FMT_GREY,
            PixelFormat::I420 => V4L2_PIX_FMT_YUV420,
            PixelFormat::NV12 => V4L2_PIX_FMT_NV12,
            PixelFormat::YUYV => V4L2_PIX_FMT_YUYV,
            PixelFormat::UYVY => V4L2_PIX_FMT_UYVY,
            _ => {
                return Err(VirtualCamError::UnsupportedFormat(format!(
                    "{} is not directly supported by V4L2, use I420, NV12, YUYV, UYVY, or GRAY",
                    format
                )))
            }
        };

        let frame_size = format.frame_size(width, height);

        // Use the same logic but with custom format
        let try_open = |device_name: &str| -> Result<File> {
            {
                let active = ACTIVE_DEVICES.lock().unwrap();
                if active.contains(device_name) {
                    return Err(VirtualCamError::DeviceInUse(device_name.to_string()));
                }
            }

            let file = OpenOptions::new()
                .write(true)
                .custom_flags(libc::O_SYNC)
                .open(device_name)
                .map_err(|e| match e.kind() {
                    std::io::ErrorKind::PermissionDenied => {
                        VirtualCamError::PermissionDenied(format!(
                            "Could not access {} due to missing permissions. \
                             Did you add your user to the 'video' group?",
                            device_name
                        ))
                    }
                    std::io::ErrorKind::NotFound => {
                        VirtualCamError::DeviceNotFound(format!(
                            "Device {} does not exist",
                            device_name
                        ))
                    }
                    _ => VirtualCamError::IoError(e),
                })?;

            let fd = file.as_raw_fd();
            let mut cap = V4l2Capability::default();

            if unsafe { libc::ioctl(fd, VIDIOC_QUERYCAP, &mut cap) } == -1 {
                return Err(VirtualCamError::InitializationFailed(
                    "Could not query device capabilities".to_string(),
                ));
            }

            if (cap.capabilities & V4L2_CAP_VIDEO_OUTPUT) == 0 {
                return Err(VirtualCamError::DeviceNotFound(
                    "Not a video output device".to_string(),
                ));
            }

            let driver = String::from_utf8_lossy(&cap.driver)
                .trim_end_matches('\0')
                .to_string();
            if driver != V4L2_LOOPBACK_DRIVER {
                return Err(VirtualCamError::DeviceNotFound(
                    "Not a v4l2loopback device".to_string(),
                ));
            }

            Ok(file)
        };

        let (device_name, file) = if let Some(dev) = device {
            let file = try_open(dev)?;
            (dev.to_string(), file)
        } else {
            let mut found = None;
            for i in 0..100 {
                let device_name = format!("/dev/video{}", i);
                match try_open(&device_name) {
                    Ok(file) => {
                        found = Some((device_name, file));
                        break;
                    }
                    Err(_) => continue,
                }
            }
            found.ok_or(VirtualCamError::NoDeviceFound)?
        };

        let fd = file.as_raw_fd();
        let mut fmt = V4l2Format::default();
        fmt.type_ = V4L2_BUF_TYPE_VIDEO_OUTPUT;

        unsafe {
            fmt.fmt.pix = V4l2PixFormat {
                width,
                height,
                pixelformat: v4l2_pixfmt,
                ..Default::default()
            };
        }

        if unsafe { libc::ioctl(fd, VIDIOC_S_FMT, &mut fmt) } == -1 {
            let err = std::io::Error::last_os_error();
            return Err(VirtualCamError::InitializationFailed(format!(
                "Could not configure device: {}",
                err
            )));
        }

        {
            let mut active = ACTIVE_DEVICES.lock().unwrap();
            active.insert(device_name.clone());
        }

        log::info!(
            "V4L2 Virtual Camera initialized: {}x{} {} on {}",
            width,
            height,
            format,
            device_name
        );

        Ok(Self {
            width,
            height,
            device: device_name,
            is_open: true,
            file: Some(file),
            native_format: format,
            frame_size,
        })
    }
}

impl Backend for V4l2Backend {
    fn name(&self) -> &'static str {
        "v4l2loopback"
    }

    fn device(&self) -> &str {
        &self.device
    }

    fn native_format(&self) -> PixelFormat {
        self.native_format
    }

    fn send(&mut self, frame: &[u8]) -> Result<()> {
        if !self.is_open {
            return Err(VirtualCamError::AlreadyClosed);
        }

        // Validate frame size
        if frame.len() != self.frame_size {
            return Err(VirtualCamError::FrameSizeMismatch {
                expected: self.frame_size,
                actual: frame.len(),
            });
        }

        // Write frame
        if let Some(ref mut file) = self.file {
            file.write_all(frame).map_err(|e| {
                VirtualCamError::SendFailed(format!("Error writing frame: {}", e))
            })?;
        } else {
            return Err(VirtualCamError::AlreadyClosed);
        }

        Ok(())
    }

    fn close(&mut self) -> Result<()> {
        if !self.is_open {
            return Ok(());
        }

        self.is_open = false;

        // Close file (drop will close fd)
        self.file.take();

        // Remove from active devices
        {
            let mut active = ACTIVE_DEVICES.lock().unwrap();
            active.remove(&self.device);
        }

        log::info!("V4L2 Virtual Camera closed: {}", self.device);
        Ok(())
    }

    fn is_open(&self) -> bool {
        self.is_open
    }
}

impl Drop for V4l2Backend {
    fn drop(&mut self) {
        let _ = self.close();
    }
}

/// Check if v4l2loopback is available
pub fn is_available() -> bool {
    for i in 0..100 {
        let device_name = format!("/dev/video{}", i);

        if let Ok(file) = OpenOptions::new().write(true).open(&device_name) {
            let fd = file.as_raw_fd();
            let mut cap = V4l2Capability::default();

            if unsafe { libc::ioctl(fd, VIDIOC_QUERYCAP, &mut cap) } == 0 {
                let driver = String::from_utf8_lossy(&cap.driver)
                    .trim_end_matches('\0')
                    .to_string();

                if driver == V4L2_LOOPBACK_DRIVER
                    && (cap.capabilities & V4L2_CAP_VIDEO_OUTPUT) != 0
                {
                    return true;
                }
            }
        }
    }
    false
}

/// List all available v4l2loopback devices
pub fn list_devices() -> Vec<String> {
    let mut devices = Vec::new();

    for i in 0..100 {
        let device_name = format!("/dev/video{}", i);

        if let Ok(file) = OpenOptions::new().write(true).open(&device_name) {
            let fd = file.as_raw_fd();
            let mut cap = V4l2Capability::default();

            if unsafe { libc::ioctl(fd, VIDIOC_QUERYCAP, &mut cap) } == 0 {
                let driver = String::from_utf8_lossy(&cap.driver)
                    .trim_end_matches('\0')
                    .to_string();

                if driver == V4L2_LOOPBACK_DRIVER
                    && (cap.capabilities & V4L2_CAP_VIDEO_OUTPUT) != 0
                {
                    devices.push(device_name);
                }
            }
        }
    }

    devices
}

/// Get device info
pub fn device_info(device: &str) -> Result<String> {
    let file = OpenOptions::new()
        .write(true)
        .open(device)
        .map_err(|e| VirtualCamError::IoError(e))?;

    let fd = file.as_raw_fd();
    let mut cap = V4l2Capability::default();

    if unsafe { libc::ioctl(fd, VIDIOC_QUERYCAP, &mut cap) } == -1 {
        return Err(VirtualCamError::InitializationFailed(
            "Could not query device".to_string(),
        ));
    }

    let driver = String::from_utf8_lossy(&cap.driver)
        .trim_end_matches('\0')
        .to_string();
    let card = String::from_utf8_lossy(&cap.card)
        .trim_end_matches('\0')
        .to_string();
    let bus_info = String::from_utf8_lossy(&cap.bus_info)
        .trim_end_matches('\0')
        .to_string();

    Ok(format!(
        "Device: {}\n  Driver: {}\n  Card: {}\n  Bus: {}\n  Version: {}",
        device,
        driver,
        card,
        bus_info,
        cap.version
    ))
}
