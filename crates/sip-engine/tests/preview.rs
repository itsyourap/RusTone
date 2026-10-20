//! The Settings camera preview survives choosing another camera: that
//! restarts the engine, and the preview must come back with the new camera
//! (it used to stay at "Starting the camera…"). Uses pjmedia's two test
//! cameras; runs only when asked:
//! `cargo test -p sip-engine --test preview -- --ignored`.
//! (A separate test binary, because pjsua is a singleton.)

use std::sync::mpsc;
use std::time::{Duration, Instant};

use sip_engine::{Command, Engine, EngineConfig, Event, VideoConfig, VideoSource};

fn config(camera: &str) -> EngineConfig {
    EngineConfig { null_audio: true, video: VideoConfig { enabled: true, camera: camera.into(), bitrate_kbps: 0 }, ..EngineConfig::default() }
}

#[test]
#[ignore = "uses the video stack"]
fn the_preview_comes_back_after_switching_camera() {
    let (tx, rx) = mpsc::channel();
    let engine = Engine::spawn(config("Colorbar generator"), move |e| {
        let _ = tx.send(e);
    })
    .expect("engine starts");
    let started = |what: &str| {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            if let Ok(Event::Started) = rx.recv_timeout(Duration::from_millis(200)) {
                return;
            }
        }
    };
    started("the engine");
    let frames = engine.video();
    let preview_frames = |what: &str| {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if frames.latest(VideoSource::Preview).is_some_and(|f| f.seq > 5) {
                return;
            }
            assert!(Instant::now() < deadline, "no preview frames {what}");
            std::thread::sleep(Duration::from_millis(100));
        }
    };

    engine.send(Command::SetPreview(true));
    preview_frames("with the first camera");

    // Another camera restarts the engine.
    engine.send(Command::Restart(config("Colorbar-active")));
    started("the restarted engine");
    preview_frames("after switching camera");

    engine.send(Command::SetPreview(false));
    std::thread::sleep(Duration::from_millis(500));
    assert!(frames.latest(VideoSource::Preview).is_none(), "the preview stops");
    drop(engine);
}
