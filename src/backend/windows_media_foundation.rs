//! Windows 11 Media Foundation backend used by the bundled VCam runtime.

use std::ffi::c_void;
use std::sync::atomic::{AtomicU32, Ordering};

use windows::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
use windows::Win32::Security::{
    InitializeSecurityDescriptor, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, SECURITY_DESCRIPTOR,
    SetSecurityDescriptorDacl,
};
use windows::Win32::System::Memory::{
    CreateFileMappingW, FILE_MAP_ALL_ACCESS, MEMORY_MAPPED_VIEW_ADDRESS, MapViewOfFile,
    PAGE_READWRITE, UnmapViewOfFile,
};
use windows::Win32::System::SystemInformation::OSVERSIONINFOW;
use windows::core::PCWSTR;
use winreg::RegKey;
use winreg::enums::HKEY_CLASSES_ROOT;

use super::Backend;
use crate::error::{Result, VirtualCamError};
use crate::pixel_format::PixelFormat;

pub const SHM_NAME: &str = "Global\\vcam_frames";
pub const OUTPUT_WIDTH: u32 = 1280;
pub const OUTPUT_HEIGHT: u32 = 960;
pub const OUTPUT_FPS: u32 = 30;

const DEVICE_NAME: &str = "VCam";
const VCAM_SOURCE_CLSID: &str = "{3cad447d-f283-4af4-a3b2-6f5363309f52}";
const SHM_MAGIC: u32 = 0x4d414356;

#[link(name = "ntdll")]
unsafe extern "system" {
    fn RtlGetVersion(version: *mut OSVERSIONINFOW) -> i32;
}

fn supports_media_foundation_virtual_camera() -> bool {
    let mut version = OSVERSIONINFOW {
        dwOSVersionInfoSize: std::mem::size_of::<OSVERSIONINFOW>() as u32,
        ..Default::default()
    };
    unsafe { RtlGetVersion(&mut version) >= 0 && version.dwBuildNumber >= 22000 }
}

pub fn is_available() -> bool {
    if !supports_media_foundation_virtual_camera() {
        return false;
    }
    RegKey::predef(HKEY_CLASSES_ROOT)
        .open_subkey(format!("CLSID\\{VCAM_SOURCE_CLSID}\\InprocServer32"))
        .is_ok()
}

#[repr(C)]
struct ShmHeader {
    magic: u32,
    width: u32,
    height: u32,
    seq: AtomicU32,
    ready: AtomicU32,
    pad: [u32; 3],
}

const HEADER_SIZE: usize = std::mem::size_of::<ShmHeader>();

struct FrameWriter {
    mapping: HANDLE,
    view: *mut u8,
    frame_size: usize,
}

unsafe impl Send for FrameWriter {}

impl FrameWriter {
    fn new() -> anyhow::Result<Self> {
        let frame_size = OUTPUT_WIDTH as usize * OUTPUT_HEIGHT as usize * 3 / 2;
        let mapping_size = HEADER_SIZE + frame_size;

        // Frame Server runs outside the producer session and must be able to
        // open the global mapping. This matches the native VCam contract.
        let mut descriptor = SECURITY_DESCRIPTOR::default();
        unsafe {
            InitializeSecurityDescriptor(
                PSECURITY_DESCRIPTOR((&mut descriptor as *mut SECURITY_DESCRIPTOR).cast()),
                1,
            )?;
            SetSecurityDescriptorDacl(
                PSECURITY_DESCRIPTOR((&mut descriptor as *mut SECURITY_DESCRIPTOR).cast()),
                true,
                None,
                false,
            )?;
        }
        let attributes = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: (&mut descriptor as *mut SECURITY_DESCRIPTOR).cast::<c_void>(),
            bInheritHandle: false.into(),
        };
        let name: Vec<u16> = SHM_NAME.encode_utf16().chain(std::iter::once(0)).collect();
        let mapping = unsafe {
            CreateFileMappingW(
                INVALID_HANDLE_VALUE,
                Some(&attributes),
                PAGE_READWRITE,
                0,
                mapping_size as u32,
                PCWSTR(name.as_ptr()),
            )
        }?;
        let mapped = unsafe { MapViewOfFile(mapping, FILE_MAP_ALL_ACCESS, 0, 0, mapping_size) };
        if mapped.Value.is_null() {
            unsafe { CloseHandle(mapping) }?;
            anyhow::bail!("MapViewOfFile returned null");
        }

