//! Prints the input devices the settings window offers, with the id the config stores. With
//! `listen [id]` it records two seconds from a device and prints how loud it was.

use std::time::Duration;

use tokativ_audio::{Levels, Options, Recorder, Timing};

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    if args.next().as_deref() != Some("listen") {
        for device in tokativ_audio::input_devices() {
            println!("{:<50} {}", device.name, device.id);
        }
        return Ok(());
    }
    let timing = Timing {
        threshold: 0.5,
        silence_ms: 500,
        max_segment_s: 20,
        preroll_ms: 200,
    };
    let options = Options {
        device: args.next(),
        timing,
        progress: None,
    };
    let levels = Levels::default();
    let recorder = Recorder::start(options, None, levels.clone(), |_| {})?;
    let mut loudest = 0f32;
    for _ in 0..40 {
        std::thread::sleep(Duration::from_millis(50));
        loudest = loudest.max(levels.peak());
    }
    recorder.cancel();
    println!("loudest band level over two seconds: {loudest:.2}");
    Ok(())
}
