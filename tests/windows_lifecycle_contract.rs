use std::fs;
use std::path::Path;

fn source(relative: &str) -> String {
    fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(relative))
        .unwrap_or_else(|error| panic!("cannot read {relative}: {error}"))
}

fn line_index(haystack: &str, needle: &str) -> usize {
    haystack
        .lines()
        .position(|line| line.contains(needle))
        .unwrap_or_else(|| panic!("missing contract line: {needle}"))
}

fn line_indices(haystack: &str, needle: &str) -> Vec<usize> {
    haystack
        .lines()
        .enumerate()
        .filter_map(|(index, line)| line.contains(needle).then_some(index))
        .collect()
}

#[test]
fn registrar_replace_only_removes_the_owned_media_foundation_camera() {
    let registrar = source("cpp/VCam/Registrar/Registrar.cpp");

    for forbidden in [
        "TerminateProcess",
        "CreateToolhelp32Snapshot",
        "OpenProcess(",
        "OpenSCManagerW",
        "OpenServiceW",
        "ControlService(",
        "FrameServerMonitor",
        "L\"FrameServer\"",
        "Sleep(INFINITE)",
    ] {
        assert!(
            !registrar.contains(forbidden),
            "registrar replacement must not use global process/service control: {forbidden}"
        );
    }

    let replace = registrar
        .split("\nHRESULT ReplaceOwnedVirtualCamera()\n{")
        .nth(1)
        .expect("registrar must have an identity-scoped replacement routine");
    let replace = replace
        .split("HRESULT RegisterVirtualCamera()")
        .next()
        .expect("replacement routine must precede normal registration");
    assert!(replace.contains("MFVirtualCameraAccess_CurrentUser"));
    assert!(replace.contains("vcam::contract::SourceClsid"));
    assert!(replace.contains("Remove()"));
    assert!(replace.contains("HRESULT_FROM_WIN32(ERROR_NOT_FOUND)"));
    assert!(replace.contains("MF_E_NOT_FOUND"));
    assert!(registrar.contains("return SUCCEEDED(exitResult) ? ERROR_SUCCESS"));

    let close_ready = registrar
        .find("CloseHandle(_readyEvent)")
        .expect("ready handle must be closed during teardown");
    let release_owner = registrar
        .rfind("ReleaseRegistrarOwnership();")
        .expect("registrar ownership must be released during teardown");
    assert!(
        close_ready < release_owner,
        "the ownership mutex must cover the complete ready/mapping teardown"
    );
}

#[test]
fn installer_secures_the_runtime_root_before_copying_any_executable() {
    let payload = source("scripts/install-runtime.ps1");
    let lower = payload.to_ascii_lowercase();

    assert!(lower.contains("programfilesdir"));
    assert!(lower.contains("join-path $programfiles 'noperson'"));
    assert!(lower.contains("s-1-5-18"));
    assert!(lower.contains("s-1-5-32-544"));
    assert!(lower.contains("filesystemrights]::readandexecute"));
    assert!(!lower.contains("filesystemrights]::modify"));
    assert!(lower.contains("setaccessruleprotection($true, $false)"));
    assert!(lower.contains("[io.fileattributes]::reparsepoint"));
    assert!(lower.contains("[io.directory]::enumeratefilesystementries($root)"));
    assert!(!lower.contains("-recurse"));

    let read_verified = lower
        .split("function read-verifiedartifact")
        .nth(1)
        .expect("artifact read-once verifier must exist")
        .split("function ")
        .next()
        .expect("artifact verifier must be bounded");
    assert_eq!(read_verified.matches("[io.file]::readallbytes").count(), 1);
    assert!(
        line_index(read_verified, "[io.file]::readallbytes") < line_index(read_verified, "sha256")
    );

    let install = lower
        .split("function install-runtime")
        .nth(1)
        .expect("install transaction must exist");
    assert!(install.contains("[io.file]::writeallbytes"));
    assert!(install.contains(".partial"));
    assert!(install.contains("[io.file]::move"));
    assert!(install.contains("[io.file]::replace"));
    assert!(!install.contains("[io.file]::delete($destination)"));
    assert!(!install.contains("copy-item"));
    assert!(
        line_index(install, "protect-runtimetree")
            < line_index(install, "start-process -filepath $registrarpath")
    );
    assert!(
        !install.contains("$registrarpath /remove"),
        "payload must never execute a pre-existing runtime helper"
    );
}

