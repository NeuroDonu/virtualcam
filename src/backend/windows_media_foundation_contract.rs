//! Platform-neutral pieces of the Windows shared-frame IPC contract.
//!
//! Kept free of Win32 types so the security, lease and seqlock decisions are
//! exercised by the Linux test suite as well as consumed by the Windows code.

pub const SHM_NAME: &str = "Global\\vcam_frames";
pub const WRITER_MUTEX_NAME: &str = "Global\\vcam_frames_writer";
pub const SHM_MAGIC: u32 = 0x4d41_4356;
pub const SHM_VERSION: u32 = 2;
pub const HEADER_SIZE: usize = 12 * size_of::<u32>();
pub const MAX_WIDTH: u32 = 4096;
pub const MAX_HEIGHT: u32 = 2160;
pub const MAX_FRAME_SIZE: usize = MAX_WIDTH as usize * MAX_HEIGHT as usize * 3 / 2;

pub const WAIT_OBJECT_0_RAW: u32 = 0;
pub const WAIT_ABANDONED_RAW: u32 = 0x80;
pub const WAIT_TIMEOUT_RAW: u32 = 0x102;
pub const WAIT_FAILED_RAW: u32 = u32::MAX;

pub fn startup_ack_name(process_id: u32, generation: u64) -> String {
    format!("Local\\noperson_vcam_startup_{process_id:08x}_{generation:016x}")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IpcTrustee {
    System,
    Administrators,
    OwnerRights,
    LocalService,
}

impl IpcTrustee {
    const fn sddl_alias(self) -> &'static str {
        match self {
            Self::System => "SY",
            Self::Administrators => "BA",
            Self::OwnerRights => "OW",
            Self::LocalService => "LS",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessMask {
    Full,
    Read,
}

impl AccessMask {
    const fn sddl_alias(self) -> &'static str {
        match self {
            Self::Full => "GA",
            Self::Read => "GR",
        }
    }
}

const IPC_ACL_ENTRIES: [(IpcTrustee, AccessMask); 4] = [
    (IpcTrustee::System, AccessMask::Full),
    (IpcTrustee::Administrators, AccessMask::Full),
    (IpcTrustee::OwnerRights, AccessMask::Full),
    (IpcTrustee::LocalService, AccessMask::Read),
];

pub fn ipc_acl_entries() -> &'static [(IpcTrustee, AccessMask)] {
    &IPC_ACL_ENTRIES
}

pub fn ipc_security_sddl() -> String {
    let mut sddl = String::from("D:P");
    for &(trustee, access) in ipc_acl_entries() {
        sddl.push_str("(A;;");
        sddl.push_str(access.sddl_alias());
        sddl.push_str(";;;");
        sddl.push_str(trustee.sddl_alias());
        sddl.push(')');
    }
    sddl
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriterLeaseDisposition {
    Acquired,
    Busy,
    Failed(u32),
}

pub const fn classify_writer_wait(wait_result: u32) -> WriterLeaseDisposition {
    match wait_result {
        WAIT_OBJECT_0_RAW | WAIT_ABANDONED_RAW => WriterLeaseDisposition::Acquired,
        WAIT_TIMEOUT_RAW => WriterLeaseDisposition::Busy,
        WAIT_FAILED_RAW => WriterLeaseDisposition::Failed(WAIT_FAILED_RAW),
        other => WriterLeaseDisposition::Failed(other),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawHeaderSnapshot {
    pub magic: u32,
    pub version: u32,
    pub header_bytes: u32,
    pub width: u32,
    pub height: u32,
    pub fps_num: u32,
    pub fps_den: u32,
    pub frame_bytes: u32,
    pub seq: u32,
    pub ready: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ValidatedHeaderSnapshot {
    pub width: u32,
    pub height: u32,
    pub fps_num: u32,
    pub fps_den: u32,
    pub frame_bytes: u32,
    pub seq: u32,
    pub ready: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeaderReadError {
    WriterActive,
    SequenceChanged,
    InvalidLayout,
}

pub fn validate_stable_header(
    seq_before: u32,
    snapshot: RawHeaderSnapshot,
    seq_after: u32,
) -> Result<ValidatedHeaderSnapshot, HeaderReadError> {
    if seq_before & 1 != 0 {
        return Err(HeaderReadError::WriterActive);
    }
    if snapshot.seq != seq_before || seq_after != seq_before {
        return Err(HeaderReadError::SequenceChanged);
    }
    if snapshot.magic != SHM_MAGIC
        || snapshot.version != SHM_VERSION
        || snapshot.header_bytes != HEADER_SIZE as u32
        || snapshot.width == 0
        || snapshot.height == 0
        || snapshot.width & 1 != 0
        || snapshot.height & 1 != 0
        || snapshot.width > MAX_WIDTH
        || snapshot.height > MAX_HEIGHT
        || snapshot.fps_num == 0
        || snapshot.fps_den == 0
        || snapshot.ready > 1
    {
        return Err(HeaderReadError::InvalidLayout);
    }

    let expected_frame_bytes = u64::from(snapshot.width)
        .checked_mul(u64::from(snapshot.height))
        .and_then(|pixels| pixels.checked_mul(3))
        .map(|bytes| bytes / 2)
        .ok_or(HeaderReadError::InvalidLayout)?;
    if expected_frame_bytes != u64::from(snapshot.frame_bytes)
        || expected_frame_bytes > MAX_FRAME_SIZE as u64
    {
        return Err(HeaderReadError::InvalidLayout);
    }

    Ok(ValidatedHeaderSnapshot {
        width: snapshot.width,
        height: snapshot.height,
        fps_num: snapshot.fps_num,
        fps_den: snapshot.fps_den,
        frame_bytes: snapshot.frame_bytes,
        seq: snapshot.seq,
        ready: snapshot.ready != 0,
    })
}
