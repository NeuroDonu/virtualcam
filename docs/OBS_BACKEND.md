# OBS backend provenance

The Windows OBS backend implements the shared-memory protocol published by OBS
Studio in:

- `plugins/win-dshow/shared-memory-queue.h`
- `plugins/win-dshow/shared-memory-queue.c`

Upstream: <https://github.com/obsproject/obs-studio>

The interoperability contract consists of the `OBSVirtualCamVideo` mapping,
an 80-byte queue header, three 32-byte-aligned NV12 frame slots, per-slot
timestamps, and the STARTING/READY/STOPPING state machine. `virtualcam` keeps
compile-time layout assertions next to its Rust representation.
