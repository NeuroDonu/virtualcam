# Integration

## Application frames

Configure the format your application actually produces:

```rust,no_run
use virtualcam::{Camera, PixelFormat};

let mut camera = Camera::builder(1280, 720, 60.0)
    .format(PixelFormat::BGR)
    .build()?;
camera.send(&vec![0; 1280 * 720 * 3])?;
# Ok::<(), virtualcam::VirtualCamError>(())
```

`send` validates every frame. Conversion storage is retained by `Camera`, so
steady-state publication does not recreate the main destination buffer.

## Stable backend selection

Use automatic selection for desktop applications and explicit selection for
managed deployments:

```rust,no_run
use virtualcam::{BackendKind, Camera};

let obs = Camera::builder(1280, 720, 30.0)
    .backend(BackendKind::Obs)
    .build()?;

let linux = Camera::builder(1280, 720, 30.0)
    .backend(BackendKind::V4l2)
    .device("/dev/video10")
    .build()?;
# Ok::<(), virtualcam::VirtualCamError>(())
```

Call `available_backends()` to populate a UI without duplicating platform
detection in the application.

## Native fast path

High-throughput applications should query the selected contract once:

```rust,no_run
# use virtualcam::Camera;
# let mut camera = Camera::new(1280, 720, 30.0)?;
let format = camera.native_format();
let (width, height) = camera.native_dimensions();
let frame = vec![0; format.frame_size(width, height)];
camera.send_native(&frame)?;
# Ok::<(), virtualcam::VirtualCamError>(())
```

With the `gpu` feature, `send_native_gpu` accepts the same native frame in a
CUDA allocation. The pinned host staging allocation is created once and reused.

## Error handling

`VirtualCamError` distinguishes unavailable backends, missing or busy devices,
invalid dimensions, frame-size mismatches, initialization failures, and send
failures. Applications can display these errors directly or match variants for
recovery logic.