#[test]
fn installer_keeps_build_unelevated_and_uses_read_once_encoded_payload() {
    let install = source("install.bat").to_ascii_lowercase();
    assert!(!install.contains("--install-only"));
    assert!(!install.contains("%~f0"));
    assert!(install.contains("cargo build --release"));
    assert!(install.contains("bootstrap-nuget.bat"));
    assert!(install.contains("%msbuild%"));
    assert!(install.contains("refusing to build from an elevated shell"));
    assert!(install.contains("scripts\\install-runtime.ps1"));
    assert!(install.contains("[io.file]::readallbytes($env:install_payload)"));
    assert_eq!(
        install
            .matches("[io.file]::readallbytes($env:install_payload)")
            .count(),
        1
    );
    assert_eq!(
        install
            .matches("[io.file]::readallbytes([string]$context.payloadpath)")
            .count(),
        1
    );
    assert!(install.contains("[scriptblock]::create"));
    assert!(install.contains("-encodedcommand"));
    assert!(install.contains("-verb runas"));
    assert!(install.contains("get-filehash -algorithm sha256"));
}

#[test]
fn install_and_uninstall_never_kill_by_basename_or_control_frame_server() {
    for script in ["install.bat", "uninstall.bat"] {
        let body = source(script);
        for forbidden in [
            "taskkill",
            "TerminateProcess",
            "net stop FrameServer",
            "sc stop FrameServer",
            "net start FrameServer",
            "sc start FrameServer",
        ] {
            assert!(
                !body.contains(forbidden),
                "{script} must not use basename-wide termination or service control: {forbidden}"
            );
        }
    }

    let uninstall = source("uninstall.bat");
    assert!(uninstall.contains("set \"OUTDIR=%ProgramFiles%\\noperson\""));
    assert!(uninstall.contains("ProgramFilesDir"));
    assert!(uninstall.contains("VCamRegistrar.exe\" /remove"));
    assert!(uninstall.contains("FSUTIL_EXE hardlink list"));
    assert!(uninstall.contains("Get-Acl"));
    assert!(uninstall.contains("[IO.FileAttributes]::ReparsePoint"));
    assert!(!uninstall.contains("Collections.Generic.Stack"));
    assert!(
        line_index(&uninstall, "call :verify_runtime_tree")
            < line_index(&uninstall, "VCamRegistrar.exe\" /remove")
    );
    assert!(
        line_index(&uninstall, "call :verify_runtime_execution_files")
            < line_index(&uninstall, "VCamRegistrar.exe\" /remove")
    );
    assert!(
        line_index(&uninstall, "VCamRegistrar.exe\" /remove")
            < line_index(&uninstall, "\"%REGSVR32_EXE%\" /u /s")
    );
}

#[test]
fn registrar_creates_shared_frames_with_an_exact_token_user_acl() {
    let registrar = source("cpp/VCam/Registrar/Registrar.cpp");

    for required in [
        "OpenProcessToken",
        "GetTokenInformation",
        "TokenUser",
        "ConvertSidToStringSidW",
        "ConvertStringSecurityDescriptorToSecurityDescriptorW",
        "D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GA;;;",
        ")(A;;GR;;;LS)",
        "CreateFileMappingW",
        "SetKernelObjectSecurity",
        "MapViewOfFile",
        "LocalFree",
    ] {
        assert!(
            registrar.contains(required),
            "the elevated registrar mapping contract is missing: {required}"
        );
    }
    for forbidden in [
        "SetSecurityDescriptorDacl",
        ";;;OW)",
        ";;;WD)",
        ";;;AU)",
        ";;;BU)",
    ] {
        assert!(
            !registrar.contains(forbidden),
            "the shared mapping must not grant implicit/broad access: {forbidden}"
        );
    }
}

