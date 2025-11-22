//! Pixel format definitions

/// Supported pixel formats for virtual camera frames
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PixelFormat {
    /// RGB format, 3 bytes per pixel (Red, Green, Blue)
    RGB,
    /// BGR format, 3 bytes per pixel (Blue, Green, Red) - OpenCV default
    BGR,
    /// RGBA format, 4 bytes per pixel with alpha channel
    RGBA,
    /// Grayscale, 1 byte per pixel
    GRAY,
    /// YUV 4:2:0 planar (I420)
    I420,
    /// YUV 4:2:0 semi-planar (NV12)
    NV12,
    /// YUV 4:2:2 packed (YUYV)
    YUYV,
    /// YUV 4:2:2 packed (UYVY)
    UYVY,
}

impl PixelFormat {
    /// Get the FourCC code for this pixel format
    pub fn fourcc(&self) -> u32 {
        match self {
            PixelFormat::RGB => encode_fourcc(b"raw "),
            PixelFormat::BGR => encode_fourcc(b"24BG"),
            PixelFormat::RGBA => encode_fourcc(b"ABGR"),
            PixelFormat::GRAY => encode_fourcc(b"J400"),
            PixelFormat::I420 => encode_fourcc(b"I420"),
            PixelFormat::NV12 => encode_fourcc(b"NV12"),
            PixelFormat::YUYV => encode_fourcc(b"YUY2"),
            PixelFormat::UYVY => encode_fourcc(b"UYVY"),
        }
    }

    /// Get the number of bytes per pixel (for non-planar formats)
    /// For planar formats like I420/NV12, returns the average bytes per pixel
    pub fn bytes_per_pixel(&self) -> f32 {
        match self {
            PixelFormat::RGB | PixelFormat::BGR => 3.0,
            PixelFormat::RGBA => 4.0,
            PixelFormat::GRAY => 1.0,
            PixelFormat::I420 | PixelFormat::NV12 => 1.5,
            PixelFormat::YUYV | PixelFormat::UYVY => 2.0,
        }
    }

    /// Calculate the frame size in bytes for given dimensions
    pub fn frame_size(&self, width: u32, height: u32) -> usize {
        let pixels = (width * height) as usize;
        match self {
            PixelFormat::RGB | PixelFormat::BGR => pixels * 3,
            PixelFormat::RGBA => pixels * 4,
            PixelFormat::GRAY => pixels,
            PixelFormat::I420 | PixelFormat::NV12 => pixels * 3 / 2,
            PixelFormat::YUYV | PixelFormat::UYVY => pixels * 2,
        }
    }

    /// Get the expected shape of a frame with these dimensions
    /// Returns (height, width, channels) or (height, width) for grayscale
    pub fn frame_shape(&self, width: u32, height: u32) -> Vec<usize> {
        match self {
            PixelFormat::RGB | PixelFormat::BGR => {
                vec![height as usize, width as usize, 3]
            }
            PixelFormat::RGBA => vec![height as usize, width as usize, 4],
            PixelFormat::GRAY => vec![height as usize, width as usize],
            // Planar/packed formats are 1D arrays
            PixelFormat::I420 | PixelFormat::NV12 | PixelFormat::YUYV | PixelFormat::UYVY => {
                vec![self.frame_size(width, height)]
            }
        }
    }

    /// Check if this format is planar (separate Y, U, V planes)
    pub fn is_planar(&self) -> bool {
        matches!(self, PixelFormat::I420 | PixelFormat::NV12)
    }

    /// Check if this format is packed YUV
    pub fn is_packed_yuv(&self) -> bool {
        matches!(self, PixelFormat::YUYV | PixelFormat::UYVY)
    }

    /// Get format name as string
    pub fn name(&self) -> &'static str {
        match self {
            PixelFormat::RGB => "RGB",
            PixelFormat::BGR => "BGR",
            PixelFormat::RGBA => "RGBA",
            PixelFormat::GRAY => "GRAY",
            PixelFormat::I420 => "I420",
            PixelFormat::NV12 => "NV12",
            PixelFormat::YUYV => "YUYV",
            PixelFormat::UYVY => "UYVY",
        }
    }

    /// Create from FourCC code
    pub fn from_fourcc(fourcc: u32) -> Option<Self> {
        match fourcc {
            x if x == encode_fourcc(b"raw ") => Some(PixelFormat::RGB),
            x if x == encode_fourcc(b"24BG") => Some(PixelFormat::BGR),
            x if x == encode_fourcc(b"ABGR") => Some(PixelFormat::RGBA),
            x if x == encode_fourcc(b"J400") => Some(PixelFormat::GRAY),
            x if x == encode_fourcc(b"I420") => Some(PixelFormat::I420),
            x if x == encode_fourcc(b"NV12") => Some(PixelFormat::NV12),
            x if x == encode_fourcc(b"YUY2") => Some(PixelFormat::YUYV),
            x if x == encode_fourcc(b"UYVY") => Some(PixelFormat::UYVY),
            _ => None,
        }
    }
}

impl std::fmt::Display for PixelFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name())
    }
}

impl Default for PixelFormat {
    fn default() -> Self {
        PixelFormat::RGB
    }
}

/// Encode a 4-byte FourCC code to a u32
pub fn encode_fourcc(bytes: &[u8; 4]) -> u32 {
    u32::from_le_bytes(*bytes)
}

/// Decode a u32 FourCC code to a 4-byte array
pub fn decode_fourcc(code: u32) -> [u8; 4] {
    code.to_le_bytes()
}

/// Convert FourCC code to string
pub fn fourcc_to_string(code: u32) -> String {
    let bytes = decode_fourcc(code);
    String::from_utf8_lossy(&bytes).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fourcc_encoding() {
        assert_eq!(encode_fourcc(b"I420"), 0x30323449);
        assert_eq!(decode_fourcc(0x30323449), *b"I420");
    }

    #[test]
    fn test_frame_size() {
        let width = 1920;
        let height = 1080;

        assert_eq!(PixelFormat::RGB.frame_size(width, height), 1920 * 1080 * 3);
        assert_eq!(PixelFormat::RGBA.frame_size(width, height), 1920 * 1080 * 4);
        assert_eq!(PixelFormat::GRAY.frame_size(width, height), 1920 * 1080);
        assert_eq!(PixelFormat::I420.frame_size(width, height), 1920 * 1080 * 3 / 2);
        assert_eq!(PixelFormat::YUYV.frame_size(width, height), 1920 * 1080 * 2);
    }

    #[test]
    fn test_from_fourcc() {
        assert_eq!(PixelFormat::from_fourcc(encode_fourcc(b"I420")), Some(PixelFormat::I420));
        assert_eq!(PixelFormat::from_fourcc(encode_fourcc(b"NV12")), Some(PixelFormat::NV12));
        assert_eq!(PixelFormat::from_fourcc(0), None);
    }
}
