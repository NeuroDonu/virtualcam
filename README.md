# virtualcam

Cross-platform Rust output for virtual cameras. One API selects the best
available backend and accepts RGB, BGR, RGBA, grayscale, I420, NV12, YUYV, or
UYVY frames.

| Platform | Backend | Native transport |
|---|---|---|
| Windows 11 | `media-foundation` | Bundled VCam Media Foundation source |
| Windows 10/11 | `obs` | OBS Virtual Camera shared-memory queue |
| Linux | `v4l2` | `v4l2loopback` mmap streaming |

## Usage

```toml
[dependencies]
virtualcam = "1"
```

Automatic backend selection:

```rust,no_run
use virtualcam::{Camera, PixelFormat};

let mut camera = Camera::builder(1280, 720, 60.0)
    .format(PixelFormat::RGB)
    .build()?;

let rgb = vec![0_u8; 1280 * 720 * 3];
camera.send(&rgb)?;
# Ok::<(), virtualcam::VirtualCamError>(())
```

The automatic order is deterministic:

1. On Windows 11, use the bundled Media Foundation backend when VCam is
   registered.
2. On Windows 10 or when VCam is unavailable, use OBS Virtual Camera.
3. On Linux, find a `v4l2loopback` output device.

Select a backend or device explicitly when an application needs stable routing:

```rust,no_run
use virtualcam::{BackendKind, Camera, PixelFormat};

let mut camera = Camera::builder(1920, 1080, 30.0)
    .format(PixelFormat::BGR)
    .backend(BackendKind::Obs)
    .build()?;

// Linux example:
let linux_camera = Camera::builder(1280, 720, 60.0)
    .format(PixelFormat::RGB)
    .backend(BackendKind::V4l2)
    .device("/dev/video10")
    .build()?;
# Ok::<(), virtualcam::VirtualCamError>(())
```

`available_backends()` reports what is usable on the current machine. Explicit
selection never silently changes backend; `BackendKind::Auto` is the only mode
that falls back.

## Native and GPU frames

`Camera::send` validates, converts, and scales application frames as needed.
High-performance producers can skip conversion with `send_native`:

```rust,no_run
# use virtualcam::{BackendKind, Camera, PixelFormat};
let mut camera = Camera::builder(1280, 960, 30.0)
    .format(PixelFormat::NV12)
    .backend(BackendKind::MediaFoundation)
    .build()?;
let nv12 = vec![0_u8; camera.native_format().frame_size(
    camera.native_dimensions().0,
    camera.native_dimensions().1,
)];
camera.send_native(&nv12)?;
# Ok::<(), virtualcam::VirtualCamError>(())
```

Enable `gpu` to publish a device-resident native frame through reusable pinned
host memory:

```toml
virtualcam = { version = "1", features = ["gpu"] }
```

```rust,ignore
camera.send_native_gpu(&cuda_stream, &device_nv12)?;
```

The crate deliberately does not prescribe a CUDA image layout or processing
pipeline. GPU applications produce the backend's native frame and query its
format and dimensions through `native_format()` and `native_dimensions()`.

## Platform setup

### Windows 11 Media Foundation

From an elevated Visual Studio Build Tools command prompt:

```bat
install.bat
```

The installer downloads pinned C++/WinRT and WIL packages with verified
SHA-256 hashes, builds the native source, registers `VCamSource.dll`, and runs a
real capture check. See [operations](docs/OPERATIONS.md).

### Windows 10/11 OBS

Install OBS Studio with its Virtual Camera component. The `obs` backend writes
the queue consumed by the registered `OBS Virtual Camera` DirectShow filter;
OBS itself does not need to own the queue at the same time.

### Linux V4L2

`virtualcam` does not install, load, or register kernel devices. A system
administrator must create a loopback output device before the application
opens the camera:

```bash
sudo modprobe v4l2loopback video_nr=10 exclusive_caps=1
```

This is a one-time system setup (or a boot-time service), not a reason to run
the camera application as root. Run the application as the normal desktop
user. When no loopback output is configured, `Camera::builder(...).build()`
returns an actionable error containing the setup command instead of silently
failing or requesting elevated privileges.

`exclusive_caps=1` lets browsers and WebRTC clients recognize the device as a
capture-only camera while the producer is active. The backend uses the full
V4L2 mmap lifecycle rather than the limited `write()` path.

## Features

```text
default          media-foundation + obs + v4l2
media-foundation bundled Windows 11 VCam transport
obs              OBS Virtual Camera transport for Windows 10/11
v4l2             mmap v4l2loopback transport for Linux
gpu              cudarc device-to-pinned-host native frame publishing
```

Only dependencies for the current target platform are compiled.

## License

AGPL-3.0. Native upstream attribution is documented in
[`cpp/VCam/UPSTREAM.md`](cpp/VCam/UPSTREAM.md).

## Special Thanks
A big thanks to [`smourier`](https://github.com/smourier) for [`VCamSample`](https://github.com/smourier/VCamSample) — without it, it would have been difficult to add native support for VirtualCam on Windows 11!
