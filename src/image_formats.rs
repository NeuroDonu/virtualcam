//! Image format conversion utilities

use crate::pixel_format::PixelFormat;

/// Convert frame from one pixel format to another
pub fn convert_frame(
    src: &[u8],
    src_format: PixelFormat,
    dst_format: PixelFormat,
    width: u32,
    height: u32,
) -> Vec<u8> {
    if src_format == dst_format {
        return src.to_vec();
    }

    match (src_format, dst_format) {
        // RGB conversions
        (PixelFormat::RGB, PixelFormat::BGR) => rgb_to_bgr(src),
        (PixelFormat::RGB, PixelFormat::RGBA) => rgb_to_rgba(src),
        (PixelFormat::RGB, PixelFormat::I420) => rgb_to_i420(src, width, height),
        (PixelFormat::RGB, PixelFormat::NV12) => rgb_to_nv12(src, width, height),

        // BGR conversions
        (PixelFormat::BGR, PixelFormat::RGB) => bgr_to_rgb(src),
        (PixelFormat::BGR, PixelFormat::RGBA) => bgr_to_rgba(src),
        (PixelFormat::BGR, PixelFormat::I420) => bgr_to_i420(src, width, height),
        (PixelFormat::BGR, PixelFormat::NV12) => bgr_to_nv12(src, width, height),

        // RGBA conversions
        (PixelFormat::RGBA, PixelFormat::RGB) => rgba_to_rgb(src),
        (PixelFormat::RGBA, PixelFormat::BGR) => rgba_to_bgr(src),
        (PixelFormat::RGBA, PixelFormat::NV12) => rgba_to_nv12(src, width, height),

        // GRAY conversions
        (PixelFormat::GRAY, PixelFormat::RGB) => gray_to_rgb(src),
        (PixelFormat::GRAY, PixelFormat::BGR) => gray_to_bgr(src),
        (PixelFormat::GRAY, PixelFormat::RGBA) => gray_to_rgba(src),
        (PixelFormat::GRAY, PixelFormat::NV12) => gray_to_nv12(src, width, height),

        // I420 conversions
        (PixelFormat::I420, PixelFormat::NV12) => i420_to_nv12(src, width, height),
        (PixelFormat::I420, PixelFormat::RGBA) => i420_to_rgba(src, width, height),

        // NV12 conversions
        (PixelFormat::NV12, PixelFormat::I420) => nv12_to_i420(src, width, height),
        (PixelFormat::NV12, PixelFormat::RGBA) => nv12_to_rgba(src, width, height),

        // YUYV conversions
        (PixelFormat::YUYV, PixelFormat::NV12) => yuyv_to_nv12(src, width, height),
        (PixelFormat::YUYV, PixelFormat::RGBA) => yuyv_to_rgba(src, width, height),

        // UYVY conversions
        (PixelFormat::UYVY, PixelFormat::NV12) => uyvy_to_nv12(src, width, height),
        (PixelFormat::UYVY, PixelFormat::RGBA) => uyvy_to_rgba(src, width, height),

        // Unsupported conversions - return empty
        _ => Vec::new(),
    }
}

/// Flip frame vertically (for Unity Capture)
pub fn flip_vertical(data: &mut [u8], width: u32, height: u32, bytes_per_pixel: u32) {
    let stride = (width * bytes_per_pixel) as usize;
    let mut temp = vec![0u8; stride];

    for y in 0..(height as usize / 2) {
        let top_offset = y * stride;
        let bottom_offset = (height as usize - 1 - y) * stride;

        temp.copy_from_slice(&data[top_offset..top_offset + stride]);
        data.copy_within(bottom_offset..bottom_offset + stride, top_offset);
        data[bottom_offset..bottom_offset + stride].copy_from_slice(&temp);
    }
}

// RGB <-> BGR conversions
fn rgb_to_bgr(src: &[u8]) -> Vec<u8> {
    let mut dst = vec![0u8; src.len()];
    for i in (0..src.len()).step_by(3) {
        dst[i] = src[i + 2];     // B
        dst[i + 1] = src[i + 1]; // G
        dst[i + 2] = src[i];     // R
    }
    dst
}

