# Platform operations

## Windows 11 VCam installation

Run `install.bat` from a normal, unelevated Command Prompt. It refuses to build
when already elevated. Rust, NuGet bootstrap, and MSBuild therefore run only as
the interactive user. After the build, the script records SHA-256 hashes for
the four artifacts and for `scripts\install-runtime.ps1`, then requests one UAC
prompt for a fixed encoded PowerShell bootstrap.

The elevated bootstrap reads the payload bytes exactly once, verifies the hash
captured before UAC, and creates a `ScriptBlock` from those same bytes. The
payload likewise reads every artifact once, verifies its captured hash, and
writes those exact in-memory bytes to protected `.partial` files before atomic
rename. It never re-enters `install.bat`, runs build tools, or executes a
pre-existing runtime helper while elevated.

Before MSBuild runs, `scripts\bootstrap-nuget.bat` downloads the pinned WIL and
C++/WinRT `.nupkg` files directly. It uses the NuGet v2 download endpoint first
and the V3 flat-container endpoint as a fallback because the latter is not
reachable from every network. It verifies SHA-256 hashes and atomically
populates `cpp\VCam\packages`; no MSBuild or NuGet restore target participates
in this step.

The runtime directory is `%ProgramFiles%\noperson`. It is independent of the
checkout drive and is the only directory from which the elevated registrar may
be launched. Frame Server must be able to read `VCamSource.dll`; do not move or
delete the DLL while it is registered.

The runtime is deliberately flat: nested directories and reparse points are
rejected. An existing root must already have a protected, non-user-writable
DACL before it is touched. SYSTEM and Administrators receive full control; the
interactive user receives read/execute only. Each final file is created from
verified bytes inside this protected root and receives an explicit protected
DACL before the new registrar is launched. Any ownership, ACL, reparse, hash,
write, or activation failure aborts the install.

## Manual build

```bat
cargo build --release --no-default-features --features media-foundation --bin vcam-pump --bin vcam-verify
scripts\bootstrap-nuget.bat
msbuild cpp\VCam\VCam.sln /p:Configuration=Release /p:Platform=x64 /m
```

Expected native outputs:

```text
cpp\VCam\x64\Release\VCamSource.dll
cpp\VCam\x64\Release\VCamRegistrar.exe
```

## Health checks

```bat
tasklist /FI "IMAGENAME eq VCamRegistrar.exe"
reg query "HKLM\SOFTWARE\Classes\CLSID\{3cad447d-f283-4af4-a3b2-6f5363309f52}\InprocServer32"
"%ProgramFiles%\noperson\vcam-verify.exe"
```

`vcam-verify.exe` enumerates cameras, opens the device whose friendly name
starts with `VCam` (Windows may append a localized virtual-camera suffix),
captures through `IMFSourceReader`, and validates dimensions, format, and
deterministic test-frame bytes.

## Common failures

### Camera is not listed

- Confirm `VCamRegistrar.exe` is running.
- Confirm the CLSID registry path points to
  `%ProgramFiles%\noperson\VCamSource.dll`.
- Restart the client; many applications cache camera enumeration.

### `regsvr32` fails

- For manual `regsvr32` use, run the terminal as administrator. The normal
  `install.bat` entry point must remain unelevated.
- Close applications using VCam and retry the installer.
- If Windows still retains the previous camera session or DLL, restart Windows
  and rerun the installer. The scripts never stop global camera services.
- Confirm the DLL is x64 when registering it with the system `regsvr32`.

### Camera opens but has no injected frame

- Confirm the producer owns `Global\vcam_frames`.
- Check `pump.err` and `pump.log`.
- Confirm the producer and negotiated MF type report the same even dimensions
  and FPS. NV12 payload size must be exactly `width * height * 3 / 2`.
- Run `vcam-pump.exe` and then `vcam-verify.exe` to isolate the application.

### Green test fails byte validation

An older producer, client, or DLL is probably still active. Close applications
using VCam and rerun the installer. Restart Windows if the old Frame Server
session remains cached; the installer does not terminate processes by basename
or control system camera services.

## Upgrade and removal

Before upgrading, close camera clients so the COM DLL can be released. The
installer asks the project-owned registrar to remove the exact CurrentUser
virtual-camera identity. It fails with restart guidance if Windows still holds
the old camera session or DLL. The installer never executes a helper already
present in the runtime directory: it first copies and verifies newly built
artifacts, then launches that protected registrar with `/replace`.

`uninstall.bat` derives Program Files from HKLM, verifies the root and exact
helper/DLL paths are not reparse points, checks owner and protected DACLs, and
rejects a hard-linked registrar. Only then does it ask the trusted installed
registrar to remove the owned registration and unregister the COM class. It
leaves runtime files in place for diagnosis.

## Mode replacement and signing

The Rust backend compares the requested mode with the live shared-memory
contract. If width, height, or FPS changed, it launches the installed
`VCamRegistrar.exe` through the Windows `runas` verb with
`/headless /replace width height fps_numerator fps_denominator`. The helper
cooperatively releases the previous noperson registrar through a product-owned
event and mutex, reopens the virtual camera by its exact source CLSID, Session
lifetime, and CurrentUser access, and calls `IMFVirtualCamera::Remove` before
publishing the requested contract. It never enumerates or terminates processes
and never stops or restarts Frame Server services. Applications which had the
old camera open must reopen it after replacement; restart Windows only if the
camera session remains cached. Matching modes do not prompt for UAC.

For distribution, Authenticode-sign `VCamRegistrar.exe`, `VCamSource.dll`, and
the application after the final Release build and before staging. With a code
signing certificate available to the release machine, use the Windows SDK
`signtool` with SHA-256 file and RFC 3161 timestamp digests, then verify every
artifact before packaging:

```bat
signtool sign /fd SHA256 /td SHA256 /tr <RFC3161_TIMESTAMP_URL> /a VCamRegistrar.exe
signtool sign /fd SHA256 /td SHA256 /tr <RFC3161_TIMESTAMP_URL> /a VCamSource.dll
signtool verify /pa /all /v VCamRegistrar.exe
signtool verify /pa /all /v VCamSource.dll
```

Use an organization-validated or extended-validation certificate for public
releases. Signing does not replace the protected runtime-directory ACL.

## Windows 10/11 OBS

The OBS backend requires the `OBS Virtual Camera` DirectShow filter installed
by OBS Studio. It does not require the OBS application to be running, but OBS
must not already own the `OBSVirtualCamVideo` queue.

Use `BackendKind::Obs` to require this route. With `BackendKind::Auto`, Windows
10 selects OBS because the Media Foundation virtual-camera API is unavailable.

## Linux v4l2loopback

Create a device with output/capture capability switching enabled:

```bash
sudo modprobe v4l2loopback video_nr=10 exclusive_caps=1 max_buffers=4
```

Grant the producer access through the distribution's `video` group or device
rules. Useful diagnostics:

```bash
v4l2-ctl --device /dev/video10 --all
ls -l /dev/video10
```

If automatic discovery does not select the intended loopback device, pass its
path through `CameraBuilder::device`.
