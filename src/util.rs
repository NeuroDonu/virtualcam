//! Utility functions and types

use std::time::{Duration, Instant};

/// FPS counter using exponential moving average
pub struct FpsCounter {
    last_time: Instant,
    avg_delta: f64,
    smoothing: f64,
}

impl FpsCounter {
    /// Create a new FPS counter with initial target FPS
    pub fn new(initial_fps: f64) -> Self {
        Self {
            last_time: Instant::now(),
            avg_delta: 1.0 / initial_fps,
            smoothing: 0.2,
        }
    }

    /// Record a frame and update the average
    pub fn tick(&mut self) {
        let now = Instant::now();
        let delta = now.duration_since(self.last_time).as_secs_f64();
        self.last_time = now;

        // Exponential moving average
        self.avg_delta += (delta - self.avg_delta) * self.smoothing;
    }

    /// Get the current average FPS
    pub fn fps(&self) -> f64 {
        if self.avg_delta > 0.0 {
            1.0 / self.avg_delta
        } else {
            0.0
        }
    }

    /// Get the average frame delta in seconds
    pub fn avg_delta(&self) -> f64 {
        self.avg_delta
    }

    /// Set the smoothing factor (0.0 - 1.0)
    pub fn set_smoothing(&mut self, smoothing: f64) {
        self.smoothing = smoothing.clamp(0.0, 1.0);
    }
}

impl Default for FpsCounter {
    fn default() -> Self {
        Self::new(30.0)
    }
}

/// Frame timing controller for maintaining target FPS
pub struct FrameTimer {
    target_fps: f64,
    frame_duration: Duration,
    last_frame: Instant,
    extra_time: f64,
}

impl FrameTimer {
    /// Create a new frame timer with target FPS
    pub fn new(target_fps: f64) -> Self {
        Self {
            target_fps,
            frame_duration: Duration::from_secs_f64(1.0 / target_fps),
            last_frame: Instant::now(),
            extra_time: 0.0,
        }
    }

    /// Mark the start of a new frame
    pub fn begin_frame(&mut self) {
        self.last_frame = Instant::now();
    }

    /// Sleep until the next frame is due
    /// Returns the actual sleep duration
    pub fn sleep_until_next_frame(&mut self) -> Duration {
        let elapsed = self.last_frame.elapsed();
        let target = self.frame_duration.as_secs_f64();

        let sleep_time = target - elapsed.as_secs_f64() - self.extra_time;

        if sleep_time > 0.0 {
            let sleep_duration = Duration::from_secs_f64(sleep_time);
            std::thread::sleep(sleep_duration);

            // Adjust extra time based on actual sleep accuracy
            let actual_elapsed = self.last_frame.elapsed().as_secs_f64();
            let overshoot = actual_elapsed - target;
            self.extra_time += overshoot * 0.1;
            self.extra_time = self.extra_time.clamp(-target * 0.5, target * 0.5);

            sleep_duration
        } else {
            // We're behind schedule, adjust
            self.extra_time = (self.extra_time - 0.001).max(0.0);
            Duration::ZERO
        }
    }

    /// Get the target FPS
    pub fn target_fps(&self) -> f64 {
        self.target_fps
    }

    /// Set a new target FPS
    pub fn set_target_fps(&mut self, fps: f64) {
        self.target_fps = fps;
        self.frame_duration = Duration::from_secs_f64(1.0 / fps);
    }

    /// Get time since last frame
    pub fn elapsed(&self) -> Duration {
        self.last_frame.elapsed()
    }
}

/// Get current timestamp in nanoseconds (for frame timing)
#[cfg(windows)]
pub fn get_timestamp_ns() -> u64 {
    use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};

    let mut counter = 0i64;
    let mut frequency = 0i64;

    unsafe {
        QueryPerformanceCounter(&mut counter).ok();
        QueryPerformanceFrequency(&mut frequency).ok();
    }

    if frequency > 0 {
        ((counter as u128 * 1_000_000_000) / frequency as u128) as u64
    } else {
        0
    }
}

#[cfg(not(windows))]
pub fn get_timestamp_ns() -> u64 {
    use std::time::SystemTime;
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fps_counter() {
        let mut counter = FpsCounter::new(30.0);
        assert!((counter.fps() - 30.0).abs() < 0.1);

        // Simulate some frames
        for _ in 0..10 {
            std::thread::sleep(Duration::from_millis(33));
            counter.tick();
        }

        // FPS should be close to 30
        let fps = counter.fps();
        assert!(fps > 20.0 && fps < 40.0, "FPS was {}", fps);
    }

    #[test]
    fn test_frame_timer() {
        let mut timer = FrameTimer::new(60.0);
        assert_eq!(timer.target_fps(), 60.0);

        timer.begin_frame();
        std::thread::sleep(Duration::from_millis(5));
        let slept = timer.sleep_until_next_frame();

        // Should have slept for roughly (16.67 - 5) ms = ~11.67 ms
        assert!(slept.as_millis() > 5);
    }
}
