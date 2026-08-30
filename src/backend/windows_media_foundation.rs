//! Windows 11 Media Foundation backend used by the bundled VCam runtime.

use std::os::windows::ffi::OsStrExt;
use std::os::windows::fs::MetadataExt;
use std::path::{Component, Path, PathBuf, Prefix};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::Duration;

use windows::Win32::Foundation::{
    CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, HLOCAL, LocalFree,
};
use windows::Win32::Security::Authorization::{
    ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows::Win32::Security::{
    DACL_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR,
    SECURITY_ATTRIBUTES, SetKernelObjectSecurity,
};
use windows::Win32::System::Memory::{
    FILE_MAP_ALL_ACCESS, FILE_MAP_READ, MEMORY_MAPPED_VIEW_ADDRESS, MapViewOfFile,
    OpenFileMappingW, UnmapViewOfFile,
};
use windows::Win32::System::SystemInformation::OSVERSIONINFOW;
use windows::Win32::System::Threading::{
    CreateEventW, CreateMutexW, OpenEventW, ReleaseMutex, SYNCHRONIZATION_SYNCHRONIZE,
    WaitForSingleObject,
};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::SW_HIDE;
use windows::core::PCWSTR;
use winreg::RegKey;
use winreg::enums::{HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_64KEY};

use super::Backend;
use crate::error::{Result, VirtualCamError};
use crate::pixel_format::PixelFormat;

#[path = "windows_media_foundation_contract.rs"]
mod ipc_contract;

use ipc_contract::{
    HEADER_SIZE, MAX_FRAME_SIZE, RawHeaderSnapshot, WAIT_OBJECT_0_RAW, WAIT_TIMEOUT_RAW,
    WRITER_MUTEX_NAME, WriterLeaseDisposition, classify_writer_wait, ipc_security_sddl,
    startup_ack_name, validate_stable_header,
};
pub use ipc_contract::{MAX_HEIGHT, MAX_WIDTH, SHM_NAME};

const REGISTRAR_READY_EVENT: &str = "Global\\vcam_registrar_ready";
pub const OUTPUT_WIDTH: u32 = 1280;
pub const OUTPUT_HEIGHT: u32 = 720;
pub const OUTPUT_FPS: u32 = 30;

const DEVICE_NAME: &str = "VCam";
const VCAM_SOURCE_CLSID: &str = "{3cad447d-f283-4af4-a3b2-6f5363309f52}";
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
const CSTR_EQUAL: i32 = 2;

#[link(name = "ntdll")]
unsafe extern "system" {
    fn RtlGetVersion(version: *mut OSVERSIONINFOW) -> i32;
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn CompareStringOrdinal(
        string1: *const u16,
        count1: i32,
        string2: *const u16,
        count2: i32,
        ignore_case: i32,
    ) -> i32;
}

fn supports_media_foundation_virtual_camera() -> bool {
    let mut version = OSVERSIONINFOW {
        dwOSVersionInfoSize: std::mem::size_of::<OSVERSIONINFOW>() as u32,
        ..Default::default()
    };
    unsafe { RtlGetVersion(&mut version) >= 0 && version.dwBuildNumber >= 22000 }
}

pub fn is_available() -> bool {
    supports_media_foundation_virtual_camera() && registered_registrar_path().is_ok()
}

#[repr(C)]
struct ShmHeader {
    magic: u32,
    version: u32,
    header_bytes: u32,
    width: u32,
    height: u32,
    fps_num: u32,
    fps_den: u32,
    frame_bytes: u32,
    seq: AtomicU32,
    ready: AtomicU32,
    reserved: [u32; 2],
}

const _: () = assert!(std::mem::size_of::<ShmHeader>() == HEADER_SIZE);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SharedFormat {
    pub width: u32,
    pub height: u32,
    pub fps_num: u32,
    pub fps_den: u32,
}

impl SharedFormat {
    pub fn new(width: u32, height: u32, fps: f64) -> Result<Self> {
        if width == 0
            || height == 0
            || width > MAX_WIDTH
            || height > MAX_HEIGHT
            || width % 2 != 0
            || height % 2 != 0
        {
            return Err(VirtualCamError::InvalidDimensions(width, height));
        }
        let (fps_num, fps_den) = fps_rational(fps)?;
        Ok(Self {
            width,
            height,
            fps_num,
            fps_den,
        })
    }

    fn frame_size(self) -> usize {
        self.width as usize * self.height as usize * 3 / 2
    }
}

fn fps_rational(fps: f64) -> Result<(u32, u32)> {
    if !fps.is_finite() || fps <= 0.0 || fps > 1000.0 {
        return Err(VirtualCamError::InvalidFps(fps));
    }
    if fps.fract() == 0.0 {
        return Ok((fps as u32, 1));
    }

    const DENOMINATOR: u64 = 1_000_000;
    let numerator = (fps * DENOMINATOR as f64).round() as u64;
    if numerator == 0 || numerator > u32::MAX as u64 {
        return Err(VirtualCamError::InvalidFps(fps));
    }
    let divisor = gcd(numerator, DENOMINATOR);
    Ok(((numerator / divisor) as u32, (DENOMINATOR / divisor) as u32))
}

const fn gcd(mut left: u64, mut right: u64) -> u64 {
    while right != 0 {
        let remainder = left % right;
        left = right;
        right = remainder;
    }
    left
}

fn registrar_replace_arguments(format: SharedFormat, startup_ack_name: &str) -> String {
    format!(
        "/headless /replace {} {} {} {} /ack {}",
        format.width, format.height, format.fps_num, format.fps_den, startup_ack_name
    )
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

struct IpcSecurityDescriptor {
    raw: PSECURITY_DESCRIPTOR,
}

impl IpcSecurityDescriptor {
    fn new() -> anyhow::Result<Self> {
        let sddl = wide(&ipc_security_sddl());
        let mut raw = PSECURITY_DESCRIPTOR::default();
        unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                PCWSTR(sddl.as_ptr()),
                SDDL_REVISION_1,
                &mut raw,
                None,
            )?;
        }
        anyhow::ensure!(
            !raw.is_invalid(),
            "Win32 returned an empty IPC security descriptor"
        );
        Ok(Self { raw })
    }

    fn attributes(&self) -> SECURITY_ATTRIBUTES {
        SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: self.raw.0,
            bInheritHandle: false.into(),
        }
    }

    fn protect(&self, handle: HANDLE) -> anyhow::Result<()> {
        unsafe {
            SetKernelObjectSecurity(
                handle,
                DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                self.raw,
            )?;
        }
        Ok(())
    }
}

