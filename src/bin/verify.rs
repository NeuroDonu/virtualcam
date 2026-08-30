//! Media Foundation end-to-end verifier.
//!
//! Enumerates video capture devices, activates the VCam,
//! reads through IMFSourceReader until the startup transition settles, and
//! validates the NV12 green pump.

#[cfg(target_os = "windows")]
mod platform {
    use std::slice;

    use windows::Win32::Media::MediaFoundation::*;
    use windows::Win32::System::Com::{
        COINIT_MULTITHREADED, CoInitializeEx, CoTaskMemFree, CoUninitialize,
    };
    use windows::core::PWSTR;

    fn is_expected_green_nv12(bytes: &[u8], width: usize, height: usize) -> bool {
        let y_size = width * height;
        let expected = y_size * 3 / 2;
        bytes.len() == expected
            && bytes[..y_size].iter().all(|&value| value == 149)
            && bytes[y_size..]
                .chunks_exact(2)
                .all(|uv| uv[0] == 43 && uv[1] == 21)
    }

    fn unpack_u32_pair(value: u64) -> (u32, u32) {
        ((value >> 32) as u32, value as u32)
    }

    fn media_type_geometry(media_type: &IMFMediaType) -> anyhow::Result<(u32, u32, u32, u32)> {
        let frame_size = unsafe { media_type.GetUINT64(&MF_MT_FRAME_SIZE)? };
        let frame_rate = unsafe { media_type.GetUINT64(&MF_MT_FRAME_RATE)? };
        let (width, height) = unpack_u32_pair(frame_size);
        let (fps_numerator, fps_denominator) = unpack_u32_pair(frame_rate);
        anyhow::ensure!(
            width > 0 && height > 0 && fps_numerator > 0 && fps_denominator > 0,
            "invalid negotiated media type {width}x{height} @ {fps_numerator}/{fps_denominator}"
        );
        Ok((width, height, fps_numerator, fps_denominator))
    }

    pub fn run() -> anyhow::Result<()> {
        unsafe { CoInitializeEx(None, COINIT_MULTITHREADED).ok()? };
        unsafe { MFStartup(MF_VERSION, MFSTARTUP_FULL)? };
        let result = verify();
        unsafe {
            let _ = MFShutdown();
            CoUninitialize();
        }
        result
    }

