//! End-to-end test producer for the virtual-camera transport.

#[cfg(target_os = "windows")]
mod platform {
    use std::time::{Duration, Instant};
    use virtualcam::{BackendKind, Camera, PixelFormat};

    #[derive(Debug, Clone, Copy, PartialEq)]
    pub(crate) struct PumpOptions {
        pub(crate) seconds: u64,
        pub(crate) width: u32,
        pub(crate) height: u32,
        pub(crate) fps: f64,
    }

    impl PumpOptions {
        pub(crate) fn parse<I, S>(args: I) -> anyhow::Result<Self>
        where
            I: IntoIterator<Item = S>,
            S: AsRef<str>,
        {
            let values = args
                .into_iter()
                .map(|value| value.as_ref().to_owned())
                .collect::<Vec<_>>();
            anyhow::ensure!(
                values.len() <= 4,
                "usage: vcam-pump [seconds] [width] [height] [fps]"
            );
            Ok(Self {
                seconds: parse_or(&values, 0, 1_u64, "seconds")?,
                width: parse_or(&values, 1, 1280_u32, "width")?,
                height: parse_or(&values, 2, 720_u32, "height")?,
                fps: parse_or(&values, 3, 30.0_f64, "fps")?,
            })
        }
    }

    fn parse_or<T>(values: &[String], index: usize, default: T, name: &str) -> anyhow::Result<T>
    where
        T: std::str::FromStr,
        T::Err: std::fmt::Display,
    {
        values.get(index).map_or(Ok(default), |value| {
            value
                .parse::<T>()
                .map_err(|error| anyhow::anyhow!("invalid {name} '{value}': {error}"))
        })
    }

    pub fn run() -> anyhow::Result<()> {
        let options = PumpOptions::parse(std::env::args().skip(1))?;
        let PumpOptions {
            seconds,
            width,
            height,
            fps,
        } = options;
        let mut camera = Camera::builder(width, height, fps)
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
            height,
        );
        if seconds == 0 {
            println!("Publishing green at {fps} FPS until stopped");
        } else {
            println!("Publishing green at {fps} FPS for {seconds}s");
        }
        let interval = Duration::from_secs_f64(1.0 / fps);
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

#[cfg(all(test, target_os = "windows"))]
mod tests {
    use super::platform::PumpOptions;

    #[test]
    fn pump_arguments_default_to_bounded_720p30() {
        let options = PumpOptions::parse(std::iter::empty::<&str>()).unwrap();
        assert_eq!(options.seconds, 1);
        assert_eq!((options.width, options.height), (1280, 720));
        assert_eq!(options.fps, 30.0);
    }

    #[test]
    fn pump_arguments_accept_1080p60() {
        let options = PumpOptions::parse(["2", "1920", "1080", "60"]).unwrap();
        assert_eq!(options.seconds, 2);
        assert_eq!((options.width, options.height), (1920, 1080));
        assert_eq!(options.fps, 60.0);
    }
}