        let view = mapped.Value.cast::<u8>();
        let header = unsafe { &mut *view.cast::<ShmHeader>() };
        header.magic = SHM_MAGIC;
        header.width = OUTPUT_WIDTH;
        header.height = OUTPUT_HEIGHT;
        header.seq.store(0, Ordering::Relaxed);
        header.ready.store(0, Ordering::Release);
        header.pad = [0; 3];

        Ok(Self {
            mapping,
            view,
            frame_size,
        })
    }

    fn send(&mut self, frame: &[u8]) -> anyhow::Result<()> {
        anyhow::ensure!(
            frame.len() == self.frame_size,
            "NV12 frame has {} bytes, expected {}",
            frame.len(),
            self.frame_size
        );
        let header = unsafe { &*self.view.cast::<ShmHeader>() };
        header.seq.fetch_add(1, Ordering::AcqRel);
        unsafe {
            std::ptr::copy_nonoverlapping(frame.as_ptr(), self.view.add(HEADER_SIZE), frame.len());
        }
        header.seq.fetch_add(1, Ordering::Release);
        header.ready.store(1, Ordering::Release);
        Ok(())
    }
}

impl Drop for FrameWriter {
    fn drop(&mut self) {
        if !self.view.is_null() {
            let header = unsafe { &*self.view.cast::<ShmHeader>() };
            header.ready.store(0, Ordering::Release);
            unsafe {
                let _ = UnmapViewOfFile(MEMORY_MAPPED_VIEW_ADDRESS {
                    Value: self.view.cast(),
                });
            }
            self.view = std::ptr::null_mut();
        }
        if !self.mapping.is_invalid() {
            unsafe {
                let _ = CloseHandle(self.mapping);
            }
            self.mapping = HANDLE::default();
        }
    }
}

pub struct MediaFoundationBackend {
    writer: Option<FrameWriter>,
}

impl MediaFoundationBackend {
    pub fn new(_width: u32, _height: u32, _fps: f64, device: Option<&str>) -> Result<Self> {
        if !is_available() {
            return Err(VirtualCamError::BackendNotAvailable(
                "media-foundation (requires Windows 11 and installed VCam runtime)".into(),
            ));
        }
        if device.is_some_and(|name| !name.eq_ignore_ascii_case(DEVICE_NAME)) {
            return Err(VirtualCamError::DeviceNotFound(device.unwrap().to_owned()));
        }
        let writer = FrameWriter::new()
            .map_err(|error| VirtualCamError::InitializationFailed(error.to_string()))?;
        Ok(Self {
            writer: Some(writer),
        })
    }
}

impl Backend for MediaFoundationBackend {
    fn name(&self) -> &'static str {
        "media-foundation"
    }

    fn device(&self) -> &str {
        DEVICE_NAME
    }

    fn native_format(&self) -> PixelFormat {
        PixelFormat::NV12
    }

    fn native_dimensions(&self) -> (u32, u32) {
        (OUTPUT_WIDTH, OUTPUT_HEIGHT)
    }

    fn send(&mut self, frame: &[u8]) -> Result<()> {
        self.writer
            .as_mut()
            .ok_or(VirtualCamError::AlreadyClosed)?
            .send(frame)
            .map_err(|error| VirtualCamError::SendFailed(error.to_string()))
    }

    fn close(&mut self) -> Result<()> {
        self.writer.take();
        Ok(())
    }

    fn is_open(&self) -> bool {
        self.writer.is_some()
    }
}