    fn verify() -> anyhow::Result<()> {
        let mut attrs = None;
        unsafe { MFCreateAttributes(&mut attrs, 1)? };
        let attrs = attrs.ok_or_else(|| anyhow::anyhow!("MFCreateAttributes returned null"))?;
        unsafe {
            attrs.SetGUID(
                &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE,
                &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID,
            )?;
        }

        let mut raw = std::ptr::null_mut();
        let mut count = 0u32;
        unsafe { MFEnumDeviceSources(&attrs, &mut raw, &mut count)? };
        anyhow::ensure!(
            !raw.is_null() && count > 0,
            "no Media Foundation cameras enumerated"
        );
        let activates = unsafe { slice::from_raw_parts(raw, count as usize) };

        let mut selected = None;
        for activate in activates.iter().flatten() {
            let mut wide = PWSTR::null();
            let mut chars = 0u32;
            unsafe {
                activate.GetAllocatedString(
                    &MF_DEVSOURCE_ATTRIBUTE_FRIENDLY_NAME,
                    &mut wide,
                    &mut chars,
                )?;
            }
            let name = unsafe { wide.to_string()? };
            unsafe { CoTaskMemFree(Some(wide.as_ptr().cast())) };
            println!("camera: {name}");
            if name
                .get(..4)
                .is_some_and(|prefix| prefix.eq_ignore_ascii_case("VCam"))
            {
                selected = Some(activate.clone());
            }
        }
        unsafe { CoTaskMemFree(Some(raw.cast())) };
        let activate = selected.ok_or_else(|| anyhow::anyhow!("VCam camera not found"))?;

        let source: IMFMediaSource = unsafe { activate.ActivateObject()? };
        let reader = unsafe { MFCreateSourceReaderFromMediaSource(&source, None)? };
        let media_type =
            unsafe { reader.GetCurrentMediaType(MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32)? };
        let (width, height, fps_numerator, fps_denominator) = media_type_geometry(&media_type)?;
        println!("negotiated: {width}x{height} @ {fps_numerator}/{fps_denominator} FPS");
        let mut last_flags = 0u32;
        let mut samples_seen = 0u32;
        for _ in 0..120 {
            let mut sample = None;
            let mut flags = 0u32;
            unsafe {
                reader.ReadSample(
                    MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32,
                    0,
                    None,
                    Some(&mut flags),
                    None,
                    Some(&mut sample),
                )?;
            }
            last_flags = flags;
            if flags & MF_SOURCE_READERF_ERROR.0 as u32 != 0 {
                anyhow::bail!("source reader reported an error; flags=0x{flags:08x}");
            }
            if let Some(sample) = sample {
                samples_seen += 1;
                let buffer = unsafe { sample.ConvertToContiguousBuffer()? };
                let mut data = std::ptr::null_mut();
                let mut len = 0u32;
                unsafe { buffer.Lock(&mut data, None, Some(&mut len))? };
                anyhow::ensure!(
                    !data.is_null() && len > 0,
                    "camera returned an empty buffer"
                );
                let bytes = unsafe { slice::from_raw_parts(data, len as usize) };
                let matches = is_expected_green_nv12(bytes, width as usize, height as usize);
                let stride = (bytes.len() / 4096).max(1);
                let checksum = bytes
                    .iter()
                    .step_by(stride)
                    .fold(0u64, |sum, &byte| sum + u64::from(byte));
                unsafe { buffer.Unlock()? };
                if matches {
                    unsafe { activate.ShutdownObject()? };
                    println!(
                        "PASS: captured exact green NV12 frame, {len} bytes, checksum={checksum}, samples={samples_seen}"
                    );
                    return Ok(());
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        unsafe { activate.ShutdownObject()? };
        anyhow::bail!(
            "camera returned no expected green NV12 frame after 120 reads; samples={samples_seen}, flags=0x{last_flags:08x}"
        )
    }

    #[cfg(test)]
    mod tests {
        use super::is_expected_green_nv12;

        #[test]
        fn transitional_frame_is_rejected_before_exact_gpu_frame_is_accepted() {
            const WIDTH: usize = 1280;
            const HEIGHT: usize = 960;
            let y_size = WIDTH * HEIGHT;
            let mut expected = vec![149_u8; y_size * 3 / 2];
            for uv in expected[y_size..].chunks_exact_mut(2) {
                uv[0] = 43;
                uv[1] = 21;
            }
            let mut transitional = expected.clone();
            transitional[997] = 16;

            assert!(!is_expected_green_nv12(&transitional, WIDTH, HEIGHT));
            assert!(is_expected_green_nv12(&expected, WIDTH, HEIGHT));
            assert!(!is_expected_green_nv12(
                &expected[..expected.len() - 1],
                WIDTH,
                HEIGHT
            ));
        }

        #[test]
        fn green_nv12_validation_uses_negotiated_geometry() {
            let width = 1920usize;
            let height = 1080usize;
            let y_size = width * height;
            let mut frame = vec![149_u8; y_size * 3 / 2];
            for uv in frame[y_size..].chunks_exact_mut(2) {
                uv[0] = 43;
                uv[1] = 21;
            }

            assert!(is_expected_green_nv12(&frame, width, height));
            assert!(!is_expected_green_nv12(&frame, 1280, 720));
        }
    }
}

#[cfg(target_os = "windows")]
fn main() -> anyhow::Result<()> {
    platform::run()
}

#[cfg(not(target_os = "windows"))]
fn main() {
    eprintln!("vcam-verify is available on Windows only");
}
