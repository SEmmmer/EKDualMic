use common_types::{AudioFrame, ResidualConfig, VadDecision};

pub struct ResidualSuppressor {
    enabled: bool,
    strength: f32,
    residual_anti_phase_gain: f32,
}

impl ResidualSuppressor {
    pub fn new(config: &ResidualConfig) -> Self {
        Self {
            enabled: config.enabled,
            strength: config.strength.clamp(0.0, 1.0),
            residual_anti_phase_gain: 0.0,
        }
    }

    pub fn process(
        &mut self,
        canceled: &AudioFrame,
        peer_aligned: &AudioFrame,
        local_vad: VadDecision,
        peer_vad: VadDecision,
        coherence: f32,
        estimated_crosstalk_rms: f32,
    ) -> AudioFrame {
        if !self.enabled {
            return canceled.clone();
        }

        let peer_activity = peer_vad.score.clamp(0.0, 1.0);
        let coherence = coherence.clamp(0.0, 1.0);
        let cancellation_confidence = (peer_activity * 0.72 + coherence * 0.28).clamp(0.0, 1.0);
        let residual_ratio = (canceled.rms() / estimated_crosstalk_rms.max(1.0e-4)).clamp(0.0, 1.0);
        let residual_direct_gain = estimate_correlated_gain(canceled, peer_aligned, 1.8);
        let anti_phase_target_gain = if local_vad.is_speech {
            residual_direct_gain * 0.22 * cancellation_confidence
        } else {
            residual_direct_gain
                * (0.85 + 0.65 * self.strength)
                * (0.45 + 0.55 * cancellation_confidence)
        };
        self.update_residual_anti_phase_gain(anti_phase_target_gain);

        if local_vad.is_speech {
            let samples = canceled
                .samples
                .iter()
                .zip(peer_aligned.samples.iter())
                .map(|(sample, peer_sample)| sample - self.residual_anti_phase_gain * peer_sample)
                .collect();
            return AudioFrame::new(
                canceled.sequence,
                canceled.capture_timestamp_us,
                canceled.sample_rate,
                samples,
            );
        }

        let floor_gain = (1.0 - self.strength * 1.35 * cancellation_confidence).clamp(0.02, 1.0);
        let broadband_gain =
            (1.0 - self.strength * 0.38 * cancellation_confidence).clamp(floor_gain, 1.0);
        let gate_threshold = (0.010
            + 0.07 * self.strength * cancellation_confidence * residual_ratio.max(0.35))
        .clamp(0.010, 0.09);

        let samples = canceled
            .samples
            .iter()
            .zip(peer_aligned.samples.iter())
            .map(|sample| {
                let sample = sample.0 - self.residual_anti_phase_gain * sample.1;
                let magnitude = sample.abs();
                if magnitude < gate_threshold {
                    let openness = (magnitude / gate_threshold.max(1.0e-6)).clamp(0.0, 1.0);
                    let gain = floor_gain + (1.0 - floor_gain) * openness.powf(1.8);
                    sample * gain
                } else {
                    sample * broadband_gain
                }
            })
            .collect();

        AudioFrame::new(
            canceled.sequence,
            canceled.capture_timestamp_us,
            canceled.sample_rate,
            samples,
        )
    }

    fn update_residual_anti_phase_gain(&mut self, target_gain: f32) {
        let target_gain = target_gain.clamp(0.0, 1.4);
        self.residual_anti_phase_gain = self.residual_anti_phase_gain * 0.78 + target_gain * 0.22;
    }
}

fn estimate_correlated_gain(frame: &AudioFrame, peer_aligned: &AudioFrame, max_gain: f32) -> f32 {
    let mut cross = 0.0_f64;
    let mut energy = 1.0e-6_f64;
    for (&sample, &peer_sample) in frame.samples.iter().zip(peer_aligned.samples.iter()) {
        cross += (sample as f64) * (peer_sample as f64);
        energy += (peer_sample as f64) * (peer_sample as f64);
    }

    ((cross / energy).max(0.0) as f32).clamp(0.0, max_gain)
}

#[cfg(test)]
mod tests {
    use super::*;
    use common_types::{AudioFrame, now_micros};

    #[test]
    fn suppressor_leaves_local_speech_intact() {
        let mut suppressor = ResidualSuppressor::new(&ResidualConfig::default());
        let canceled = AudioFrame::new(1, now_micros(), 48_000, vec![0.12; 480]);
        let output = suppressor.process(
            &canceled,
            &AudioFrame::new(1, now_micros(), 48_000, vec![0.08; 480]),
            VadDecision {
                score: 0.9,
                is_speech: true,
            },
            VadDecision {
                score: 0.8,
                is_speech: true,
            },
            0.95,
            0.08,
        );
        assert!(output.rms() > canceled.rms() * 0.85);
    }

    #[test]
    fn suppressor_attentuates_low_level_residual_when_local_is_silent() {
        let mut suppressor = ResidualSuppressor::new(&ResidualConfig {
            enabled: true,
            strength: 0.8,
        });
        let canceled = AudioFrame::new(1, now_micros(), 48_000, vec![0.01; 480]);
        let output = suppressor.process(
            &canceled,
            &AudioFrame::new(1, now_micros(), 48_000, vec![0.08; 480]),
            VadDecision {
                score: 0.1,
                is_speech: false,
            },
            VadDecision {
                score: 1.0,
                is_speech: true,
            },
            0.95,
            0.08,
        );
        assert!(output.rms() < canceled.rms() * 0.5);
    }

    #[test]
    fn suppressor_second_pass_anti_phase_reduces_correlated_residual() {
        let mut suppressor = ResidualSuppressor::new(&ResidualConfig {
            enabled: true,
            strength: 0.9,
        });
        let peer = AudioFrame::new(1, now_micros(), 48_000, vec![0.09; 480]);
        let canceled = AudioFrame::new(1, now_micros(), 48_000, vec![0.05; 480]);

        let mut rms = canceled.rms();
        for _ in 0..18 {
            rms = suppressor
                .process(
                    &canceled,
                    &peer,
                    VadDecision {
                        score: 0.1,
                        is_speech: false,
                    },
                    VadDecision {
                        score: 0.95,
                        is_speech: true,
                    },
                    0.98,
                    0.08,
                )
                .rms();
        }

        assert!(
            rms < canceled.rms() * 0.25,
            "expected stronger residual removal, got {rms}"
        );
    }
}