fn bgr_to_rgb(src: &[u8]) -> Vec<u8> {
    rgb_to_bgr(src) // Same operation
}

// RGB/BGR to RGBA
fn rgb_to_rgba(src: &[u8]) -> Vec<u8> {
    let pixels = src.len() / 3;
    let mut dst = vec![255u8; pixels * 4];
    for i in 0..pixels {
        dst[i * 4] = src[i * 3];
        dst[i * 4 + 1] = src[i * 3 + 1];
        dst[i * 4 + 2] = src[i * 3 + 2];
        // Alpha is already 255
    }
    dst
}

fn bgr_to_rgba(src: &[u8]) -> Vec<u8> {
    let pixels = src.len() / 3;
    let mut dst = vec![255u8; pixels * 4];
    for i in 0..pixels {
        dst[i * 4] = src[i * 3 + 2];     // R
        dst[i * 4 + 1] = src[i * 3 + 1]; // G
        dst[i * 4 + 2] = src[i * 3];     // B
        // Alpha is already 255
    }
    dst
}

// RGBA to RGB/BGR
fn rgba_to_rgb(src: &[u8]) -> Vec<u8> {
    let pixels = src.len() / 4;
    let mut dst = vec![0u8; pixels * 3];
    for i in 0..pixels {
        dst[i * 3] = src[i * 4];
        dst[i * 3 + 1] = src[i * 4 + 1];
        dst[i * 3 + 2] = src[i * 4 + 2];
    }
    dst
}

fn rgba_to_bgr(src: &[u8]) -> Vec<u8> {
    let pixels = src.len() / 4;
    let mut dst = vec![0u8; pixels * 3];
    for i in 0..pixels {
        dst[i * 3] = src[i * 4 + 2];     // B
        dst[i * 3 + 1] = src[i * 4 + 1]; // G
        dst[i * 3 + 2] = src[i * 4];     // R
    }
    dst
}

// Grayscale conversions
fn gray_to_rgb(src: &[u8]) -> Vec<u8> {
    let mut dst = vec![0u8; src.len() * 3];
    for (i, &g) in src.iter().enumerate() {
        dst[i * 3] = g;
        dst[i * 3 + 1] = g;
        dst[i * 3 + 2] = g;
    }
    dst
}

fn gray_to_bgr(src: &[u8]) -> Vec<u8> {
    gray_to_rgb(src) // Same for grayscale
}

fn gray_to_rgba(src: &[u8]) -> Vec<u8> {
    let mut dst = vec![255u8; src.len() * 4];
    for (i, &g) in src.iter().enumerate() {
        dst[i * 4] = g;
        dst[i * 4 + 1] = g;
        dst[i * 4 + 2] = g;
        // Alpha is already 255
    }
    dst
}

fn gray_to_nv12(src: &[u8], width: u32, height: u32) -> Vec<u8> {
    let y_size = (width * height) as usize;
    let uv_size = y_size / 2;
    let mut dst = vec![0u8; y_size + uv_size];

    // Y plane is just the grayscale values
    dst[..y_size].copy_from_slice(src);

    // UV plane: neutral chroma (128)
    for i in 0..uv_size {
        dst[y_size + i] = 128;
    }

    dst
}

// RGB/BGR to YUV conversions
fn rgb_to_yuv(r: u8, g: u8, b: u8) -> (u8, u8, u8) {
    let r = r as i32;
    let g = g as i32;
    let b = b as i32;

    let y = ((66 * r + 129 * g + 25 * b + 128) >> 8) + 16;
    let u = ((-38 * r - 74 * g + 112 * b + 128) >> 8) + 128;
    let v = ((112 * r - 94 * g - 18 * b + 128) >> 8) + 128;

    (
        y.clamp(0, 255) as u8,
        u.clamp(0, 255) as u8,
        v.clamp(0, 255) as u8,
    )
}

