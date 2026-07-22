//! Example of sending animated gradient frames

use virtualcam::{Camera, PixelFormat};

fn main() -> virtualcam::error::Result<()> {
    env_logger::init();

    // Create a 720p camera at 60fps
    let mut cam = Camera::builder(1280, 720, 60.0)
        .format(PixelFormat::RGB)
        .print_fps(true)
        .build()?;

    println!("Gradient demo on {}", cam.device());
    println!("Press Ctrl+C to stop");
    println!();

    let width = cam.width() as usize;
    let height = cam.height() as usize;
    let mut frame = vec![0u8; width * height * 3];

    let mut frame_num = 0u64;

    // Set up Ctrl+C handler
    let running = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    let r = running.clone();

    ctrlc::set_handler(move || {
        r.store(false, std::sync::atomic::Ordering::SeqCst);
    })
    .unwrap_or_else(|_| {
        println!("Note: Ctrl+C handler not available");
    });

    // Pre-calculate row colors for speed
    let mut row_colors: Vec<(u8, u8, u8)> = vec![(0, 0, 0); height];

    while running.load(std::sync::atomic::Ordering::SeqCst) {
        // Generate animated gradient - optimize by rows
        let offset = (frame_num as f32 * 0.05) % 1.0;

        // Calculate color per row (much faster than per pixel)
        for (y, color) in row_colors.iter_mut().enumerate() {
            let t = ((y as f32 / height as f32) + offset) % 1.0;
            *color = hsv_to_rgb(t, 0.9, 1.0);
        }

        // Fill frame by rows
        for (y, &(r, g, b)) in row_colors.iter().enumerate() {
            let row_start = y * width * 3;
            for x in 0..width {
                let idx = row_start + x * 3;
                frame[idx] = r;
                frame[idx + 1] = g;
                frame[idx + 2] = b;
            }
        }

        cam.send(&frame)?;
        cam.sleep_until_next_frame();

        frame_num += 1;

        // Print stats every second
        if frame_num % 60 == 0 {
            println!("Frame {}: {:.1} FPS", cam.frames_sent(), cam.current_fps());
        }
    }

    println!("\nStopped. Total frames: {}", cam.frames_sent());

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
