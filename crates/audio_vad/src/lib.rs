use common_types::{AudioFrame, VadDecision};

pub struct VoiceActivityDetector {
    threshold: f32,
    smoothing: f32,
    smoothed_score: f32,
    enabled: bool,
}

impl VoiceActivityDetector {
    pub fn new(enabled: bool, threshold: f32, smoothing: f32) -> Self {
        Self {
            threshold,
            smoothing: smoothing.clamp(0.0, 0.999),
            smoothed_score: 0.0,
            enabled,
        }
    }

    pub fn detect(&mut self, frame: &AudioFrame) -> VadDecision {
        if !self.enabled {
            return VadDecision {
                score: 0.0,
                is_speech: false,
            };
        }

        let normalized_energy = (frame.rms() * 12.0).clamp(0.0, 1.0);
        self.smoothed_score =
            self.smoothed_score * self.smoothing + normalized_energy * (1.0 - self.smoothing);

        VadDecision {
            score: self.smoothed_score,
            is_speech: self.smoothed_score >= self.threshold,
        }
    }
}