fn rgb_to_i420(src: &[u8], width: u32, height: u32) -> Vec<u8> {
    let w = width as usize;
    let h = height as usize;
    let y_size = w * h;
    let uv_size = y_size / 4;

    let mut dst = vec![0u8; y_size + uv_size * 2];
    let (y_plane, uv_planes) = dst.split_at_mut(y_size);
    let (u_plane, v_plane) = uv_planes.split_at_mut(uv_size);

    // Process Y plane
    for y in 0..h {
        for x in 0..w {
            let idx = (y * w + x) * 3;
            let (yy, _, _) = rgb_to_yuv(src[idx], src[idx + 1], src[idx + 2]);
            y_plane[y * w + x] = yy;
        }
    }

    // Process U and V planes (subsampled 2x2)
    for y in (0..h).step_by(2) {
        for x in (0..w).step_by(2) {
            let idx = (y * w + x) * 3;
            let (_, u, v) = rgb_to_yuv(src[idx], src[idx + 1], src[idx + 2]);
            let uv_idx = (y / 2) * (w / 2) + (x / 2);
            u_plane[uv_idx] = u;
            v_plane[uv_idx] = v;
        }
    }

    dst
}

fn bgr_to_i420(src: &[u8], width: u32, height: u32) -> Vec<u8> {
    let w = width as usize;
    let h = height as usize;
    let y_size = w * h;
    let uv_size = y_size / 4;

    let mut dst = vec![0u8; y_size + uv_size * 2];
    let (y_plane, uv_planes) = dst.split_at_mut(y_size);
    let (u_plane, v_plane) = uv_planes.split_at_mut(uv_size);

    // Process Y plane
    for y in 0..h {
        for x in 0..w {
            let idx = (y * w + x) * 3;
            let (yy, _, _) = rgb_to_yuv(src[idx + 2], src[idx + 1], src[idx]); // BGR order
            y_plane[y * w + x] = yy;
        }
    }

    // Process U and V planes (subsampled 2x2)
    for y in (0..h).step_by(2) {
        for x in (0..w).step_by(2) {
            let idx = (y * w + x) * 3;
            let (_, u, v) = rgb_to_yuv(src[idx + 2], src[idx + 1], src[idx]);
            let uv_idx = (y / 2) * (w / 2) + (x / 2);
            u_plane[uv_idx] = u;
            v_plane[uv_idx] = v;
        }
    }

    dst
}

fn rgb_to_nv12(src: &[u8], width: u32, height: u32) -> Vec<u8> {
    let i420 = rgb_to_i420(src, width, height);
    i420_to_nv12(&i420, width, height)
}

fn bgr_to_nv12(src: &[u8], width: u32, height: u32) -> Vec<u8> {
    let i420 = bgr_to_i420(src, width, height);
    i420_to_nv12(&i420, width, height)
}

fn rgba_to_nv12(src: &[u8], width: u32, height: u32) -> Vec<u8> {
    let rgb = rgba_to_rgb(src);
    rgb_to_nv12(&rgb, width, height)
}

// I420 <-> NV12 conversions
fn i420_to_nv12(src: &[u8], width: u32, height: u32) -> Vec<u8> {
    let w = width as usize;
    let h = height as usize;
    let y_size = w * h;
    let uv_size = y_size / 4;

    let mut dst = vec![0u8; y_size + uv_size * 2];

    // Copy Y plane
    dst[..y_size].copy_from_slice(&src[..y_size]);

    // Interleave U and V planes
    let u_plane = &src[y_size..y_size + uv_size];
    let v_plane = &src[y_size + uv_size..];
    let uv_dst = &mut dst[y_size..];

    for i in 0..uv_size {
        uv_dst[i * 2] = u_plane[i];
        uv_dst[i * 2 + 1] = v_plane[i];
    }

    dst
}

fn nv12_to_i420(src: &[u8], width: u32, height: u32) -> Vec<u8> {
    let w = width as usize;
    let h = height as usize;
    let y_size = w * h;
    let uv_size = y_size / 4;

    let mut dst = vec![0u8; y_size + uv_size * 2];

    // Copy Y plane
    dst[..y_size].copy_from_slice(&src[..y_size]);

    // Deinterleave UV plane
    let uv_src = &src[y_size..];
    let (u_plane, v_plane) = dst[y_size..].split_at_mut(uv_size);

    for i in 0..uv_size {
        u_plane[i] = uv_src[i * 2];
        v_plane[i] = uv_src[i * 2 + 1];
    }

    dst
}

