#[allow(dead_code)]
#[path = "../src/backend/windows_media_foundation_contract.rs"]
mod contract;

#[cfg(not(windows))]
use std::process::Command;
use std::{fs, path::Path};

use contract::{
    AccessMask, HeaderReadError, IpcTrustee, RawHeaderSnapshot, WriterLeaseDisposition,
    classify_writer_wait, ipc_acl_entries, ipc_security_sddl, startup_ack_name,
    validate_stable_header,
};

fn valid_header() -> RawHeaderSnapshot {
    RawHeaderSnapshot {
        magic: contract::SHM_MAGIC,
        version: contract::SHM_VERSION,
        header_bytes: contract::HEADER_SIZE as u32,
        width: 1920,
        height: 1080,
        fps_num: 60,
        fps_den: 1,
        frame_bytes: 1920 * 1080 * 3 / 2,
        seq: 42,
        ready: 1,
    }
}

#[test]
fn writer_mutex_acl_grants_only_owner_system_admin_and_local_service() {
    assert_eq!(
        ipc_acl_entries(),
        &[
            (IpcTrustee::System, AccessMask::Full),
            (IpcTrustee::Administrators, AccessMask::Full),
            (IpcTrustee::OwnerRights, AccessMask::Full),
            (IpcTrustee::LocalService, AccessMask::Read),
        ]
    );
    assert_eq!(
        ipc_security_sddl(),
        "D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GA;;;OW)(A;;GR;;;LS)"
    );
}

#[test]
fn writer_lease_rejects_a_live_second_producer_and_recovers_abandonment() {
    assert_eq!(
        classify_writer_wait(contract::WAIT_OBJECT_0_RAW),
        WriterLeaseDisposition::Acquired
    );
    assert_eq!(
        classify_writer_wait(contract::WAIT_ABANDONED_RAW),
        WriterLeaseDisposition::Acquired
    );
    assert_eq!(
        classify_writer_wait(contract::WAIT_TIMEOUT_RAW),
        WriterLeaseDisposition::Busy
    );
    assert_eq!(
        classify_writer_wait(contract::WAIT_FAILED_RAW),
        WriterLeaseDisposition::Failed(contract::WAIT_FAILED_RAW)
    );
}

#[test]
fn startup_ack_name_is_generation_specific_and_session_local() {
    assert_eq!(
        startup_ack_name(0x1234, 7),
        "Local\\noperson_vcam_startup_00001234_0000000000000007"
    );
    assert_ne!(startup_ack_name(0x1234, 7), startup_ack_name(0x1234, 8));
}

#[test]
fn stable_header_validation_rejects_torn_or_oversized_snapshots() {
    let valid = valid_header();
    assert!(validate_stable_header(42, valid, 42).is_ok());

    assert_eq!(
        validate_stable_header(42, valid, 44),
        Err(HeaderReadError::SequenceChanged)
    );

    let mut oversized = valid;
    oversized.width = contract::MAX_WIDTH + 2;
    oversized.frame_bytes = u32::MAX;
    assert_eq!(
        validate_stable_header(42, oversized, 42),
        Err(HeaderReadError::InvalidLayout)
    );
}

#[test]
fn validated_header_is_an_immutable_copy_of_the_observed_layout() {
    let mut shared = valid_header();
    let validated = validate_stable_header(shared.seq, shared, shared.seq).unwrap();

    shared.width = 4096;
    shared.height = 2160;
    shared.frame_bytes = contract::MAX_FRAME_SIZE as u32;

    assert_eq!(shared.width, 4096);
    assert_eq!(shared.height, 2160);
    assert_eq!(shared.frame_bytes, contract::MAX_FRAME_SIZE as u32);
    assert_eq!(validated.width, 1920);
    assert_eq!(validated.height, 1080);
    assert_eq!(validated.frame_bytes, 1920 * 1080 * 3 / 2);
}

#[cfg(not(windows))]
#[test]
fn native_reader_contract_compiles_and_runs_without_win32() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let source = root.join("tests/windows_frame_contract.cpp");
    let binary = std::env::temp_dir().join(format!(
        "virtualcam-windows-frame-contract-{}",
        std::process::id()
    ));

    let compile = Command::new("c++")
        .args(["-std=c++20", "-Wall", "-Wextra", "-Werror"])
        .arg(&source)
        .arg("-o")
        .arg(&binary)
        .output()
        .expect("launch the host C++ compiler");
    assert!(
        compile.status.success(),
        "C++ contract compilation failed:\n{}",
        String::from_utf8_lossy(&compile.stderr)
    );

    let run = Command::new(&binary)
        .output()
        .expect("run the C++ contract test");
    let _ = std::fs::remove_file(&binary);
    assert!(
        run.status.success(),
        "C++ contract test failed:\n{}",
        String::from_utf8_lossy(&run.stderr)
    );
}

#[test]
fn registrar_path_is_pinned_to_canonical_64_bit_hklm_program_files() {
    let backend = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src/backend/windows_media_foundation.rs"),
    )
    .expect("read Windows Media Foundation backend");

    for required in [
        "HKEY_LOCAL_MACHINE",
        "KEY_READ | KEY_WOW64_64KEY",
        "SOFTWARE\\\\Classes\\\\CLSID",
        "SOFTWARE\\\\Microsoft\\\\Windows\\\\CurrentVersion",
        "ProgramFilesDir",
        "VCamSource.dll",
        "VCamRegistrar.exe",
        "std::fs::canonicalize",
        "std::fs::symlink_metadata",
        "FILE_ATTRIBUTE_REPARSE_POINT",
        "CompareStringOrdinal",
        "canonical_paths_equal_case_insensitive",
        ".is_file()",
    ] {
        assert!(
            backend.contains(required),
            "trusted registrar path contract is missing: {required}"
        );
    }
    for forbidden in ["HKEY_CLASSES_ROOT", "HKEY_CURRENT_USER"] {
        assert!(
            !backend.contains(forbidden),
            "merged/per-user registry lookup is forbidden: {forbidden}"
        );
    }
}

#[test]
fn validated_registrar_path_stays_shell_compatible() {
    let backend = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src/backend/windows_media_foundation.rs"),
    )
    .expect("read Windows Media Foundation backend");
    let lookup = backend
        .split("fn registered_registrar_path()")
        .nth(1)
        .expect("trusted registrar lookup must exist")
        .split("fn ensure_registrar_format")
        .next()
        .expect("trusted lookup must precede helper launch");

    assert!(lookup.contains("Ok(expected_registrar)"));
    assert!(!lookup.contains("Ok(canonical_registrar)"));
}
