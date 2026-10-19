//! Call quality: a MOS estimate (an E-model simplification) from
//! RTCP statistics, and its codec description.

use crate::call::CallId;

/// pjmedia's default sound latencies on Windows, plus a local delay (ms).
const LOCAL_DELAY_MS: f32 = 30.0 + 140.0 + 100.0;

/// A call's quality over the last second.
#[derive(Debug, Clone, PartialEq)]
pub struct CallQuality {
    pub call: CallId,
    /// Mean opinion score, 1 (bad) ..= 4.5 (best).
    pub mos: f32,
    /// e.g. "AMR-WB@16kHz 23kbit/s".
    pub codec: String,
}

/// `packets` and `lost` are this
/// interval's received and lost packets (no packets counts as all lost).
pub fn mos(packets: u32, lost: u32, jitter_ms: f32, rtt_ms: f32) -> f32 {
    let loss = if packets == 0 { 1.0 } else { lost as f32 / (packets + lost) as f32 };
    let delay = rtt_ms / 2.0 + LOCAL_DELAY_MS + jitter_ms;
    let id = 0.024 * delay + if delay >= 177.3 { 0.11 * (delay - 177.3) } else { 0.0 };
    let ie = 19.8 * (1.0 + 29.7 * loss).ln();
    let r = 94.2 - id - ie;
    if r < 0.0 {
        1.0
    } else if r > 100.0 {
        4.5
    } else {
        1.0 + 0.035 * r + 7.1e-6 * r * (r - 60.0) * (100.0 - r)
    }
}

/// `codec@kHz kbit/s` plus " Stereo".
pub fn codec_label(codec: &str, clock_rate: u32, avg_bps: u32, channels: u32) -> String {
    let mut label = format!("{codec}@{}kHz", clock_rate / 1000);
    if avg_bps > 0 {
        label.push_str(&format!(" {}kbit/s", avg_bps / 1000));
    }
    if channels == 2 {
        label.push_str(" Stereo");
    }
    label
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mos_follows_the_e_model() {
        // Clean LAN call: delay 270 ms → R ≈ 77.5.
        let clean = mos(50, 0, 0.0, 0.0);
        assert!((clean - 3.93).abs() < 0.01, "{clean}");
        // 10 % loss costs about a point.
        let lossy = mos(45, 5, 0.0, 0.0);
        assert!(lossy < 3.0 && lossy > 2.0, "{lossy}");
        assert!(mos(0, 0, 0.0, 0.0) < 1.1, "nothing received reads as all lost");
        assert!(mos(50, 0, 40.0, 400.0) < clean, "delay lowers the score");
    }

    #[test]
    fn codec_labels_read_well() {
        assert_eq!(codec_label("AMR-WB", 16000, 23850, 1), "AMR-WB@16kHz 23kbit/s");
        assert_eq!(codec_label("opus", 48000, 0, 2), "opus@48kHz Stereo");
    }
}
