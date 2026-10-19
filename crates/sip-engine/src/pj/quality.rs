//! Samples call quality once a second. Engine thread.

use std::collections::HashMap;

use super::calls;
use crate::call::{CallId, CallState, MediaState};
use crate::quality::{CallQuality, codec_label, mos};

/// Received and lost packet counters at the last sample, per call.
#[derive(Default)]
pub struct QualityMonitor {
    last: HashMap<CallId, (u32, u32)>,
}

impl QualityMonitor {
    /// Quality of every answered call with flowing audio (held calls are
    /// skipped).
    pub fn sample(&mut self) -> Vec<CallQuality> {
        let mut out = Vec::new();
        let ids = calls::call_ids();
        self.last.retain(|id, _| ids.contains(id));
        for id in ids {
            let Some(info) = calls::call_info(id) else { continue };
            if info.state != CallState::Confirmed || info.media != MediaState::Active || info.connected_for.is_zero() {
                continue;
            }
            let Some(s) = super::debug::stream_stats(id) else { continue };
            let (packets, lost) = self.last.insert(id, (s.rx_packets, s.rx_lost)).unwrap_or((0, 0));
            out.push(CallQuality {
                call: id,
                mos: mos(s.rx_packets.saturating_sub(packets), s.rx_lost.saturating_sub(lost), s.rx_jitter_ms, s.rtt_ms),
                codec: codec_label(&s.codec, s.clock_rate, s.avg_bps, s.channels),
            });
        }
        out
    }
}
