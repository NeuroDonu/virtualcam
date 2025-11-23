//! Virtual Camera Demo
//!
//! Run with: cargo run --release

use virtualcam::{Camera, PixelFormat};

fn main() -> virtualcam::error::Result<()> {
    println!("=== Virtual Camera for Rust ===\n");

    // Show available backends
    let backends = virtualcam::available_backends();

    if backends.is_empty() {
        println!("No backends available!");
        println!("\nOn Windows: Install OBS Studio or Unity Video Capture");
        println!("On Linux: Run 'sudo modprobe v4l2loopback'");
        return Ok(());
    }

    println!("Available backends:");
    for backend in &backends {
        println!("  - {} ({})", backend.name, backend.description);
        println!("    Native format: {}", backend.native_format);
    }
    println!();

    // Create camera
    let mut cam = Camera::builder(1280, 720, 30.0)
        .format(PixelFormat::RGB)
        .print_fps(true)
        .build()?;

    println!("Camera started:");
    println!("  Backend: {}", cam.backend());
    println!("  Device: {}", cam.device());
    println!("  Resolution: {}x{}", cam.width(), cam.height());
    println!("  FPS: {}", cam.fps());
    println!("  Format: {} -> {}", cam.format(), cam.native_format());
    println!();

    // Create frame buffer
    let frame_size = cam.width() as usize * cam.height() as usize * 3;
    let mut frame = vec![0u8; frame_size];

    // Send frames for 60 seconds
    let duration = std::time::Duration::from_secs(60);
    let start = std::time::Instant::now();

    println!("Sending color cycling frames for {} seconds...", duration.as_secs());
    println!("Open your camera app to see the output!\n");

    while start.elapsed() < duration {
        // Calculate rainbow color based on time
        let t = start.elapsed().as_secs_f32();
        let hue = (t * 0.1) % 1.0;
        let (r, g, b) = hsv_to_rgb(hue, 1.0, 1.0);

        // Fill frame with color
        for pixel in frame.chunks_exact_mut(3) {
            pixel[0] = r;
            pixel[1] = g;
            pixel[2] = b;
        }

        // Send frame
        cam.send(&frame)?;
        cam.sleep_until_next_frame();
    }

    println!("Done!");
    println!("  Frames sent: {}", cam.frames_sent());
    println!("  Average FPS: {:.1}", cam.current_fps());

    Ok(())
}

fn hsv_to_rgb(h: f32, s: f32, v: f32) -> (u8, u8, u8) {
    let i = (h * 6.0) as i32;
    let f = h * 6.0 - i as f32;
    let p = v * (1.0 - s);
    let q = v * (1.0 - f * s);
    let t = v * (1.0 - (1.0 - f) * s);

    let (r, g, b) = match i % 6 {
        0 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        _ => (v, p, q),
    };

    ((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8)
}
