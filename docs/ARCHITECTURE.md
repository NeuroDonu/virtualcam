# Architecture

`virtualcam` separates application-facing frame handling from platform
transports:

```text
CameraBuilder
    -> Camera (validation, conversion, scaling, pacing, statistics)
        -> Backend trait
            -> MediaFoundationBackend (Windows 11, NV12 1280x960)
            -> ObsBackend             (Windows 10/11, NV12, requested size)
            -> V4l2Backend            (Linux, RGB24, requested size)
```

Only `BackendKind::Auto` falls back. Explicit backend selection either opens
that backend or returns an error, which keeps deployments deterministic.

## Public frame paths

- `send`: validates the configured input format, converts it to the native
  backend format, and scales when the backend has fixed dimensions.
- `send_native`: accepts bytes already matching `native_format()` and
  `native_dimensions()`.
- `send_native_gpu` (`gpu` feature): downloads a native `CudaSlice<u8>` through
  one reusable pinned allocation before backend publication.

GPU layout and image processing remain application decisions. The library only
defines the final native frame contract.

## Windows Media Foundation

The bundled native source exposes one NV12 media type at 1280x960 and 30 FPS.
The Rust backend publishes frames to `Global\vcam_frames`; Windows Frame Server
loads `VCamSource.dll` and reads the same mapping. A sequence counter uses an
odd/even publication protocol so readers never accept a partially copied
frame.

The backend is available only on Windows build 22000 or newer and only when the
VCam COM source is registered. This lets automatic selection fall back to OBS
on Windows 10.

## OBS

The OBS backend implements the official `OBSVirtualCamVideo` three-slot queue:

```text
queue header -> slot 0 timestamp + NV12
             -> slot 1 timestamp + NV12
             -> slot 2 timestamp + NV12
```

Header state and indices use cross-process atomics. The layout is guarded by
compile-time size assertions and follows OBS Studio's
`plugins/win-dshow/shared-memory-queue.c` protocol.

## Linux V4L2

The V4L2 backend owns the complete streaming lifecycle:

```text
QUERYCAP -> S_FMT -> S_PARM -> REQBUFS -> QUERYBUF/mmap
         -> QBUF -> STREAMON -> DQBUF/copy/QBUF -> STREAMOFF/munmap
```

It accepts only `v4l2loopback` devices during automatic discovery, honors the
driver's actual mmap buffer count, validates negotiated RGB24 stride, and
computes ioctl request sizes from the current target ABI.

## Ownership

Each `Camera` exclusively owns one backend. Backends are `Send` but all frame
publication requires `&mut self`, preventing concurrent writes into one queue.
Dropping `Camera` closes the queue or stream and releases every mapping.