// YUV to RGB conversion
fn yuv_to_rgb(y: u8, u: u8, v: u8) -> (u8, u8, u8) {
    let y = y as i32 - 16;
    let u = u as i32 - 128;
    let v = v as i32 - 128;

    let r = (298 * y + 409 * v + 128) >> 8;
    let g = (298 * y - 100 * u - 208 * v + 128) >> 8;
    let b = (298 * y + 516 * u + 128) >> 8;

    (
        r.clamp(0, 255) as u8,
        g.clamp(0, 255) as u8,
        b.clamp(0, 255) as u8,
    )
}

fn i420_to_rgba(src: &[u8], width: u32, height: u32) -> Vec<u8> {
    let w = width as usize;
    let h = height as usize;
    let y_size = w * h;
    let uv_size = y_size / 4;

    let y_plane = &src[..y_size];
    let u_plane = &src[y_size..y_size + uv_size];
    let v_plane = &src[y_size + uv_size..];

    let mut dst = vec![255u8; w * h * 4];

    for y in 0..h {
        for x in 0..w {
            let y_val = y_plane[y * w + x];
            let uv_idx = (y / 2) * (w / 2) + (x / 2);
            let u_val = u_plane[uv_idx];
            let v_val = v_plane[uv_idx];

            let (r, g, b) = yuv_to_rgb(y_val, u_val, v_val);
            let dst_idx = (y * w + x) * 4;
            dst[dst_idx] = r;
            dst[dst_idx + 1] = g;
            dst[dst_idx + 2] = b;
            // Alpha is already 255
        }
    }

    dst
}

fn nv12_to_rgba(src: &[u8], width: u32, height: u32) -> Vec<u8> {
    let w = width as usize;
    let h = height as usize;
    let y_size = w * h;

    let y_plane = &src[..y_size];
    let uv_plane = &src[y_size..];

    let mut dst = vec![255u8; w * h * 4];

    for y in 0..h {
        for x in 0..w {
            let y_val = y_plane[y * w + x];
            let uv_idx = (y / 2) * (w / 2) + (x / 2);
            let u_val = uv_plane[uv_idx * 2];
            let v_val = uv_plane[uv_idx * 2 + 1];

            let (r, g, b) = yuv_to_rgb(y_val, u_val, v_val);
            let dst_idx = (y * w + x) * 4;
            dst[dst_idx] = r;
            dst[dst_idx + 1] = g;
            dst[dst_idx + 2] = b;
        }
    }

    dst
}

// YUYV/UYVY conversions
fn yuyv_to_nv12(src: &[u8], width: u32, height: u32) -> Vec<u8> {
    let w = width as usize;
    let h = height as usize;
    let y_size = w * h;
    let uv_size = y_size / 2;

    let mut dst = vec![0u8; y_size + uv_size];
    let (y_plane, uv_plane) = dst.split_at_mut(y_size);

    // YUYV: Y0 U0 Y1 V0 Y2 U2 Y3 V2 ...
    for y in 0..h {
        for x in (0..w).step_by(2) {
            let src_idx = (y * w + x) * 2;
            let y0 = src[src_idx];
            let u = src[src_idx + 1];
            let y1 = src[src_idx + 2];
            let v = src[src_idx + 3];

            y_plane[y * w + x] = y0;
            y_plane[y * w + x + 1] = y1;

            // Write UV for every 2nd row
            if y % 2 == 0 {
                let uv_idx = (y / 2) * w + x;
                uv_plane[uv_idx] = u;
                uv_plane[uv_idx + 1] = v;
            }
        }
    }

    dst
}