impl Drop for IpcSecurityDescriptor {
    fn drop(&mut self) {
        if !self.raw.is_invalid() {
            unsafe {
                let _ = LocalFree(Some(HLOCAL(self.raw.0)));
            }
            self.raw = PSECURITY_DESCRIPTOR::default();
        }
    }
}

struct WriterLease {
    mutex: HANDLE,
}

impl WriterLease {
    fn acquire() -> anyhow::Result<Self> {
        let security = IpcSecurityDescriptor::new()?;
        let attributes = security.attributes();
        let name = wide(WRITER_MUTEX_NAME);
        let mutex = unsafe {
            CreateMutexW(
                Some(&attributes as *const SECURITY_ATTRIBUTES),
                false,
                PCWSTR(name.as_ptr()),
            )
        }?;

        if let Err(error) = security.protect(mutex) {
            unsafe {
                let _ = CloseHandle(mutex);
            }
            return Err(error.context("apply the writer-mutex DACL"));
        }

        let wait_result = unsafe { WaitForSingleObject(mutex, 0) }.0;
        match classify_writer_wait(wait_result) {
            WriterLeaseDisposition::Acquired => Ok(Self { mutex }),
            WriterLeaseDisposition::Busy => {
                unsafe {
                    let _ = CloseHandle(mutex);
                }
                anyhow::bail!(
                    "another Media Foundation frame producer already owns the writer lease"
                )
            }
            WriterLeaseDisposition::Failed(raw) => {
                let error = unsafe { GetLastError() };
                unsafe {
                    let _ = CloseHandle(mutex);
                }
                anyhow::bail!(
                    "wait for the Media Foundation writer lease failed (wait={raw:#x}, Win32={})",
                    error.0
                )
            }
        }
    }
}

impl Drop for WriterLease {
    fn drop(&mut self) {
        if !self.mutex.is_invalid() {
            unsafe {
                let _ = ReleaseMutex(self.mutex);
                let _ = CloseHandle(self.mutex);
            }
            self.mutex = HANDLE::default();
        }
    }
}

static STARTUP_ACK_GENERATION: AtomicU64 = AtomicU64::new(1);

struct StartupAck {
    event: HANDLE,
    name: String,
}

