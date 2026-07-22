# Platform operations

## Windows 11 VCam installation

Run `install.bat` from an elevated Command Prompt. It performs the complete
cycle: Rust build, MSBuild, runtime copy, COM registration, registrar startup,
test producer startup, and Media Foundation capture verification.

Before MSBuild runs, `scripts\bootstrap-nuget.bat` downloads the pinned WIL and
C++/WinRT `.nupkg` files directly. It uses the NuGet v2 download endpoint first
and the V3 flat-container endpoint as a fallback because the latter is not
reachable from every network. It verifies SHA-256 hashes and atomically
populates `cpp\VCam\packages`; no MSBuild or NuGet restore target participates
in this step.

The runtime directory is `C:\vcam`. Frame Server must be able to read
`VCamSource.dll`; do not move or delete the DLL while it is registered.

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
C:\vcam\vcam-verify.exe
```

`vcam-verify.exe` enumerates cameras, opens the device whose friendly name
starts with `VCam` (Windows may append a localized virtual-camera suffix),
captures through `IMFSourceReader`, and validates dimensions, format, and
deterministic test-frame bytes.

## Common failures

### Camera is not listed

- Confirm `VCamRegistrar.exe` is running.
- Confirm the CLSID registry path points to `C:\vcam\VCamSource.dll`.
- Restart the client; many applications cache camera enumeration.

### `regsvr32` fails

- Run the terminal as administrator.
- Stop `FrameServerMonitor` and `FrameServer` before replacing the DLL.
- Confirm the DLL is x64 when registering it with the system `regsvr32`.

### Camera opens but has no injected frame

- Confirm the producer owns `Global\vcam_frames`.
- Check `pump.err` and `pump.log`.
- Confirm the producer uses NV12 1280×960 and a 1,843,200-byte payload.
- Run `vcam-pump.exe` and then `vcam-verify.exe` to isolate the application.

### Green test fails byte validation

An older producer or DLL is probably still running. Stop the pump, registrar,
Frame Server Monitor, and Frame Server; replace both native binaries; register
the new DLL; then rerun the installer.

## Upgrade and removal

Before upgrading, stop the registrar and Frame Server services so the COM DLL
is not locked. The installer does this automatically.

`uninstall.bat` stops VCam processes and unregisters the COM class. It leaves
runtime files in place so logs and binaries remain available for diagnosis.

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
