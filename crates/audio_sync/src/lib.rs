use common_types::{AudioFrame, SyncConfig, SyncReport};
use std::collections::VecDeque;

pub struct SyncAligner {
    history: VecDeque<AudioFrame>,
    max_history_frames: usize,
    frame_ms: usize,
    last_report: SyncReport,
}

impl SyncAligner {
    pub fn new(config: &SyncConfig, frame_ms: usize) -> Self {
        let search_frames = (config.coarse_search_ms as usize / frame_ms.max(1)).max(1);
        let max_history_frames = search_frames + config.jitter_buffer_frames as usize + 1;

        Self {
            history: VecDeque::with_capacity(max_history_frames + 1),
            max_history_frames,
            frame_ms,
            last_report: SyncReport::default(),
        }
    }

    pub fn align(
        &mut self,
        peer_raw: AudioFrame,
        local_raw: &AudioFrame,
    ) -> (AudioFrame, SyncReport) {
        self.history.push_back(peer_raw);
        while self.history.len() > self.max_history_frames {
            self.history.pop_front();
        }

        let mut best_score = -1.0_f32;
        let mut best_delay_frames = 0_usize;
        let mut best_frame = self
            .history
            .back()
            .cloned()
            .unwrap_or_else(|| AudioFrame::zero(local_raw.sequence));

        for (delay_frames, candidate) in self.history.iter().rev().enumerate() {
            let score = local_raw.correlation(candidate);
            if score > best_score {
                best_score = score;
                best_delay_frames = delay_frames;
                best_frame = candidate.clone();
            }
        }

        let report = SyncReport {
            coarse_delay_ms: (best_delay_frames * self.frame_ms) as f32,
            drift_ppm: 0.0,
            coherence: best_score.max(0.0),
        };
        self.last_report = report;

        (
            best_frame
                .with_sequence(local_raw.sequence)
                .with_timestamp(local_raw.capture_timestamp_us),
            report,
        )
    }

    pub fn last_report(&self) -> SyncReport {
        self.last_report
    }
}
