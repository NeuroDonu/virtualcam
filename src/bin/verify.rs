//! Media Foundation end-to-end verifier.
//!
//! Enumerates video capture devices, activates the VCam,
//! reads one frame through IMFSourceReader, and validates the NV12 green pump.

#[cfg(target_os = "windows")]
mod platform {
    use std::slice;

    use windows::Win32::Media::MediaFoundation::*;
    use windows::Win32::System::Com::{
        COINIT_MULTITHREADED, CoInitializeEx, CoTaskMemFree, CoUninitialize,
    };
    use windows::core::PWSTR;

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
        let mut captured = None;
        let mut last_flags = 0u32;
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
                captured = Some(sample);
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let sample = captured.ok_or_else(|| {
            anyhow::anyhow!("camera returned no sample after 120 reads; flags=0x{last_flags:08x}")
        })?;
        let buffer = unsafe { sample.ConvertToContiguousBuffer()? };
        let mut data = std::ptr::null_mut();
        let mut len = 0u32;
        unsafe { buffer.Lock(&mut data, None, Some(&mut len))? };
        anyhow::ensure!(
            !data.is_null() && len > 0,
            "camera returned an empty buffer"
        );
        let bytes = unsafe { slice::from_raw_parts(data, len as usize) };
        const WIDTH: usize = 1280;
        const HEIGHT: usize = 960;
        let y_size = WIDTH * HEIGHT;
        let expected = y_size * 3 / 2;
        anyhow::ensure!(
            bytes.len() == expected,
            "unexpected NV12 length: {} != {expected}",
            bytes.len()
        );
        let y_matches = bytes[..y_size].iter().step_by(997).all(|&v| v == 149);
        let uv_matches = bytes[y_size..]
            .chunks_exact(2)
            .step_by(499)
            .all(|uv| uv[0] == 43 && uv[1] == 21);
        let stride = (bytes.len() / 4096).max(1);
        let checksum = bytes
            .iter()
            .step_by(stride)
            .fold(0u64, |sum, &b| sum + b as u64);
        unsafe { buffer.Unlock()? };
        unsafe { activate.ShutdownObject()? };
        anyhow::ensure!(
            y_matches && uv_matches,
            "captured frame is not the pump's BT.601 green NV12 frame"
        );
        println!("PASS: captured exact green NV12 frame, {len} bytes, checksum={checksum}");
        Ok(())
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
