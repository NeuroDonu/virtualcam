//! Unity Video Capture backend for Windows

use crate::error::{Result, VirtualCamError};
use crate::pixel_format::PixelFormat;
use std::ptr;
use std::sync::Mutex;
use super::Backend;
use winreg::enums::*;
use winreg::RegKey;

use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::Memory::{
    OpenFileMappingW, MapViewOfFile, UnmapViewOfFile, FILE_MAP_ALL_ACCESS,
};
use windows::core::PCWSTR;

// Unity Capture registry base
const UNITY_CAPTURE_CLSID_BASE: &str = "CLSID\\{5C2CD55C-92AD-4999-8666-912BD3E700";

// Shared memory format constants
const UNITY_SHM_PREFIX: &str = "UnityCapture_";

// Global tracker for devices in use
static DEVICES_IN_USE: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// List available Unity Capture devices
pub fn list_devices() -> Vec<String> {
    let mut devices = Vec::new();
    let hkcr = RegKey::predef(HKEY_CLASSES_ROOT);

    // Check device indices 0-9
    for i in 0..10 {
        let suffix = if cfg!(target_pointer_width = "64") {
            format!("{:02X}", 0x10 + i)
        } else {
            format!("{:02X}", 0x20 + i)
        };

        let path = format!("{}{}}}", UNITY_CAPTURE_CLSID_BASE, suffix);

        if let Ok(key) = hkcr.open_subkey(&path) {
            // Try to get the device name from FriendlyName
            let name: String = key
                .get_value("")
                .unwrap_or_else(|_| format!("Unity Video Capture #{}", i));
            devices.push(name);
        }
    }

    devices
}

/// Check if Unity Capture is available
pub fn is_available() -> bool {
    !list_devices().is_empty()
}

/// Unity Video Capture backend
pub struct UnityCaptureBackend {
    width: u32,
    height: u32,
    _fps: f64,
    device: String,
    _device_index: usize,
    is_open: bool,

    // Shared memory handles
    file_mapping: HANDLE,
    shm_data: *mut u8,
    _shm_size: usize,

    // Frame buffer
    _frame_buffer: Vec<u8>,
}

// Shared memory header (must match Unity Capture plugin)
#[repr(C)]
struct UnitySharedHeader {
    // Format and size info
    width: u32,
    height: u32,
    stride: u32,
    data_size: u32,
    format: u32,
    resize_mode: u32,
    mirror_mode: u32,
    timeout: u32,

    // Frame counter
    frame_counter: u64,
}

unsafe impl Send for UnityCaptureBackend {}

