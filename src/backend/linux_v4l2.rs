#![cfg(target_os = "linux")]
//! Virtual camera output via v4l2loopback — mmap streaming path.
//!
//! Writes RGB24 frames to /dev/videoN through the V4L2 mmap streaming API:
//! QUERYCAP → S_FMT → S_PARM → REQBUFS → QUERYBUF+mmap → QBUF → STREAMON,
//! then per-frame DQBUF → memcpy → QBUF. Drop does STREAMOFF → munmap → close.
//!
//! This is the Rust equivalent of v4l2loopback's own `tests/producer.c`
//! (IO_METHOD_MMAP), NOT pyvirtualcam (which uses the simpler write() path).
//! mmap is what streaming-only consumers (GStreamer v4l2sink, OBS, browsers)
//! actually want — and it's strictly more capable than write().
//!
//! Requirements:
//!   sudo modprobe v4l2loopback video_nr=10 exclusive_caps=1
//!
//! exclusive_caps=1 is REQUIRED for Chrome/WebRTC: the device advertises
//! OUTPUT until a producer calls S_FMT(OUTPUT), then flips to CAPTURE-only
//! so browsers/OBS (which refuse mixed-cap devices) can see it.
//!
//! Applications access this transport through `CameraBuilder` and
//! `BackendKind::V4l2`; the mmap writer remains private to the backend.

use std::ffi::c_void;
use std::fs::OpenOptions;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::io::AsRawFd;
use std::ptr;

use anyhow::{Context, ensure};
use libc::{c_int, c_ulong, c_void as libc_c_void, off_t, size_t};

use super::{Backend, ShutdownHandle};
use crate::error::{Result, VirtualCamError};
use crate::pixel_format::PixelFormat;

// Linux asm-generic ioctl encoding. Sizes come from the current target ABI,
// avoiding the x86-64-only request constants used by the original app code.
const IOC_WRITE: c_ulong = 1;
const IOC_READ: c_ulong = 2;
const fn ioctl_code<T>(direction: c_ulong, number: c_ulong) -> c_ulong {
    (direction << 30)
        | ((std::mem::size_of::<T>() as c_ulong) << 16)
        | ((b'V' as c_ulong) << 8)
        | number
}

const VIDIOC_QUERYCAP: c_ulong = ioctl_code::<V4l2Capability>(IOC_READ, 0);
const VIDIOC_S_FMT: c_ulong = ioctl_code::<V4l2Format>(IOC_READ | IOC_WRITE, 5);
const VIDIOC_REQBUFS: c_ulong = ioctl_code::<V4l2Requestbuffers>(IOC_READ | IOC_WRITE, 8);
const VIDIOC_QUERYBUF: c_ulong = ioctl_code::<V4l2Buffer>(IOC_READ | IOC_WRITE, 9);
const VIDIOC_QBUF: c_ulong = ioctl_code::<V4l2Buffer>(IOC_READ | IOC_WRITE, 15);
const VIDIOC_DQBUF: c_ulong = ioctl_code::<V4l2Buffer>(IOC_READ | IOC_WRITE, 17);
const VIDIOC_STREAMON: c_ulong = ioctl_code::<u32>(IOC_WRITE, 18);
const VIDIOC_STREAMOFF: c_ulong = ioctl_code::<u32>(IOC_WRITE, 19);
const VIDIOC_S_PARM: c_ulong = ioctl_code::<V4l2Streamparm>(IOC_READ | IOC_WRITE, 22);

// ── V4L2 constants (from linux/videodev2.h) ──
const V4L2_PIX_FMT_RGB24: u32 = 0x33424752; // 'R','G','B' little-endian
const V4L2_BUF_TYPE_VIDEO_OUTPUT: u32 = 2;
const V4L2_MEMORY_MMAP: u32 = 1;
const V4L2_FIELD_NONE: u32 = 1;
const V4L2_CAP_VIDEO_OUTPUT: u32 = 0x00000002;
const V4L2_CAP_STREAMING: u32 = 0x04000000;
const V4L2_CAP_DEVICE_CAPS: u32 = 0x80000000;

