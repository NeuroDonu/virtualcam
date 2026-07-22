# Changelog

## 1.0.0

- Added the bundled Windows 11 Media Foundation VCam backend.
- Kept OBS Virtual Camera as a first-class Windows 10/11 backend.
- Replaced the Linux write path with full V4L2 mmap streaming.
- Added typed backend selection and deterministic automatic fallback.
- Added reusable conversion/scaling storage and native frame publication.
- Added optional CUDA device-frame publication through pinned host memory.
- Added native runtime bootstrap, installation, verification, and operations
  documentation.
- Added an actionable Linux error when no v4l2loopback device is configured;
  applications remain unprivileged and device setup stays an administrator task.