impl UnityCaptureBackend {
    /// Create a new Unity Capture backend
    pub fn new(width: u32, height: u32, fps: f64, device: Option<&str>) -> Result<Self> {
        let devices = list_devices();

        if devices.is_empty() {
            return Err(VirtualCamError::DeviceNotFound(
                "Unity Video Capture is not installed".to_string(),
            ));
        }

        // Find device index
        let (device_name, device_index) = if let Some(name) = device {
            match devices.iter().position(|d| d == name) {
                Some(idx) => (name.to_string(), idx),
                None => return Err(VirtualCamError::DeviceNotFound(name.to_string())),
            }
        } else {
            // Use first available device
            let mut in_use = DEVICES_IN_USE.lock().unwrap();
            let available = devices
                .iter()
                .enumerate()
                .find(|(_, d)| !in_use.contains(d));

            match available {
                Some((idx, name)) => {
                    in_use.push(name.clone());
                    (name.clone(), idx)
                }
                None => return Err(VirtualCamError::NoDeviceFound),
            }
        };

        // Check if device is already in use
        {
            let mut in_use = DEVICES_IN_USE.lock().unwrap();
            if in_use.contains(&device_name) && device.is_some() {
                return Err(VirtualCamError::DeviceInUse(device_name));
            }
            if !in_use.contains(&device_name) {
                in_use.push(device_name.clone());
            }
        }

        // Calculate frame size (RGBA)
        let rgba_size = (width * height * 4) as usize;
        let header_size = std::mem::size_of::<UnitySharedHeader>();
        let shm_size = header_size + rgba_size;

        // Create shared memory name
        let shm_name = format!("{}{}", UNITY_SHM_PREFIX, device_index);
        let shm_name_wide: Vec<u16> = shm_name.encode_utf16().chain(std::iter::once(0)).collect();

        // Open or create shared memory
        let file_mapping = unsafe {
            OpenFileMappingW(
                FILE_MAP_ALL_ACCESS.0,
                false,
                PCWSTR(shm_name_wide.as_ptr()),
            )
        };

        let file_mapping = match file_mapping {
            Ok(h) => h,
            Err(_) => {
                // Create new mapping
                unsafe {
                    windows::Win32::System::Memory::CreateFileMappingW(
                        HANDLE::default(),
                        None,
                        windows::Win32::System::Memory::PAGE_READWRITE,
                        0,
                        shm_size as u32,
                        PCWSTR(shm_name_wide.as_ptr()),
                    )?
                }
            }
        };

        if file_mapping.is_invalid() {
            Self::release_device(&device_name);
            return Err(VirtualCamError::SharedMemoryError(
                "Failed to create/open file mapping".to_string(),
            ));
        }

        let shm_data = unsafe { MapViewOfFile(file_mapping, FILE_MAP_ALL_ACCESS, 0, 0, shm_size) };

        if shm_data.Value.is_null() {
            unsafe { CloseHandle(file_mapping).ok() };
            Self::release_device(&device_name);
            return Err(VirtualCamError::SharedMemoryError(
                "Failed to map view of file".to_string(),
            ));
        }

        // Initialize header
        let header_ptr = shm_data.Value as *mut UnitySharedHeader;
        unsafe {
            (*header_ptr).width = width;
            (*header_ptr).height = height;
            (*header_ptr).stride = width * 4;
            (*header_ptr).data_size = rgba_size as u32;
            (*header_ptr).format = 0; // RGBA
            (*header_ptr).resize_mode = 1; // Linear
            (*header_ptr).mirror_mode = 0; // Disabled
            (*header_ptr).timeout = 1000;
            (*header_ptr).frame_counter = 0;
        }

        log::info!(
            "Unity Video Capture initialized: {} ({}x{} @ {} fps)",
            device_name, width, height, fps
        );

        Ok(Self {
            width,
            height,
            _fps: fps,
            device: device_name,
            _device_index: device_index,
            is_open: true,
            file_mapping,
            shm_data: shm_data.Value as *mut u8,
            _shm_size: shm_size,
            _frame_buffer: vec![0u8; rgba_size],
        })
    }

    fn release_device(device_name: &str) {
        let mut in_use = DEVICES_IN_USE.lock().unwrap();
        in_use.retain(|d| d != device_name);
    }

    fn write_frame(&mut self, frame: &[u8]) -> Result<()> {
        if !self.is_open {
            return Err(VirtualCamError::AlreadyClosed);
        }

        let header_size = std::mem::size_of::<UnitySharedHeader>();
        let header_ptr = self.shm_data as *mut UnitySharedHeader;

        // Copy frame data (after header)
        unsafe {
            let dst = self.shm_data.add(header_size);
            ptr::copy_nonoverlapping(frame.as_ptr(), dst, frame.len());

            // Increment frame counter
            (*header_ptr).frame_counter += 1;
        }

        Ok(())
    }
}

impl super::Backend for UnityCaptureBackend {
    fn name(&self) -> &'static str {
        "unitycapture"
    }

    fn device(&self) -> &str {
        &self.device
    }

    fn native_format(&self) -> PixelFormat {
        PixelFormat::RGBA
    }

    fn send(&mut self, frame: &[u8]) -> Result<()> {
        let expected_size = PixelFormat::RGBA.frame_size(self.width, self.height);
        if frame.len() != expected_size {
            return Err(VirtualCamError::FrameSizeMismatch {
                expected: expected_size,
                actual: frame.len(),
            });
        }

        self.write_frame(frame)
    }

    fn close(&mut self) -> Result<()> {
        if !self.is_open {
            return Ok(());
        }

        self.is_open = false;

        // Clean up handles
        unsafe {
            if !self.shm_data.is_null() {
                UnmapViewOfFile(windows::Win32::System::Memory::MEMORY_MAPPED_VIEW_ADDRESS {
                    Value: self.shm_data as *mut _,
                })
                .ok();
                self.shm_data = ptr::null_mut();
            }

            if !self.file_mapping.is_invalid() {
                CloseHandle(self.file_mapping).ok();
                self.file_mapping = HANDLE::default();
            }
        }

        // Release device from in-use list
        Self::release_device(&self.device);

        log::info!("Unity Video Capture closed: {}", self.device);
        Ok(())
    }

    fn is_open(&self) -> bool {
        self.is_open
    }
}

impl Drop for UnityCaptureBackend {
    fn drop(&mut self) {
        let _ = self.close();
    }
}