// ── struct layouts (#[repr(C)], verified against videodev2.h) ──

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct V4l2Timecode {
    typ: u32,
    flags: u32,
    frames: u8,
    seconds: u8,
    minutes: u8,
    hours: u8,
    userbits: [u8; 4],
} // 16 bytes, align 4

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
    ycbcr_enc: u32, // union with hsv_enc — both u32
    quantization: u32,
    xfer_func: u32,
} // 48 bytes, align 4

/// `v4l2_format.fmt` union — must be 200 bytes with align 8 (the C union
/// contains pointer members like `v4l2_pix_format_mplane`, so its natural
/// alignment is 8). `#[repr(C, align(8))]` forces the 4-byte tail padding
/// so `V4l2Format` = 4 + 200 + 4(pad) = 208 bytes, matching the ioctl number.
#[repr(C)]
union V4l2FormatUnion {
    pix: V4l2PixFormat,
    raw_data: [u8; 200],
    align: *mut c_void,
}

#[repr(C)]
struct V4l2Format {
    typ: u32,
    fmt: V4l2FormatUnion,
} // 4 + 200 + 4 tail pad (align 8) = 208 bytes, align 8

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct V4l2Capability {
    driver: [u8; 16],
    card: [u8; 32],
    bus_info: [u8; 32],
    version: u32,
    capabilities: u32,
    device_caps: u32,
    reserved: [u32; 3],
} // 104 bytes

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct V4l2Requestbuffers {
    count: u32,
    typ: u32,
    memory: u32,
    capabilities: u32,
    flags: u8,
    reserved: [u8; 3],
} // 20 bytes, align 4

/// `v4l2_buffer.m` union — largest member is `unsigned long`/pointer = 8 bytes on x86-64.
/// For the MMAP OUTPUT path only `offset` is read/written by userspace.
#[repr(C)]
#[derive(Clone, Copy)]
union V4l2BufferM {
    offset: u32,
    userptr: libc::c_ulong,
    planes: *mut c_void,
    fd: i32,
}

#[repr(C)]
struct V4l2Buffer {
    index: u32,
    typ: u32,
    bytesused: u32,
    flags: u32,
    field: u32,
    timestamp: libc::timeval,
    timecode: V4l2Timecode,
    sequence: u32,
    memory: u32,
    m: V4l2BufferM,
    length: u32,
    reserved2: u32,
    request_fd: u32, // union { i32 request_fd; u32 reserved } — 4 bytes
} // 88 bytes, align 8

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct V4l2Fract {
    numerator: u32,
    denominator: u32,
} // 8 bytes

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct V4l2Outputparm {
    capability: u32,
    outputmode: u32,
    timeperframe: V4l2Fract,
    extendedmode: u32,
    writebuffers: u32,
    reserved: [u32; 4],
} // 40 bytes

#[repr(C)]
union V4l2StreamparmUnion {
    output: V4l2Outputparm,
    raw_data: [u8; 200],
}

#[repr(C)]
struct V4l2Streamparm {
    typ: u32,
    parm: V4l2StreamparmUnion,
} // 204 bytes

// Compile-time checks for ABI-invariant structures. Architecture-dependent
// structures feed their actual size into ioctl_code above.
const _: () = assert!(std::mem::size_of::<V4l2Capability>() == 104);
const _: () = assert!(std::mem::size_of::<V4l2Requestbuffers>() == 20);
const _: () = assert!(std::mem::size_of::<V4l2Streamparm>() == 204);

/// A single mmap'd V4L2 buffer.
struct MmapBuffer {
    ptr: *mut u8,
    len: usize,
}

/// Low-level owner of one v4l2loopback mmap stream.
struct MmapWriter {
    file: std::fs::File,
    device_num: u32,
    width: u32,
    height: u32,
    frame_size: usize,
    buffers: Vec<MmapBuffer>,
    shutdown: ShutdownHandle,
}

// The mapping and descriptor have one owner and can safely move as a unit to
// another thread. Access remains exclusive through &mut self.
unsafe impl Send for MmapWriter {}

