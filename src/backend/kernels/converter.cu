// nvcc -ptx kernels.cu -arch=compute_60 -code=compute_60 -use_fast_math -o kernels.ptx

// --- HELPERS ---

__device__ __forceinline__ unsigned int pack_argb(float r, float g, float b) {
    unsigned int ir = (unsigned int)(__saturatef(r) * 255.0f + 0.5f);
    unsigned int ig = (unsigned int)(__saturatef(g) * 255.0f + 0.5f);
    unsigned int ib = (unsigned int)(__saturatef(b) * 255.0f + 0.5f);
    return (255u << 24) | (ir << 16) | (ig << 8) | ib;
}

__device__ __forceinline__ float tonemap(float c) {
    return __fdividef(c, c + 1.0f);
}

extern "C" {

    // 1. NV12 -> ARGB
    __global__ void nv12_to_argb_sdr(
        const unsigned char* __restrict__ src_y, 
        const unsigned char* __restrict__ src_uv, 
        unsigned int* __restrict__ dst, 
        int width, int height, 
        int y_stride, int uv_stride,
        int is_full_range
    ) {
        int x = blockIdx.x * blockDim.x + threadIdx.x;
        int y = blockIdx.y * blockDim.y + threadIdx.y;
        if (x >= width || y >= height) return;

        float Y_raw = (float)src_y[y * y_stride + x];
        float Y;

        if (is_full_range) {
            Y = Y_raw * 0.003921569f; 
        } else {
            Y = (Y_raw - 16.0f) * 0.004566210f; 
        }

        int uv_idx = (y >> 1) * uv_stride + (x & ~1);
        float U = (float)src_uv[uv_idx] * 0.003921569f - 0.5f;
        float V = (float)src_uv[uv_idx + 1] * 0.003921569f - 0.5f;

        float r = __fmaf_rn(1.402f, V, Y);
        float g = Y - 0.344136f * U - 0.714136f * V;
        float b = __fmaf_rn(1.772f, U, Y);

        dst[y * width + x] = pack_argb(r, g, b);
    }

    // 3. P010 -> ARGB (HDR)
    __global__ void p010_to_argb_hdr(
        const unsigned char* __restrict__ src_y, 
        const unsigned char* __restrict__ src_uv, 
        unsigned int* __restrict__ dst, 
        int width, int height, 
        int y_stride, int uv_stride
    ) {
        int x = blockIdx.x * blockDim.x + threadIdx.x;
        int y = blockIdx.y * blockDim.y + threadIdx.y;
        if (x >= width || y >= height) return;

        const unsigned short* sy = (const unsigned short*)src_y;
        const unsigned short* suv = (const unsigned short*)src_uv;
        int ys_short = y_stride >> 1;
        int uvs_short = uv_stride >> 1;

        float Y = (float)sy[y * ys_short + x] * 0.000015259f;
        int uv_idx = (y >> 1) * uvs_short + (x & ~1);
        float U = ((float)suv[uv_idx] * 0.000015259f) - 0.5f;
        float V = ((float)suv[uv_idx + 1] * 0.000015259f) - 0.5f;

        float r = __fmaf_rn(1.4746f, V, Y);
        float g = Y - 0.16455f * U - 0.57135f * V;
        float b = __fmaf_rn(1.8814f, U, Y);

        dst[y * width + x] = pack_argb(tonemap(r), tonemap(g), tonemap(b));
    }

    // --- RGB/BGR/RGBA to NV12 conversions (for virtual camera output) ---

    // RGB -> NV12
    __global__ void rgb_to_nv12(
        const unsigned char* __restrict__ rgb,
        unsigned char* __restrict__ nv12_y,
        unsigned char* __restrict__ nv12_uv,
        int width, int height
    ) {
        int x = blockIdx.x * blockDim.x + threadIdx.x;
        int y = blockIdx.y * blockDim.y + threadIdx.y;
        if (x >= width || y >= height) return;

        int pixel_idx = y * width + x;
        int rgb_idx = pixel_idx * 3;

        unsigned char r = rgb[rgb_idx];
        unsigned char g = rgb[rgb_idx + 1];
        unsigned char b = rgb[rgb_idx + 2];

        // Y = 0.299*R + 0.587*G + 0.114*B (BT.601)
        int y_val = ((66 * r + 129 * g + 25 * b + 128) >> 8) + 16;
        nv12_y[pixel_idx] = (unsigned char)min(max(y_val, 0), 255);

        // UV for 2x2 blocks
        if ((x & 1) == 0 && (y & 1) == 0) {
            int uv_idx = (y >> 1) * width + x;

            // Simple: use top-left pixel for UV
            int u_val = ((-38 * r - 74 * g + 112 * b + 128) >> 8) + 128;
            int v_val = ((112 * r - 94 * g - 18 * b + 128) >> 8) + 128;

            nv12_uv[uv_idx] = (unsigned char)min(max(u_val, 0), 255);
            nv12_uv[uv_idx + 1] = (unsigned char)min(max(v_val, 0), 255);
        }
    }

    // BGR -> NV12 (OpenCV format)
    __global__ void bgr_to_nv12(
        const unsigned char* __restrict__ bgr,
        unsigned char* __restrict__ nv12_y,
        unsigned char* __restrict__ nv12_uv,
        int width, int height
    ) {
        int x = blockIdx.x * blockDim.x + threadIdx.x;
        int y = blockIdx.y * blockDim.y + threadIdx.y;
        if (x >= width || y >= height) return;

        int pixel_idx = y * width + x;
        int bgr_idx = pixel_idx * 3;

        unsigned char b = bgr[bgr_idx];
        unsigned char g = bgr[bgr_idx + 1];
        unsigned char r = bgr[bgr_idx + 2];

        int y_val = ((66 * r + 129 * g + 25 * b + 128) >> 8) + 16;
        nv12_y[pixel_idx] = (unsigned char)min(max(y_val, 0), 255);

        if ((x & 1) == 0 && (y & 1) == 0) {
            int uv_idx = (y >> 1) * width + x;

            int u_val = ((-38 * r - 74 * g + 112 * b + 128) >> 8) + 128;
            int v_val = ((112 * r - 94 * g - 18 * b + 128) >> 8) + 128;

            nv12_uv[uv_idx] = (unsigned char)min(max(u_val, 0), 255);
            nv12_uv[uv_idx + 1] = (unsigned char)min(max(v_val, 0), 255);
        }
    }

    // RGBA -> NV12
    __global__ void rgba_to_nv12(
        const unsigned char* __restrict__ rgba,
        unsigned char* __restrict__ nv12_y,
        unsigned char* __restrict__ nv12_uv,
        int width, int height
    ) {
        int x = blockIdx.x * blockDim.x + threadIdx.x;
        int y = blockIdx.y * blockDim.y + threadIdx.y;
        if (x >= width || y >= height) return;

        int pixel_idx = y * width + x;
        int rgba_idx = pixel_idx * 4;

        unsigned char r = rgba[rgba_idx];
        unsigned char g = rgba[rgba_idx + 1];
        unsigned char b = rgba[rgba_idx + 2];

        int y_val = ((66 * r + 129 * g + 25 * b + 128) >> 8) + 16;
        nv12_y[pixel_idx] = (unsigned char)min(max(y_val, 0), 255);

        if ((x & 1) == 0 && (y & 1) == 0) {
            int uv_idx = (y >> 1) * width + x;

            int u_val = ((-38 * r - 74 * g + 112 * b + 128) >> 8) + 128;
            int v_val = ((112 * r - 94 * g - 18 * b + 128) >> 8) + 128;

            nv12_uv[uv_idx] = (unsigned char)min(max(u_val, 0), 255);
            nv12_uv[uv_idx + 1] = (unsigned char)min(max(v_val, 0), 255);
        }
    }

    // ARGB -> NV12 (packed ARGB as uint32)
    __global__ void argb_to_nv12(
        const unsigned int* __restrict__ argb,
        unsigned char* __restrict__ nv12_y,
        unsigned char* __restrict__ nv12_uv,
        int width, int height
    ) {
        int x = blockIdx.x * blockDim.x + threadIdx.x;
        int y = blockIdx.y * blockDim.y + threadIdx.y;
        if (x >= width || y >= height) return;

        int pixel_idx = y * width + x;
        unsigned int pixel = argb[pixel_idx];

        unsigned char r = (pixel >> 16) & 0xFF;
        unsigned char g = (pixel >> 8) & 0xFF;
        unsigned char b = pixel & 0xFF;

        int y_val = ((66 * r + 129 * g + 25 * b + 128) >> 8) + 16;
        nv12_y[pixel_idx] = (unsigned char)min(max(y_val, 0), 255);

        if ((x & 1) == 0 && (y & 1) == 0) {
            int uv_idx = (y >> 1) * width + x;

            int u_val = ((-38 * r - 74 * g + 112 * b + 128) >> 8) + 128;
            int v_val = ((112 * r - 94 * g - 18 * b + 128) >> 8) + 128;

            nv12_uv[uv_idx] = (unsigned char)min(max(u_val, 0), 255);
            nv12_uv[uv_idx + 1] = (unsigned char)min(max(v_val, 0), 255);
        }
    }
}