impl StartupAck {
    fn create() -> anyhow::Result<Self> {
        for _ in 0..16 {
            let generation = STARTUP_ACK_GENERATION.fetch_add(1, Ordering::Relaxed);
            let name = startup_ack_name(std::process::id(), generation);
            let wide_name = wide(&name);
            let event = unsafe { CreateEventW(None, true, false, PCWSTR(wide_name.as_ptr())) }?;
            if unsafe { GetLastError() } != ERROR_ALREADY_EXISTS {
                return Ok(Self { event, name });
            }
            unsafe {
                let _ = CloseHandle(event);
            }
        }
        anyhow::bail!("could not reserve a unique registrar startup acknowledgment")
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn wait(&self, timeout: Duration) -> anyhow::Result<()> {
        let timeout_ms = u32::try_from(timeout.as_millis()).unwrap_or(u32::MAX - 1);
        let wait_result = unsafe { WaitForSingleObject(self.event, timeout_ms) }.0;
        match wait_result {
            WAIT_OBJECT_0_RAW => Ok(()),
            WAIT_TIMEOUT_RAW => anyhow::bail!(
                "VCam mode helper did not acknowledge generation '{}' within {} seconds",
                self.name,
                timeout.as_secs()
            ),
            other => {
                let error = unsafe { GetLastError() };
                anyhow::bail!(
                    "wait for VCam startup acknowledgment '{}' failed (wait={other:#x}, Win32={})",
                    self.name,
                    error.0
                )
            }
        }
    }
}

impl Drop for StartupAck {
    fn drop(&mut self) {
        if !self.event.is_invalid() {
            unsafe {
                let _ = CloseHandle(self.event);
            }
            self.event = HANDLE::default();
        }
    }
}

unsafe fn snapshot_header(header: *const ShmHeader) -> RawHeaderSnapshot {
    RawHeaderSnapshot {
        magic: unsafe { std::ptr::read_volatile(std::ptr::addr_of!((*header).magic)) },
        version: unsafe { std::ptr::read_volatile(std::ptr::addr_of!((*header).version)) },
        header_bytes: unsafe {
            std::ptr::read_volatile(std::ptr::addr_of!((*header).header_bytes))
        },
        width: unsafe { std::ptr::read_volatile(std::ptr::addr_of!((*header).width)) },
        height: unsafe { std::ptr::read_volatile(std::ptr::addr_of!((*header).height)) },
        fps_num: unsafe { std::ptr::read_volatile(std::ptr::addr_of!((*header).fps_num)) },
        fps_den: unsafe { std::ptr::read_volatile(std::ptr::addr_of!((*header).fps_den)) },
        frame_bytes: unsafe { std::ptr::read_volatile(std::ptr::addr_of!((*header).frame_bytes)) },
        seq: unsafe { (&*std::ptr::addr_of!((*header).seq)).load(Ordering::Acquire) },
        ready: unsafe { (&*std::ptr::addr_of!((*header).ready)).load(Ordering::Acquire) },
    }
}

fn current_shared_format() -> Option<SharedFormat> {
    let name = wide(SHM_NAME);
    let mapping =
        unsafe { OpenFileMappingW(FILE_MAP_READ.0, false, PCWSTR(name.as_ptr())) }.ok()?;
    let mapped = unsafe { MapViewOfFile(mapping, FILE_MAP_READ, 0, 0, HEADER_SIZE) };
    if mapped.Value.is_null() {
        unsafe {
            let _ = CloseHandle(mapping);
        }
        return None;
    }
    let header = mapped.Value.cast::<ShmHeader>();
    let seq_before = unsafe { (&*std::ptr::addr_of!((*header).seq)).load(Ordering::Acquire) };
    let snapshot = unsafe { snapshot_header(header) };
    let seq_after = unsafe { (&*std::ptr::addr_of!((*header).seq)).load(Ordering::Acquire) };
    let format = validate_stable_header(seq_before, snapshot, seq_after)
        .ok()
        .map(|validated| SharedFormat {
            width: validated.width,
            height: validated.height,
            fps_num: validated.fps_num,
            fps_den: validated.fps_den,
        });
    unsafe {
        let _ = UnmapViewOfFile(mapped);
        let _ = CloseHandle(mapping);
    }
    format
}

fn registrar_is_ready() -> bool {
    let name = wide(REGISTRAR_READY_EVENT);
    let Ok(event) =
        (unsafe { OpenEventW(SYNCHRONIZATION_SYNCHRONIZE, false, PCWSTR(name.as_ptr())) })
    else {
        return false;
    };
    unsafe {
        let _ = CloseHandle(event);
    }
    true
}

fn canonical_paths_equal_case_insensitive(left: &Path, right: &Path) -> anyhow::Result<bool> {
    let left: Vec<u16> = left.as_os_str().encode_wide().collect();
    let right: Vec<u16> = right.as_os_str().encode_wide().collect();
    let left_len = i32::try_from(left.len())
        .map_err(|_| anyhow::anyhow!("canonical path is too long for Windows comparison"))?;
    let right_len = i32::try_from(right.len())
        .map_err(|_| anyhow::anyhow!("canonical path is too long for Windows comparison"))?;
    let comparison =
        unsafe { CompareStringOrdinal(left.as_ptr(), left_len, right.as_ptr(), right_len, 1) };
    anyhow::ensure!(
        comparison != 0,
        "CompareStringOrdinal failed with Win32 error {}",
        unsafe { GetLastError() }.0
    );
    Ok(comparison == CSTR_EQUAL)
}

fn ensure_shell_compatible_absolute_path(path: &Path) -> anyhow::Result<()> {
    let mut components = path.components();
    anyhow::ensure!(
        matches!(
            components.next(),
            Some(Component::Prefix(prefix)) if matches!(prefix.kind(), Prefix::Disk(_))
        ) && matches!(components.next(), Some(Component::RootDir)),
        "trusted runtime path is not a normal drive-absolute Windows path: {}",
        path.display()
    );
    Ok(())
}

fn checked_path_metadata(path: &Path) -> anyhow::Result<std::fs::Metadata> {
    anyhow::ensure!(path.is_absolute(), "trusted runtime path is not absolute");
    let mut leaf = None;
    for (index, ancestor) in path.ancestors().enumerate() {
        let metadata = std::fs::symlink_metadata(ancestor).map_err(|error| {
            anyhow::anyhow!(
                "cannot inspect trusted path '{}': {error}",
                ancestor.display()
            )
        })?;
        anyhow::ensure!(
            metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT == 0,
            "trusted runtime path contains a reparse point: {}",
            ancestor.display()
        );
        if index == 0 {
            leaf = Some(metadata);
        }
    }
    leaf.ok_or_else(|| anyhow::anyhow!("trusted runtime path is empty"))
}

fn canonical_directory(path: &Path, label: &str) -> anyhow::Result<PathBuf> {
    let metadata = checked_path_metadata(path)?;
    anyhow::ensure!(
        metadata.is_dir(),
        "{label} is not a directory: {}",
        path.display()
    );
    std::fs::canonicalize(path).map_err(|error| {
        anyhow::anyhow!("cannot canonicalize {label} '{}': {error}", path.display())
    })
}

fn canonical_regular_file(path: &Path, label: &str) -> anyhow::Result<PathBuf> {
    let metadata = checked_path_metadata(path)?;
    anyhow::ensure!(
        metadata.is_file(),
        "{label} is not a regular file: {}",
        path.display()
    );
    std::fs::canonicalize(path).map_err(|error| {
        anyhow::anyhow!("cannot canonicalize {label} '{}': {error}", path.display())
    })
}

fn registered_registrar_path() -> anyhow::Result<PathBuf> {
    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    let registry_flags = KEY_READ | KEY_WOW64_64KEY;
    let source_key = hklm.open_subkey_with_flags(
        format!("SOFTWARE\\Classes\\CLSID\\{VCAM_SOURCE_CLSID}\\InprocServer32"),
        registry_flags,
    )?;
    let registered_dll = PathBuf::from(source_key.get_value::<String, _>("")?);
    ensure_shell_compatible_absolute_path(&registered_dll)?;

    let current_version = hklm.open_subkey_with_flags(
        "SOFTWARE\\Microsoft\\Windows\\CurrentVersion",
        registry_flags,
    )?;
    let program_files = PathBuf::from(current_version.get_value::<String, _>("ProgramFilesDir")?);
    ensure_shell_compatible_absolute_path(&program_files)?;
    let expected_runtime = program_files.join("noperson");
    let expected_dll = expected_runtime.join("VCamSource.dll");
    let expected_registrar = expected_runtime.join("VCamRegistrar.exe");

    let canonical_program_files = canonical_directory(&program_files, "ProgramFilesDir")?;
    let canonical_runtime = canonical_directory(&expected_runtime, "noperson runtime")?;
    let runtime_parent = canonical_runtime
        .parent()
        .ok_or_else(|| anyhow::anyhow!("canonical noperson runtime has no parent"))?;
    anyhow::ensure!(
        canonical_paths_equal_case_insensitive(runtime_parent, &canonical_program_files)?,
        "noperson runtime is outside canonical ProgramFilesDir"
    );

    let canonical_expected_dll = canonical_regular_file(&expected_dll, "expected VCamSource.dll")?;
    let canonical_registered_dll =
        canonical_regular_file(&registered_dll, "registered VCamSource.dll")?;
    anyhow::ensure!(
        canonical_paths_equal_case_insensitive(&canonical_registered_dll, &canonical_expected_dll,)?,
        "registered VCamSource.dll is outside the trusted runtime"
    );

    let registrar_sibling = canonical_registered_dll
        .parent()
        .ok_or_else(|| anyhow::anyhow!("canonical VCamSource.dll has no parent"))?
        .join("VCamRegistrar.exe");
    let canonical_registrar =
        canonical_regular_file(&registrar_sibling, "registered VCamRegistrar.exe")?;
    let canonical_expected_registrar =
        canonical_regular_file(&expected_registrar, "expected VCamRegistrar.exe")?;
    anyhow::ensure!(
        canonical_paths_equal_case_insensitive(
            &canonical_registrar,
            &canonical_expected_registrar,
        )?,
        "VCamRegistrar.exe is not the canonical sibling of VCamSource.dll"
    );
    Ok(expected_registrar)
}

fn ensure_registrar_format(format: SharedFormat) -> anyhow::Result<()> {
    if current_shared_format() == Some(format) && registrar_is_ready() {
        return Ok(());
    }

    let startup_ack = StartupAck::create()?;
    let registrar = registered_registrar_path()?;
    let arguments = registrar_replace_arguments(format, startup_ack.name());
    let operation = wide("runas");
    let file = wide(&registrar.to_string_lossy());
    let parameters = wide(&arguments);
    let directory = wide(
        registrar
            .parent()
            .expect("registered helper has a parent")
            .to_string_lossy()
            .as_ref(),
    );
    let result = unsafe {
        ShellExecuteW(
            None,
            PCWSTR(operation.as_ptr()),
            PCWSTR(file.as_ptr()),
            PCWSTR(parameters.as_ptr()),
            PCWSTR(directory.as_ptr()),
            SW_HIDE,
        )
    };
    anyhow::ensure!(
        result.0 as isize > 32,
        "mode helper launch was denied or failed (ShellExecute={})",
        result.0 as isize
    );

    startup_ack.wait(Duration::from_secs(20))?;
    anyhow::ensure!(
        current_shared_format() == Some(format) && registrar_is_ready(),
        "VCam generation '{}' acknowledged without publishing {}x{} @ {}/{}",
        startup_ack.name(),
        format.width,
        format.height,
        format.fps_num,
        format.fps_den
    );
    Ok(())
}

struct FrameWriter {
    mapping: HANDLE,
    view: *mut u8,
    frame_size: usize,
    _writer_lease: WriterLease,
}

unsafe impl Send for FrameWriter {}

impl FrameWriter {
    fn open(format: SharedFormat, writer_lease: WriterLease) -> anyhow::Result<Self> {
        let frame_size = format.frame_size();
        let mapping_size = HEADER_SIZE + MAX_FRAME_SIZE;

        let name = wide(SHM_NAME);
        let mapping =
            unsafe { OpenFileMappingW(FILE_MAP_ALL_ACCESS.0, false, PCWSTR(name.as_ptr())) }?;
        let mapped = unsafe { MapViewOfFile(mapping, FILE_MAP_ALL_ACCESS, 0, 0, mapping_size) };
        if mapped.Value.is_null() {
            unsafe {
                let _ = CloseHandle(mapping);
            }
            anyhow::bail!("MapViewOfFile returned null");
        }

        let view = mapped.Value.cast::<u8>();
        let header = view.cast::<ShmHeader>();
        let seq_before = unsafe { (&*std::ptr::addr_of!((*header).seq)).load(Ordering::Acquire) };
        let snapshot = unsafe { snapshot_header(header) };
        let seq_after = unsafe { (&*std::ptr::addr_of!((*header).seq)).load(Ordering::Acquire) };
        let validated = match validate_stable_header(seq_before, snapshot, seq_after) {
            Ok(validated) => validated,
            Err(error) => {
                unsafe {
                    let _ = UnmapViewOfFile(mapped);
                    let _ = CloseHandle(mapping);
                }
                anyhow::bail!("registrar mapping header is invalid: {error:?}");
            }
        };
        if validated.width != format.width
            || validated.height != format.height
            || validated.fps_num != format.fps_num
            || validated.fps_den != format.fps_den
            || validated.frame_bytes as usize != frame_size
        {
            unsafe {
                let _ = UnmapViewOfFile(mapped);
                let _ = CloseHandle(mapping);
            }
            anyhow::bail!(
                "registrar mapping format is {}x{} @ {}/{}, expected {}x{} @ {}/{}",
                validated.width,
                validated.height,
                validated.fps_num,
                validated.fps_den,
                format.width,
                format.height,
                format.fps_num,
                format.fps_den
            );
        }
        unsafe { (&*std::ptr::addr_of!((*header).ready)).store(0, Ordering::Release) };

        Ok(Self {
            mapping,
            view,
            frame_size,
            _writer_lease: writer_lease,
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
        let seq = header.seq.load(Ordering::Acquire);
        anyhow::ensure!(
            seq & 1 == 0,
            "shared-frame sequence is already writer-active"
        );
        header
            .seq
            .compare_exchange(
                seq,
                seq.wrapping_add(1),
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .map_err(|_| {
                anyhow::anyhow!("shared-frame sequence changed without the writer lease")
            })?;
        unsafe {
            std::ptr::copy_nonoverlapping(frame.as_ptr(), self.view.add(HEADER_SIZE), frame.len());
        }
        header.seq.store(seq.wrapping_add(2), Ordering::Release);
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
    format: SharedFormat,
}

impl MediaFoundationBackend {
    pub fn new(width: u32, height: u32, fps: f64, device: Option<&str>) -> Result<Self> {
        let format = SharedFormat::new(width, height, fps)?;
        if !is_available() {
            return Err(VirtualCamError::BackendNotAvailable(
                "media-foundation (requires Windows 11 and installed VCam runtime)".into(),
            ));
        }
        if device.is_some_and(|name| !name.eq_ignore_ascii_case(DEVICE_NAME)) {
            return Err(VirtualCamError::DeviceNotFound(device.unwrap().to_owned()));
        }
        let writer_lease = WriterLease::acquire()
            .map_err(|error| VirtualCamError::InitializationFailed(error.to_string()))?;
        ensure_registrar_format(format)
            .map_err(|error| VirtualCamError::InitializationFailed(error.to_string()))?;
        let writer = FrameWriter::open(format, writer_lease)
            .map_err(|error| VirtualCamError::InitializationFailed(error.to_string()))?;
        Ok(Self {
            writer: Some(writer),
            format,
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
        (self.format.width, self.format.height)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_header_layout_is_stable() {
        assert_eq!(std::mem::size_of::<ShmHeader>(), 48);
    }

    #[test]
    fn dynamic_format_accepts_1080p60_and_rejects_odd_or_oversized_frames() {
        assert!(SharedFormat::new(1920, 1080, 60.0).is_ok());
        assert!(SharedFormat::new(1919, 1080, 60.0).is_err());
        assert!(SharedFormat::new(7680, 4320, 30.0).is_err());
    }

    #[test]
    fn shared_format_preserves_fractional_frame_rates() {
        let format = SharedFormat::new(1920, 1080, 29.97).unwrap();
        assert_eq!((format.fps_num, format.fps_den), (2997, 100));
    }

    #[test]
    fn active_frame_size_matches_nv12_geometry() {
        let format = SharedFormat::new(1920, 1080, 60.0).unwrap();
        assert_eq!(format.frame_size(), 1920 * 1080 * 3 / 2);
        assert!(format.frame_size() <= MAX_FRAME_SIZE);
    }

    #[test]
    fn registrar_replace_arguments_carry_the_exact_mode() {
        let format = SharedFormat::new(1920, 1080, 29.97).unwrap();
        assert_eq!(
            registrar_replace_arguments(format, "Local\\noperson_vcam_startup_test"),
            "/headless /replace 1920 1080 2997 100 /ack Local\\noperson_vcam_startup_test"
        );
    }
}