/// `ioctl` wrapper: returns Err with errno on failure (ret < 0).
/// SAFETY: `req` must be a valid VIDIOC* ioctl number for the passed struct type.
unsafe fn ioctl<T>(fd: c_int, req: c_ulong, arg: *mut T) -> std::io::Result<()> {
    // SAFETY: caller guarantees req matches the pointed-to struct layout.
    let ret = unsafe { libc::ioctl(fd, req as libc::c_ulong, arg) };
    if ret < 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// Zero a #[repr(C)] struct. Safe because all our V4L2 structs are POD.
fn zeroed<T>() -> T {
    unsafe { std::mem::zeroed() }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DequeueDecision {
    Retry,
    Shutdown,
    Fail,
}

fn dequeue_decision(raw_error: i32, shutdown: &ShutdownHandle) -> DequeueDecision {
    if shutdown.is_requested() {
        DequeueDecision::Shutdown
    } else if raw_error == libc::EFAULT || raw_error == libc::EAGAIN {
        DequeueDecision::Retry
    } else {
        DequeueDecision::Fail
    }
}

impl MmapWriter {
    /// Open a v4l2loopback device and set up the mmap streaming pipeline.
    ///
    /// `device_num` = video device number (e.g. 10 → /dev/video10).
    /// Must be pre-created with:
    ///   sudo modprobe v4l2loopback video_nr=10 exclusive_caps=1
    pub fn open(device_num: u32, width: u32, height: u32, fps: u32) -> anyhow::Result<Self> {
        let dev_path = format!("/dev/video{device_num}");
        // O_RDWR is required for mmap (O_WRONLY fails with EACCES on
        // MAP_SHARED). O_NONBLOCK guarantees VIDIOC_DQBUF cannot trap the
        // output worker inside the kernel while its owner requests shutdown.
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(&dev_path)
            .with_context(|| format!("Failed to open {dev_path}. Is v4l2loopback loaded?"))?;
        let fd = file.as_raw_fd();
        let frame_size = (width as usize) * (height as usize) * 3;

        // 1. QUERYCAP — verify it's a v4l2 loopback OUTPUT device with streaming.
        let mut cap: V4l2Capability = zeroed();
        unsafe { ioctl(fd, VIDIOC_QUERYCAP, &mut cap) }.context("VIDIOC_QUERYCAP failed")?;
        let capabilities = if cap.capabilities & V4L2_CAP_DEVICE_CAPS != 0 {
            cap.device_caps
        } else {
            cap.capabilities
        };
        ensure!(
            capabilities & V4L2_CAP_VIDEO_OUTPUT != 0,
            "device {dev_path} is not a video output device"
        );
        ensure!(
            capabilities & V4L2_CAP_STREAMING != 0,
            "device {dev_path} does not support streaming (mmap)"
        );

        // 2. S_FMT — negotiate RGB24 format. S_FMT MUST come before REQBUFS
        //    (v4l2loopback returns -EBUSY if buffers are already allocated).
        let mut fmt: V4l2Format = zeroed();
        fmt.typ = V4L2_BUF_TYPE_VIDEO_OUTPUT;
        // SAFETY: union field assignment stabilized in Rust 2024 (RFC 3647),
        // but explicit block kept for clarity — compiler says unnecessary.
        fmt.fmt.pix = V4l2PixFormat {
            width,
            height,
            pixelformat: V4L2_PIX_FMT_RGB24,
            field: V4L2_FIELD_NONE,
            ..Default::default()
        };
        unsafe { ioctl(fd, VIDIOC_S_FMT, &mut fmt) }.context("VIDIOC_S_FMT failed")?;
        let negotiated = unsafe { fmt.fmt.pix };
        ensure!(
            negotiated.width == width
                && negotiated.height == height
                && negotiated.pixelformat == V4L2_PIX_FMT_RGB24,
            "v4l2loopback negotiated {}x{} fourcc=0x{:08x}, expected {width}x{height} RGB24",
            negotiated.width,
            negotiated.height,
            negotiated.pixelformat
        );
        ensure!(
            negotiated.bytesperline == 0 || negotiated.bytesperline == width * 3,
            "padded V4L2 RGB stride {} is not supported (expected {})",
            negotiated.bytesperline,
            width * 3
        );

        // 3. S_PARM (optional) — set timeperframe for sustain_framerate pacing.
        //    pyvirtualcam skips this, but v4l2loopback honors it.
        let mut parm: V4l2Streamparm = zeroed();
        parm.typ = V4L2_BUF_TYPE_VIDEO_OUTPUT;
        let out = V4l2Outputparm {
            timeperframe: V4l2Fract {
                numerator: 1,
                denominator: fps.max(1),
            },
            ..Default::default()
        };
        parm.parm.output = out;
        // Ignore S_PARM errors — it's advisory, not all drivers care.
        let _ = unsafe { ioctl(fd, VIDIOC_S_PARM, &mut parm) };

        // 4. REQBUFS — request N mmap buffers. v4l2loopback clamps to
        //    max_buffers (default 2) and writes the real count back.
        let mut req: V4l2Requestbuffers = zeroed();
        req.count = 4; // request 4; driver will clamp (default 2)
        req.typ = V4L2_BUF_TYPE_VIDEO_OUTPUT;
        req.memory = V4L2_MEMORY_MMAP;
        unsafe { ioctl(fd, VIDIOC_REQBUFS, &mut req) }.context("VIDIOC_REQBUFS failed")?;
        ensure!(
            req.count >= 1,
            "v4l2loopback gave 0 buffers (raise max_buffers?)"
        );

        // 5. QUERYBUF + mmap for each buffer the driver actually allocated.
        let mut buffers: Vec<MmapBuffer> = Vec::with_capacity(req.count as usize);
        for i in 0..req.count {
            let mut buf: V4l2Buffer = zeroed();
            buf.typ = V4L2_BUF_TYPE_VIDEO_OUTPUT;
            buf.memory = V4L2_MEMORY_MMAP;
            buf.index = i;
            unsafe { ioctl(fd, VIDIOC_QUERYBUF, &mut buf) }
                .with_context(|| format!("VIDIOC_QUERYBUF[{i}] failed"))?;

            let len = buf.length as usize;
            ensure!(
                len >= frame_size,
                "V4L2 buffer {i} has {len} bytes, expected at least {frame_size}"
            );
            // SAFETY: buf.m.offset is the MMAP cookie returned by QUERYBUF.
            let m_offset = unsafe { buf.m.offset } as off_t;
            let ptr = unsafe {
                libc::mmap(
                    ptr::null_mut(),
                    len as size_t,
                    libc::PROT_READ | libc::PROT_WRITE,
                    libc::MAP_SHARED,
                    fd,
                    m_offset,
                )
            };
            if ptr == libc::MAP_FAILED {
                return Err(std::io::Error::last_os_error())
                    .with_context(|| format!("mmap[{i}] failed (offset={m_offset}, len={len})"));
            }
            unsafe { std::ptr::write_bytes(ptr.cast::<u8>(), 0, len) };
            buffers.push(MmapBuffer {
                ptr: ptr as *mut u8,
                len,
            });
        }

        // 6. Queue all buffers before STREAMON. bytesused = frame_size;
        //    producer is responsible for bytesused on OUTPUT.
        for (i, buffer) in buffers.iter().enumerate() {
            let mut buf: V4l2Buffer = zeroed();
            buf.typ = V4L2_BUF_TYPE_VIDEO_OUTPUT;
            buf.memory = V4L2_MEMORY_MMAP;
            buf.index = i as u32;
            buf.bytesused = frame_size as u32;
            buf.length = buffer.len as u32;
            unsafe { ioctl(fd, VIDIOC_QBUF, &mut buf) }
                .with_context(|| format!("initial VIDIOC_QBUF[{i}] failed"))?;
        }

        // 7. STREAMON — start streaming. Requires REQBUFS (buffer_count > 0).
        let mut type_field: u32 = V4L2_BUF_TYPE_VIDEO_OUTPUT;
        unsafe { ioctl(fd, VIDIOC_STREAMON, &mut type_field) }.context("VIDIOC_STREAMON failed")?;

        let nbuf = req.count;
        log::info!(
            "Opened virtual camera /dev/video{device_num}: {width}x{height}@{fps}fps \
             ({nbuf} mmap buffers, {frame_size} bytes/frame)"
        );

        Ok(Self {
            file,
            device_num,
            width,
            height,
            frame_size,
            buffers,
            shutdown: ShutdownHandle::default(),
        })
    }

    /// Send a single RGB24 frame to the virtual camera.
    ///
    /// `frame` must be [R, G, B, R, G, B, ...] with width*height*3 bytes.
    ///
    /// Blocking DQBUF: the producer naturally rate-matches the consumer.
    /// If no consumer is attached, v4l2loopback cycles queued buffers back
    /// immediately (no EFAULT in the steady state with ≥2 buffers).
    pub fn send_frame(&mut self, frame: &[u8]) -> anyhow::Result<()> {
        ensure!(
            frame.len() == self.frame_size,
            "frame size mismatch: {} bytes, expected {} ({}x{}x3)",
            frame.len(),
            self.frame_size,
            self.width,
            self.height
        );
        let fd = self.file.as_raw_fd();

        // A. DQBUF — dequeue a free buffer without blocking in the kernel.
        let mut buf: V4l2Buffer = zeroed();
        buf.typ = V4L2_BUF_TYPE_VIDEO_OUTPUT;
        buf.memory = V4L2_MEMORY_MMAP;

        // v4l2loopback quirk: DQBUF on OUTPUT can return EFAULT (not EAGAIN)
        // when outbufs_list is empty. Retry with a short backoff.
        loop {
            if self.shutdown.is_requested() {
                return Ok(());
            }
            let ret = unsafe { libc::ioctl(fd, VIDIOC_DQBUF, &mut buf) };
            if ret == 0 {
                break;
            }
            let err = std::io::Error::last_os_error();
            let raw = err.raw_os_error().unwrap_or(0);
            match dequeue_decision(raw, &self.shutdown) {
                DequeueDecision::Retry => {
                    // v4l2loopback reports POLLOUT while its OUTPUT stream is
                    // active even for this EFAULT quirk, so poll would spin.
                    std::thread::sleep(std::time::Duration::from_millis(1));
                    continue;
                }
                DequeueDecision::Shutdown => return Ok(()),
                DequeueDecision::Fail => {}
            }
            return Err(err).context("VIDIOC_DQBUF failed");
        }

        let idx = buf.index as usize;
        ensure!(
            idx < self.buffers.len(),
            "DQBUF returned out-of-range buffer index {idx}"
        );

        // B. memcpy the frame into the mmap'd region.
        unsafe {
            libc::memcpy(
                self.buffers[idx].ptr as *mut libc_c_void,
                frame.as_ptr() as *const libc_c_void,
                self.frame_size,
            );
        }

        // C. QBUF — requeue the filled buffer. Re-set bytesused/length
        //    (driver may have touched them).
        buf.bytesused = self.frame_size as u32;
        buf.length = self.buffers[idx].len as u32;
        unsafe { ioctl(fd, VIDIOC_QBUF, &mut buf) }.context("VIDIOC_QBUF (requeue) failed")?;

        Ok(())
    }

    /// Get the device path (e.g. /dev/video10).
    pub fn device_path(&self) -> String {
        format!("/dev/video{}", self.device_num)
    }

    fn shutdown_handle(&self) -> ShutdownHandle {
        self.shutdown.clone()
    }
}

impl Drop for MmapWriter {
    fn drop(&mut self) {
        let fd = self.file.as_raw_fd();

        // STREAMOFF — best-effort, don't panic in drop.
        let mut type_field: u32 = V4L2_BUF_TYPE_VIDEO_OUTPUT;
        unsafe {
            let _ = libc::ioctl(fd, VIDIOC_STREAMOFF, &mut type_field);
        }

        // munmap each buffer — best-effort.
        for b in &self.buffers {
            unsafe {
                let _ = libc::munmap(b.ptr as *mut libc_c_void, b.len);
            }
        }
        // self.file drops here → close(fd).
    }
}

/// Backend adapter around the mmap V4L2 writer.
pub struct V4l2Backend {
    writer: Option<MmapWriter>,
    device: String,
    width: u32,
    height: u32,
}

impl V4l2Backend {
    pub fn new(width: u32, height: u32, fps: f64, device: Option<&str>) -> Result<Self> {
        let fps = fps.round().clamp(1.0, u32::MAX as f64) as u32;
        let open = |device_num| {
            MmapWriter::open(device_num, width, height, fps)
                .map_err(|error| VirtualCamError::InitializationFailed(error.to_string()))
        };

        let writer = if let Some(path) = device {
            let number = parse_device_number(path)?;
            open(number)?
        } else {
            let mut found = None;
            for number in 0..100 {
                if !is_loopback_device(number) {
                    continue;
                }
                if let Ok(writer) = open(number) {
                    found = Some(writer);
                    break;
                }
            }
            found.ok_or(VirtualCamError::V4l2LoopbackNotFound)?
        };
        let device = writer.device_path();

        Ok(Self {
            writer: Some(writer),
            device,
            width,
            height,
        })
    }
}

fn parse_device_number(path: &str) -> Result<u32> {
    path.strip_prefix("/dev/video")
        .unwrap_or(path)
        .parse::<u32>()
        .map_err(|_| VirtualCamError::DeviceNotFound(path.to_owned()))
}

fn is_loopback_device(number: u32) -> bool {
    let Ok(file) = OpenOptions::new()
        .read(true)
        .write(true)
        .open(format!("/dev/video{number}"))
    else {
        return false;
    };
    let mut capability: V4l2Capability = zeroed();
    if unsafe { ioctl(file.as_raw_fd(), VIDIOC_QUERYCAP, &mut capability) }.is_err() {
        return false;
    }
    let end = capability
        .driver
        .iter()
        .position(|&byte| byte == 0)
        .unwrap_or(capability.driver.len());
    let driver = String::from_utf8_lossy(&capability.driver[..end]);
    driver.contains("v4l2 loopback") || driver.contains("v4l2loopback")
}

pub fn is_available() -> bool {
    (0..100).any(is_loopback_device)
}

impl Backend for V4l2Backend {
    fn name(&self) -> &'static str {
        "v4l2"
    }

    fn device(&self) -> &str {
        &self.device
    }

    fn native_format(&self) -> PixelFormat {
        PixelFormat::RGB
    }

    fn native_dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    fn send(&mut self, frame: &[u8]) -> Result<()> {
        self.writer
            .as_mut()
            .ok_or(VirtualCamError::AlreadyClosed)?
            .send_frame(frame)
            .map_err(|error| VirtualCamError::SendFailed(error.to_string()))
    }

    fn close(&mut self) -> Result<()> {
        self.writer.take();
        Ok(())
    }

    fn is_open(&self) -> bool {
        self.writer.is_some()
    }

    fn shutdown_handle(&self) -> Option<ShutdownHandle> {
        self.writer.as_ref().map(MmapWriter::shutdown_handle)
    }
}

#[cfg(all(test, target_pointer_width = "64"))]
mod tests {
    use super::*;

    #[test]
    fn dequeue_retry_contract_stops_after_shutdown_request() {
        let shutdown = ShutdownHandle::default();

        assert_eq!(
            dequeue_decision(libc::EAGAIN, &shutdown),
            DequeueDecision::Retry
        );
        assert_eq!(
            dequeue_decision(libc::EFAULT, &shutdown),
            DequeueDecision::Retry
        );
        assert_eq!(
            dequeue_decision(libc::EIO, &shutdown),
            DequeueDecision::Fail
        );

        shutdown.request();

        assert_eq!(
            dequeue_decision(libc::EAGAIN, &shutdown),
            DequeueDecision::Shutdown
        );
        assert_eq!(
            dequeue_decision(libc::EFAULT, &shutdown),
            DequeueDecision::Shutdown
        );
    }

    #[test]
    fn ioctl_codes_match_linux_v4l2_uapi() {
        assert_eq!(std::mem::size_of::<V4l2Format>(), 208);
        assert_eq!(std::mem::size_of::<V4l2Buffer>(), 88);
        assert_eq!(VIDIOC_QUERYCAP, 0x8068_5600);
        assert_eq!(VIDIOC_S_FMT, 0xc0d0_5605);
        assert_eq!(VIDIOC_REQBUFS, 0xc014_5608);
        assert_eq!(VIDIOC_QUERYBUF, 0xc058_5609);
        assert_eq!(VIDIOC_QBUF, 0xc058_560f);
        assert_eq!(VIDIOC_DQBUF, 0xc058_5611);
        assert_eq!(VIDIOC_STREAMON, 0x4004_5612);
        assert_eq!(VIDIOC_STREAMOFF, 0x4004_5613);
        assert_eq!(VIDIOC_S_PARM, 0xc0cc_5616);
    }
}
