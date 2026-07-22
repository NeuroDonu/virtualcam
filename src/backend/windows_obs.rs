//! OBS Virtual Camera backend for Windows

use super::Backend;
use crate::error::{Result, VirtualCamError};
use crate::pixel_format::PixelFormat;
use std::ptr;
use std::sync::atomic::{AtomicU32, Ordering};
use winreg::RegKey;
use winreg::enums::*;

use windows::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
use windows::Win32::System::Memory::{
    CreateFileMappingW, FILE_MAP_ALL_ACCESS, FILE_MAP_READ, MapViewOfFile, OpenFileMappingW,
    PAGE_READWRITE, UnmapViewOfFile,
};
use windows::core::PCWSTR;

const OBS_VIRTUAL_CAM_GUID: &str = "{A3FCE0F5-3493-419F-958A-ABA1250EC20B}";
const DEVICE_NAME: &str = "OBS Virtual Camera";
const VIDEO_QUEUE_NAME: &str = "OBSVirtualCamVideo";

// Queue states
const SHARED_QUEUE_STATE_STARTING: u32 = 1;
const SHARED_QUEUE_STATE_READY: u32 = 2;

// Frame header size (timestamp + reserved)
const FRAME_HEADER_SIZE: usize = 32;

/// Align size to 32-byte boundary
fn align_size(size: usize) -> usize {
    (size + 31) & !31
}

/// Check if OBS Virtual Camera is installed
pub fn is_available() -> bool {
    let hkcr = RegKey::predef(HKEY_CLASSES_ROOT);
    let path = format!("CLSID\\{}", OBS_VIRTUAL_CAM_GUID);
    hkcr.open_subkey(&path).is_ok()
}

/// OBS Virtual Camera backend
pub struct ObsBackend {
    width: u32,
    height: u32,
    device: String,
    is_open: bool,

    // Shared memory
    file_mapping: HANDLE,
    header: *mut QueueHeader,

    // Frame buffer pointers
    frame_ts: [*mut u64; 3],
    frame_data: [*mut u8; 3],
}

// Queue header structure (must match OBS exactly)
#[repr(C)]
struct QueueHeader {
    write_idx: AtomicU32,
    read_idx: AtomicU32,
    state: AtomicU32,

    offsets: [u32; 3], // Byte offsets for 3 frame buffers
    queue_type: u32,   // Always 0 for video

    cx: u32,       // Frame width
    cy: u32,       // Frame height
    interval: u64, // Frame interval in 100-ns units

    reserved: [u32; 8], // Reserved space
}

const _: () = assert!(std::mem::size_of::<AtomicU32>() == 4);
const _: () = assert!(std::mem::size_of::<QueueHeader>() == 80);

unsafe impl Send for ObsBackend {}

impl ObsBackend {
    /// Create a new OBS backend
    pub fn new(width: u32, height: u32, fps: f64, device: Option<&str>) -> Result<Self> {
        // Check if OBS is installed
        if !is_available() {
            return Err(VirtualCamError::DeviceNotFound(
                "OBS Virtual Camera is not installed".to_string(),
            ));
        }

        // Only one device name is supported
        let device_name = device.unwrap_or(DEVICE_NAME);
        if device_name != DEVICE_NAME && device.is_some() {
            return Err(VirtualCamError::DeviceNotFound(device_name.to_string()));
        }
        if width % 2 != 0 || height % 2 != 0 {
            return Err(VirtualCamError::InvalidDimensions(width, height));
        }

        Self::open_queue(width, height, fps, device_name, VIDEO_QUEUE_NAME)
    }

    fn open_queue(
        width: u32,
        height: u32,
        fps: f64,
        device_name: &str,
        mapping_name: &str,
    ) -> Result<Self> {
        // Check if already in use (try to open existing)
        let queue_name: Vec<u16> = mapping_name
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();

        unsafe {
            let existing = OpenFileMappingW(FILE_MAP_READ.0, false, PCWSTR(queue_name.as_ptr()));

            if let Ok(handle) = existing {
                CloseHandle(handle).ok();
                return Err(VirtualCamError::DeviceInUse(device_name.to_string()));
            }
        }

        // Calculate frame sizes
        let frame_size = usize::try_from(u64::from(width) * u64::from(height) * 3 / 2)
            .map_err(|_| VirtualCamError::InvalidDimensions(width, height))?;

        // Calculate total size with alignment
        let mut size = std::mem::size_of::<QueueHeader>();
        size = align_size(size);

        let mut offsets = [0u32; 3];
        for i in 0..3 {
            offsets[i] = size as u32;
            size = size
                .checked_add(FRAME_HEADER_SIZE + frame_size)
                .ok_or_else(|| VirtualCamError::InvalidDimensions(width, height))?;
            size = align_size(size);
        }
        let mapping_size =
            u32::try_from(size).map_err(|_| VirtualCamError::InvalidDimensions(width, height))?;

        // Create shared memory
        let file_mapping = unsafe {
            CreateFileMappingW(
                INVALID_HANDLE_VALUE,
                None,
                PAGE_READWRITE,
                0,
                mapping_size,
                PCWSTR(queue_name.as_ptr()),
            )?
        };

        if file_mapping.is_invalid() {
            return Err(VirtualCamError::SharedMemoryError(
                "Failed to create file mapping".to_string(),
            ));
        }

        let mapped = unsafe { MapViewOfFile(file_mapping, FILE_MAP_ALL_ACCESS, 0, 0, 0) };

        if mapped.Value.is_null() {
            unsafe { CloseHandle(file_mapping).ok() };
            return Err(VirtualCamError::SharedMemoryError(
                "Failed to map view of file".to_string(),
            ));
        }

        let header = mapped.Value as *mut QueueHeader;

        // Initialize header
        let interval = (10_000_000.0 / fps) as u64; // 100-nanosecond units

        unsafe {
            (*header).write_idx.store(0, Ordering::Relaxed);
            (*header).read_idx.store(0, Ordering::Relaxed);
            (*header)
                .state
                .store(SHARED_QUEUE_STATE_STARTING, Ordering::Release);
            (*header).offsets = offsets;
            (*header).queue_type = 0; // Video
            (*header).cx = width;
            (*header).cy = height;
            (*header).interval = interval;
            (*header).reserved = [0; 8];
        }

        // Setup frame pointers
        let base = mapped.Value as *mut u8;
        let mut frame_ts = [ptr::null_mut(); 3];
        let mut frame_data = [ptr::null_mut(); 3];

        for i in 0..3 {
            let offset = offsets[i] as usize;
            unsafe {
                frame_ts[i] = base.add(offset) as *mut u64;
                frame_data[i] = base.add(offset + FRAME_HEADER_SIZE);
            }
        }

        log::info!(
            "OBS Virtual Camera initialized: {}x{} @ {} fps",
            width,
            height,
            fps
        );

        Ok(Self {
            width,
            height,
            device: device_name.to_string(),
            is_open: true,
            file_mapping,
            header,
            frame_ts,
            frame_data,
        })
    }