fn uyvy_to_nv12(src: &[u8], width: u32, height: u32) -> Vec<u8> {
    let w = width as usize;
    let h = height as usize;
    let y_size = w * h;
    let uv_size = y_size / 2;

    let mut dst = vec![0u8; y_size + uv_size];
    let (y_plane, uv_plane) = dst.split_at_mut(y_size);

    // UYVY: U0 Y0 V0 Y1 U2 Y2 V2 Y3 ...
    for y in 0..h {
        for x in (0..w).step_by(2) {
            let src_idx = (y * w + x) * 2;
            let u = src[src_idx];
            let y0 = src[src_idx + 1];
            let v = src[src_idx + 2];
            let y1 = src[src_idx + 3];

            y_plane[y * w + x] = y0;
            y_plane[y * w + x + 1] = y1;

            if y % 2 == 0 {
                let uv_idx = (y / 2) * w + x;
                uv_plane[uv_idx] = u;
                uv_plane[uv_idx + 1] = v;
            }
        }
    }

    dst
}

fn yuyv_to_rgba(src: &[u8], width: u32, height: u32) -> Vec<u8> {
    let w = width as usize;
    let h = height as usize;
    let mut dst = vec![255u8; w * h * 4];

    for y in 0..h {
        for x in (0..w).step_by(2) {
            let src_idx = (y * w + x) * 2;
            let y0 = src[src_idx];
            let u = src[src_idx + 1];
            let y1 = src[src_idx + 2];
            let v = src[src_idx + 3];

            let (r0, g0, b0) = yuv_to_rgb(y0, u, v);
            let (r1, g1, b1) = yuv_to_rgb(y1, u, v);

            let dst_idx0 = (y * w + x) * 4;
            dst[dst_idx0] = r0;
            dst[dst_idx0 + 1] = g0;
            dst[dst_idx0 + 2] = b0;

            let dst_idx1 = (y * w + x + 1) * 4;
            dst[dst_idx1] = r1;
            dst[dst_idx1 + 1] = g1;
            dst[dst_idx1 + 2] = b1;
        }
    }

    dst
}

fn uyvy_to_rgba(src: &[u8], width: u32, height: u32) -> Vec<u8> {
    let w = width as usize;
    let h = height as usize;
    let mut dst = vec![255u8; w * h * 4];

    for y in 0..h {
        for x in (0..w).step_by(2) {
            let src_idx = (y * w + x) * 2;
            let u = src[src_idx];
            let y0 = src[src_idx + 1];
            let v = src[src_idx + 2];
            let y1 = src[src_idx + 3];

            let (r0, g0, b0) = yuv_to_rgb(y0, u, v);
            let (r1, g1, b1) = yuv_to_rgb(y1, u, v);

            let dst_idx0 = (y * w + x) * 4;
            dst[dst_idx0] = r0;
            dst[dst_idx0 + 1] = g0;
            dst[dst_idx0 + 2] = b0;

            let dst_idx1 = (y * w + x + 1) * 4;
            dst[dst_idx1] = r1;
            dst[dst_idx1 + 1] = g1;
            dst[dst_idx1 + 2] = b1;
        }
    }

    dst
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rgb_bgr_conversion() {
        let rgb = vec![255, 0, 0, 0, 255, 0, 0, 0, 255];
        let bgr = rgb_to_bgr(&rgb);
        assert_eq!(bgr, vec![0, 0, 255, 0, 255, 0, 255, 0, 0]);
    }

    #[test]
    fn test_rgb_rgba_conversion() {
        let rgb = vec![255, 128, 64];
        let rgba = rgb_to_rgba(&rgb);
        assert_eq!(rgba, vec![255, 128, 64, 255]);
    }

    #[test]
    fn test_i420_nv12_roundtrip() {
        let width = 4;
        let height = 4;
        let y_size = (width * height) as usize;
        let uv_size = y_size / 4;

        let mut i420 = vec![0u8; y_size + uv_size * 2];
        for i in 0..y_size {
            i420[i] = i as u8;
        }
        for i in 0..uv_size {
            i420[y_size + i] = 100 + i as u8;
            i420[y_size + uv_size + i] = 200 + i as u8;
        }

        let nv12 = i420_to_nv12(&i420, width, height);
        let back = nv12_to_i420(&nv12, width, height);

        assert_eq!(i420, back);
    }
}
