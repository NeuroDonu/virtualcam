# Native implementation attribution

The initial Media Foundation COM source and registrar were derived from
[smourier/VCamSample](https://github.com/smourier/VCamSample).

VCam keeps the original native license in [LICENSE](LICENSE). The maintained
implementation differs materially from the reference project:

- product-specific solution and runtime artifacts;
- a single NV12 media type with a shared format contract;
- external Rust and CUDA frame producers;
- cross-session shared-memory transport with a seqlock;
- row-aware writes into padded Media Foundation surfaces;
- headless registrar lifecycle;
- deterministic end-to-end verification and installation scripts.

The upstream repository remains the historical source, not the build or
runtime dependency of this module.
