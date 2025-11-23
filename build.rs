//! Build script for compiling CUDA kernels to PTX
//!
//! This script compiles .cu files to .ptx during build time,
//! so we don't need NVRTC at runtime.

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    // Only compile CUDA kernels if the cuda feature is enabled
    if env::var("CARGO_FEATURE_CUDA").is_err() {
        return;
    }

    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());

    let kernel_src = manifest_dir.join("src/backend/kernels/converter.cu");
    let ptx_out = out_dir.join("converter.ptx");

    // Rerun if kernel source changes
    println!("cargo:rerun-if-changed={}", kernel_src.display());

    // Find nvcc
    let nvcc = find_nvcc().expect("nvcc not found. Please install CUDA Toolkit and ensure nvcc is in PATH");

    // Compile .cu to .ptx
    let status = Command::new(&nvcc)
        .args([
            "-ptx",
            "-o", ptx_out.to_str().unwrap(),
            kernel_src.to_str().unwrap(),
            // Target compute capability 5.0+ (Maxwell and newer)
            "-arch=compute_50",
            "-code=compute_50",
            // Optimizations
            "--use_fast_math",
            "-O3",
            // Suppress warnings about deprecated GPU targets
            "-Wno-deprecated-gpu-targets",
        ])
        .status()
        .expect("Failed to execute nvcc");

    if !status.success() {
        panic!("nvcc failed to compile CUDA kernel");
    }

    println!("cargo:rustc-env=CONVERTER_PTX_PATH={}", ptx_out.display());
}

/// Find nvcc in common locations
fn find_nvcc() -> Option<PathBuf> {
    // First check PATH
    if let Ok(output) = Command::new("nvcc").arg("--version").output() {
        if output.status.success() {
            return Some(PathBuf::from("nvcc"));
        }
    }

    // Check common CUDA installation paths
    let cuda_paths = if cfg!(windows) {
        vec![
            r"C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v12.6\bin\nvcc.exe",
            r"C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v12.5\bin\nvcc.exe",
            r"C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v12.4\bin\nvcc.exe",
            r"C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v12.3\bin\nvcc.exe",
            r"C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v12.2\bin\nvcc.exe",
            r"C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v12.1\bin\nvcc.exe",
            r"C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v12.0\bin\nvcc.exe",
            r"C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v11.8\bin\nvcc.exe",
            r"C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v11.7\bin\nvcc.exe",
        ]
    } else {
        vec![
            "/usr/local/cuda/bin/nvcc",
            "/usr/local/cuda-12/bin/nvcc",
            "/usr/local/cuda-11/bin/nvcc",
            "/opt/cuda/bin/nvcc",
        ]
    };

    // Also check CUDA_PATH environment variable
    if let Ok(cuda_path) = env::var("CUDA_PATH") {
        let nvcc_path = if cfg!(windows) {
            Path::new(&cuda_path).join("bin/nvcc.exe")
        } else {
            Path::new(&cuda_path).join("bin/nvcc")
        };
        if nvcc_path.exists() {
            return Some(nvcc_path);
        }
    }

    for path in cuda_paths {
        let p = Path::new(path);
        if p.exists() {
            return Some(p.to_path_buf());
        }
    }

    None
}
