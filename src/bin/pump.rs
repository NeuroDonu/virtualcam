//! End-to-end test producer for the virtual-camera transport.

#[cfg(target_os = "windows")]
mod platform {
    use std::time::{Duration, Instant};
    use virtualcam::{BackendKind, Camera, PixelFormat};

    pub fn run() -> anyhow::Result<()> {
        let (width, height, fps) = (1280u32, 960u32, 30u32);
        // 0 means continuous streaming; pass a positive number for a bounded test.
        let seconds = std::env::args()
            .nth(1)
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(0);
        let mut camera = Camera::builder(width, height, f64::from(fps))
            .format(PixelFormat::NV12)
            .backend(BackendKind::MediaFoundation)
            .build()?;
        let mut frame = vec![0u8; PixelFormat::NV12.frame_size(width, height)];
        let y_size = width as usize * height as usize;
        frame[..y_size].fill(149); // BT.601 green
        for uv in frame[y_size..].chunks_exact_mut(2) {
            uv[0] = 43;
            uv[1] = 21;
        }

        println!(
            "Shared memory ready: {} {}x{} NV12",
            virtualcam::backend::windows_media_foundation::SHM_NAME,
            width,
            height
        );
        if seconds == 0 {
            println!("Publishing green at {fps} FPS until stopped");
        } else {
            println!("Publishing green at {fps} FPS for {seconds}s");
        }
        let interval = Duration::from_nanos(1_000_000_000 / fps as u64);
        let deadline = (seconds > 0).then(|| Instant::now() + Duration::from_secs(seconds));
        let mut next = Instant::now();
        let mut frames = 0u64;
        while deadline.is_none_or(|end| Instant::now() < end) {
            camera.send_native(&frame)?;
            frames += 1;
            next += interval;
            std::thread::sleep(next.saturating_duration_since(Instant::now()));
        }
        println!("Done: {frames} frames");
        Ok(())
    }
}

#[cfg(target_os = "windows")]
fn main() -> anyhow::Result<()> {
    platform::run()
}

#[cfg(not(target_os = "windows"))]
fn main() {
    eprintln!("vcam-pump is available on Windows only");
}
