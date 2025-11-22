//! Simple example of sending solid color frames

use virtualcam::{Camera, PixelFormat};

fn main() -> virtualcam::error::Result<()> {
    env_logger::init();

    println!("Available backends:");
    for backend in virtualcam::available_backends() {
        println!("  - {}: {}", backend.name, backend.description);
    }
    println!();

    // Create a camera
    let mut cam = Camera::builder(1280, 720, 30.0)
        .format(PixelFormat::RGB)
        .print_fps(true)
        .build()?;

    println!("Camera opened!");
    println!("  Backend: {}", cam.backend());
    println!("  Device: {}", cam.device());
    println!("  Size: {}x{}", cam.width(), cam.height());
    println!("  FPS: {}", cam.fps());
    println!("  Format: {} -> {}", cam.format(), cam.native_format());
    println!();

    // Create frame buffer
    let mut frame = vec![0u8; cam.width() as usize * cam.height() as usize * 3];

    // Send color cycling frames for 10 seconds
    let duration = std::time::Duration::from_secs(10);
    let start = std::time::Instant::now();

    println!("Sending frames for {} seconds...", duration.as_secs());

    while start.elapsed() < duration {
        // Calculate color based on time
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

    println!("\nDone! Sent {} frames", cam.frames_sent());
    println!("Average FPS: {:.1}", cam.current_fps());

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