#[test]
fn registrar_fails_closed_when_the_global_mapping_already_exists() {
    let registrar = source("cpp/VCam/Registrar/Registrar.cpp");
    let mapping = registrar
        .split("\nHRESULT CreateSharedFrameMapping(const RequestedFormat& format)\n{")
        .nth(1)
        .expect("secure mapping routine must exist")
        .split("HRESULT SignalStartupAck")
        .next()
        .expect("mapping routine must precede startup acknowledgment");

    let create = line_index(mapping, "mapping = CreateFileMappingW(");
    let collision = line_index(mapping, "mappingCreateError == ERROR_ALREADY_EXISTS");
    let secure = line_index(mapping, "SetKernelObjectSecurity(");
    let map_view = line_index(mapping, "view = MapViewOfFile(");
    assert!(create < collision && collision < secure && secure < map_view);
    assert!(mapping.contains("HRESULT_FROM_WIN32(ERROR_ALREADY_EXISTS)"));
}

#[test]
fn rust_waits_for_generation_ack_before_opening_the_registrar_mapping() {
    let backend = source("src/backend/windows_media_foundation.rs");
    assert!(
        !backend.contains("CreateFileMappingW"),
        "unelevated Rust must never create the Global file mapping"
    );
    assert!(backend.contains("OpenFileMappingW(FILE_MAP_ALL_ACCESS.0"));

    let constructor = backend
        .split("impl MediaFoundationBackend {")
        .nth(1)
        .expect("MediaFoundationBackend implementation must exist")
        .split("impl Backend for MediaFoundationBackend")
        .next()
        .expect("MediaFoundationBackend constructor must precede its Backend implementation");
    let lease = line_index(constructor, "WriterLease::acquire()");
    let helper = line_index(constructor, "ensure_registrar_format(format)");
    let writer = line_index(constructor, "FrameWriter::open(format, writer_lease)");
    assert!(lease < helper && helper < writer);

    let ensure = backend
        .split("fn ensure_registrar_format(format: SharedFormat)")
        .nth(1)
        .expect("format helper must exist")
        .split("struct FrameWriter")
        .next()
        .expect("format helper must precede FrameWriter");
    assert!(ensure.contains("registrar_replace_arguments(format, startup_ack.name())"));
    assert!(ensure.contains("startup_ack.wait("));
    let published_format_checks = line_indices(ensure, "current_shared_format() == Some(format)");
    assert!(published_format_checks.len() >= 2);
    assert!(
        line_index(ensure, "startup_ack.wait(")
            < *published_format_checks
                .last()
                .expect("post-ack format check")
    );
}

#[test]
fn registrar_signals_the_exact_generation_only_after_mapping_and_camera_start() {
    let registrar = source("cpp/VCam/Registrar/Registrar.cpp");
    assert!(registrar.contains("L\"/ack\""));
    assert!(registrar.contains("OpenEventW(EVENT_MODIFY_STATE"));
    assert!(registrar.contains("SetEvent(startupAck)"));

    let startup = registrar
        .split("auto hr = AcquireRegistrarOwnership")
        .nth(1)
        .expect("registrar startup sequence must exist")
        .split("exitResult = hr")
        .next()
        .expect("startup sequence must set its exit result");
    let create_mapping = line_index(startup, "CreateSharedFrameMapping(requestedFormat)");
    let register_camera = line_index(startup, "RegisterVirtualCamera()");
    let signal_ack = line_index(startup, "SignalStartupAck(startupAckName)");
    assert!(create_mapping < register_camera && register_camera < signal_ack);
}
