//! CUDA-accelerated image format conversion
//!
//! This module provides GPU-accelerated conversion between pixel formats
//! using cudarc Driver API. Enabled with the "cuda" feature.
//!
//! PTX kernels are precompiled at build time by build.rs using nvcc.

use crate::error::{Result, VirtualCamError};
use crate::pixel_format::PixelFormat;

use cudarc::driver::safe::{CudaContext, CudaFunction, CudaModule, CudaSlice, CudaStream, LaunchConfig};
use cudarc::driver::Ptx;
use std::sync::Arc;

/// Precompiled PTX embedded at compile time (built by build.rs)
const CONVERTER_PTX: &str = include_str!("kernels/converter.ptx");

/// CUDA converter for fast GPU-based format conversion
pub struct CudaConverter {
    ctx: Arc<CudaContext>,
    stream: Arc<CudaStream>,
    module: Arc<CudaModule>,

    // Preallocated buffers
    width: u32,
    height: u32,
    input_buffer: Option<CudaSlice<u8>>,
    output_y: Option<CudaSlice<u8>>,
    output_uv: Option<CudaSlice<u8>>,
}

impl CudaConverter {
    /// Create a new CUDA converter
    pub fn new() -> Result<Self> {
        Self::with_device(0)
    }

    /// Create converter on specific GPU device
    pub fn with_device(device_id: usize) -> Result<Self> {
        let ctx = CudaContext::new(device_id).map_err(|e| {
            VirtualCamError::InitializationFailed(format!("Failed to init CUDA device: {}", e))
        })?;

        let stream = ctx.default_stream();

        // Load precompiled PTX (built by build.rs)
        let ptx = Ptx::from_src(CONVERTER_PTX);

        let module = ctx.load_module(ptx).map_err(|e| {
            VirtualCamError::InitializationFailed(format!("Failed to load PTX: {}", e))
        })?;

        Ok(Self {
            ctx,
            stream,
            module,
            width: 0,
            height: 0,
            input_buffer: None,
            output_y: None,
            output_uv: None,
        })
    }

    /// Get kernel function by name
    fn get_func(&self, name: &str) -> Result<CudaFunction> {
        self.module.get_func(name).ok_or_else(|| {
            VirtualCamError::InitializationFailed(format!("Kernel {} not found", name))
        })
    }

    /// Preallocate buffers for given dimensions (call once, reuse many times)
    pub fn init_buffers(&mut self, width: u32, height: u32, input_format: PixelFormat) -> Result<()> {
        if self.width == width && self.height == height {
            return Ok(());
        }

        let y_size = (width * height) as usize;
        let uv_size = y_size / 2;
        let input_size = input_format.frame_size(width, height);

        self.input_buffer = Some(self.stream.alloc_zeros(input_size).map_err(|e| {
            VirtualCamError::InitializationFailed(format!("Failed to alloc input: {}", e))
        })?);

        self.output_y = Some(self.stream.alloc_zeros(y_size).map_err(|e| {
            VirtualCamError::InitializationFailed(format!("Failed to alloc Y plane: {}", e))
        })?);

        self.output_uv = Some(self.stream.alloc_zeros(uv_size).map_err(|e| {
            VirtualCamError::InitializationFailed(format!("Failed to alloc UV plane: {}", e))
        })?);

        self.width = width;
        self.height = height;

        Ok(())
    }

    /// Convert frame to NV12 using CUDA
    ///
    /// Returns the converted NV12 data
    pub fn convert_to_nv12(
        &mut self,
        src: &[u8],
        src_format: PixelFormat,
        width: u32,
        height: u32,
    ) -> Result<Vec<u8>> {
        // Ensure buffers are allocated
        self.init_buffers(width, height, src_format)?;

        let input = self.input_buffer.as_mut().unwrap();
        let output_y = self.output_y.as_mut().unwrap();
        let output_uv = self.output_uv.as_mut().unwrap();

        // Copy input to GPU
        self.stream.memcpy_htod(src, input).map_err(|e| {
            VirtualCamError::SendFailed(format!("Failed to copy to GPU: {}", e))
        })?;

        // Launch config
        let block_dim = (16, 16, 1);
        let grid_dim = (
            (width + 15) / 16,
            (height + 15) / 16,
            1,
        );

        // Select kernel based on input format
        let kernel_name = match src_format {
            PixelFormat::RGB => "rgb_to_nv12",
            PixelFormat::BGR => "bgr_to_nv12",
            PixelFormat::RGBA => "rgba_to_nv12",
            _ => {
                return Err(VirtualCamError::UnsupportedFormat(format!(
                    "CUDA conversion from {} to NV12 not supported",
                    src_format
                )))
            }
        };

        let func = self.get_func(kernel_name)?;

        // Launch kernel using launch_builder
        unsafe {
            self.stream
                .launch_builder(&func)
                .arg(input)
                .arg(output_y)
                .arg(output_uv)
                .arg(&(width as i32))
                .arg(&(height as i32))
                .launch(LaunchConfig {
                    grid_dim,
                    block_dim,
                    shared_mem_bytes: 0,
                })
        }.map_err(|e| {
            VirtualCamError::SendFailed(format!("Kernel launch failed: {}", e))
        })?;

        // Copy result back
        let y_size = (width * height) as usize;
        let uv_size = y_size / 2;

        let y_data = self.stream.clone_dtoh(output_y).map_err(|e| {
            VirtualCamError::SendFailed(format!("Failed to copy Y from GPU: {}", e))
        })?;
        let uv_data = self.stream.clone_dtoh(output_uv).map_err(|e| {
            VirtualCamError::SendFailed(format!("Failed to copy UV from GPU: {}", e))
        })?;

        let mut result = vec![0u8; y_size + uv_size];
        result[..y_size].copy_from_slice(&y_data);
        result[y_size..].copy_from_slice(&uv_data);

        Ok(result)
    }

    /// Check if CUDA is available
    pub fn is_available() -> bool {
        CudaContext::new(0).is_ok()
    }

    /// Get device name
    pub fn device_name(&self) -> String {
        self.ctx.name().unwrap_or_else(|_| "Unknown".to_string())
    }
}

/// Convert frame using CUDA (convenience function)
pub fn cuda_convert_to_nv12(
    src: &[u8],
    src_format: PixelFormat,
    width: u32,
    height: u32,
) -> Result<Vec<u8>> {
    let mut converter = CudaConverter::new()?;
    converter.convert_to_nv12(src, src_format, width, height)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cuda_available() {
        if CudaConverter::is_available() {
            let converter = CudaConverter::new();
            assert!(converter.is_ok());
        }
    }
}