    fn write_frame(&mut self, frame: &[u8], timestamp: u64) -> Result<()> {
        if !self.is_open {
            return Err(VirtualCamError::AlreadyClosed);
        }

        unsafe {
            // Increment write index
            let write_idx = (*self.header)
                .write_idx
                .fetch_add(1, Ordering::AcqRel)
                .wrapping_add(1);

            // Get buffer index (triple buffering)
            let idx = (write_idx % 3) as usize;

            // Write timestamp
            *self.frame_ts[idx] = timestamp;

            // Copy frame data (already in NV12 format)
            ptr::copy_nonoverlapping(frame.as_ptr(), self.frame_data[idx], frame.len());

            // Update read index
            (*self.header).read_idx.store(write_idx, Ordering::Release);

            // Mark as ready
            (*self.header)
                .state
                .store(SHARED_QUEUE_STATE_READY, Ordering::Release);
        }

        Ok(())
    }
}

impl Backend for ObsBackend {
    fn name(&self) -> &'static str {
        "obs"
    }

    fn device(&self) -> &str {
        &self.device
    }

    fn native_format(&self) -> PixelFormat {
        PixelFormat::NV12
    }

    fn native_dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    fn send(&mut self, frame: &[u8]) -> Result<()> {
        let expected_size = PixelFormat::NV12.frame_size(self.width, self.height);
        if frame.len() != expected_size {
            return Err(VirtualCamError::FrameSizeMismatch {
                expected: expected_size,
                actual: frame.len(),
            });
        }

        // Get timestamp in 100-nanosecond units
        let timestamp = get_timestamp_100ns();
        self.write_frame(frame, timestamp)
    }

    fn close(&mut self) -> Result<()> {
        if !self.is_open {
            return Ok(());
        }

        self.is_open = false;

        // Mark queue as stopping
        if !self.header.is_null() {
            unsafe {
                (*self.header).state.store(3, Ordering::Release);
            }
        }

        // Clean up handles
        unsafe {
            if !self.header.is_null() {
                UnmapViewOfFile(windows::Win32::System::Memory::MEMORY_MAPPED_VIEW_ADDRESS {
                    Value: self.header as *mut _,
                })
                .ok();
                self.header = ptr::null_mut();
            }

            if !self.file_mapping.is_invalid() {
                CloseHandle(self.file_mapping).ok();
                self.file_mapping = HANDLE::default();
            }
        }

        log::info!("OBS Virtual Camera closed");
        Ok(())
    }

    fn is_open(&self) -> bool {
        self.is_open
    }
}

impl Drop for ObsBackend {
    fn drop(&mut self) {
        let _ = self.close();
    }
}

/// Get timestamp in 100-nanosecond units (Windows FILETIME format)
fn get_timestamp_100ns() -> u64 {
    use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};

    let mut counter = 0i64;
    let mut frequency = 0i64;

    unsafe {
        QueryPerformanceCounter(&mut counter).ok();
        QueryPerformanceFrequency(&mut frequency).ok();
    }

    if frequency > 0 {
        // Convert to 100-nanosecond units
        ((counter as u128 * 10_000_000) / frequency as u128) as u64
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_the_obs_three_slot_protocol() {
        let mapping = format!("Local\\virtualcam-obs-test-{}", std::process::id());
        let mut backend = ObsBackend::open_queue(2, 2, 30.0, DEVICE_NAME, &mapping).unwrap();
        let frame = [16, 16, 16, 16, 128, 128];
        backend.send(&frame).unwrap();

        unsafe {
            assert_eq!((*backend.header).read_idx.load(Ordering::Acquire), 1);
            assert_eq!(
                (*backend.header).state.load(Ordering::Acquire),
                SHARED_QUEUE_STATE_READY
            );
            assert_eq!(std::slice::from_raw_parts(backend.frame_data[1], 6), frame);
        }
    }
}
