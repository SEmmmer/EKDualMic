use common_types::{AudioFrame, CancelConfig, CancelReport};

pub struct NlmsCanceller {
    weights: Vec<f32>,
    history: Vec<f32>,
    cursor: usize,
    step_size: f32,
    leakage: f32,
    anti_phase_enabled: bool,
    anti_phase_gain: f32,
    anti_phase_max_gain: f32,
    anti_phase_smoothing: f32,
    update_frozen: bool,
    last_report: CancelReport,
}

impl NlmsCanceller {
    pub fn new(config: &CancelConfig) -> Self {
        let filter_length = config.filter_length.max(1);
        Self {
            weights: vec![0.0; filter_length],
            history: vec![0.0; filter_length],
            cursor: 0,
            step_size: config.step_size,
            leakage: config.leakage,
            anti_phase_enabled: config.anti_phase_enabled,
            anti_phase_gain: 0.0,
            anti_phase_max_gain: config.anti_phase_max_gain.clamp(0.0, 2.0),
            anti_phase_smoothing: config.anti_phase_smoothing.clamp(0.0, 0.999),
            update_frozen: false,
            last_report: CancelReport::default(),
        }
    }

    pub fn set_update_frozen(&mut self, frozen: bool) {
        self.update_frozen = frozen;
    }

    pub fn freeze_update(&mut self) {
        self.update_frozen = true;
    }

    pub fn reset(&mut self) {
        self.weights.fill(0.0);
        self.history.fill(0.0);
        self.cursor = 0;
        self.anti_phase_gain = 0.0;
        self.last_report = CancelReport::default();
    }

    pub fn process(
        &mut self,
        local_raw: &AudioFrame,
        peer_aligned: &AudioFrame,
    ) -> (AudioFrame, CancelReport) {
        let frame_direct_gain = if self.anti_phase_enabled {
            estimate_direct_gain(local_raw, peer_aligned, self.anti_phase_max_gain)
        } else {
            0.0
        };
        self.update_anti_phase_gain(frame_direct_gain);
        let anti_phase_gain = if self.anti_phase_enabled {
            self.anti_phase_gain
        } else {
            0.0
        };

        let mut output = Vec::with_capacity(local_raw.samples.len());
        let mut estimate = Vec::with_capacity(local_raw.samples.len());

        for (&local_sample, &peer_sample) in
            local_raw.samples.iter().zip(peer_aligned.samples.iter())
        {
            self.history[self.cursor] = peer_sample;
            self.cursor = (self.cursor + 1) % self.history.len();

            let mut predicted = 0.0_f32;
            let mut energy = 1.0e-6_f32;
            let mut history_index = self.cursor;

            for (tap, weight) in self.weights.iter().enumerate() {
                history_index = history_index
                    .checked_sub(1)
                    .unwrap_or(self.history.len() - 1);
                let reference = self.history[history_index];
                predicted += *weight * reference;
                energy += reference * reference;
                if tap + 1 == self.history.len() {
                    break;
                }
            }

            let direct_prediction = anti_phase_gain * peer_sample;
            let total_prediction = predicted + direct_prediction;
            let error = local_sample - total_prediction;
            output.push(error);
            estimate.push(total_prediction);

            if !self.update_frozen {
                let adaptation = self.step_size / energy;
                let mut update_index = self.cursor;
                for weight in &mut self.weights {
                    update_index = update_index
                        .checked_sub(1)
                        .unwrap_or(self.history.len() - 1);
                    let reference = self.history[update_index];
                    *weight = (1.0 - self.leakage) * *weight + adaptation * error * reference;
                }
            }
        }

        let output_frame = AudioFrame::new(
            local_raw.sequence,
            local_raw.capture_timestamp_us,
            local_raw.sample_rate,
            output,
        );
        let estimate_frame = AudioFrame::new(
            local_raw.sequence,
            local_raw.capture_timestamp_us,
            local_raw.sample_rate,
            estimate,
        );

        let report = CancelReport {
            filter_frozen: self.update_frozen,
            estimated_crosstalk_rms: estimate_frame.rms(),
            output_rms: output_frame.rms(),
        };
        self.last_report = report;

        (output_frame, report)
    }

    pub fn last_report(&self) -> CancelReport {
        self.last_report
    }

    fn update_anti_phase_gain(&mut self, target_gain: f32) {
        if !self.anti_phase_enabled {
            self.anti_phase_gain = 0.0;
            return;
        }

        let target_gain = target_gain.clamp(0.0, self.anti_phase_max_gain);
        let smoothed_gain = self.anti_phase_gain * self.anti_phase_smoothing
            + target_gain * (1.0 - self.anti_phase_smoothing);

        if self.update_frozen && smoothed_gain > self.anti_phase_gain {
            return;
        }

        self.anti_phase_gain = smoothed_gain;
    }
}

fn estimate_direct_gain(local_raw: &AudioFrame, peer_aligned: &AudioFrame, max_gain: f32) -> f32 {
    let mut cross = 0.0_f64;
    let mut energy = 1.0e-6_f64;

    for (&local_sample, &peer_sample) in local_raw.samples.iter().zip(peer_aligned.samples.iter()) {
        cross += (local_sample as f64) * (peer_sample as f64);
        energy += (peer_sample as f64) * (peer_sample as f64);
    }

    let gain = (cross / energy).max(0.0) as f32;
    gain.clamp(0.0, max_gain)
}

#[cfg(test)]
mod tests {
    use super::*;
    use common_types::now_micros;

    #[test]
    fn direct_gain_estimator_tracks_scaled_peer() {
        let local = AudioFrame::new(1, now_micros(), 48_000, vec![0.3; 480]);
        let peer = AudioFrame::new(1, now_micros(), 48_000, vec![0.2; 480]);
        let gain = estimate_direct_gain(&local, &peer, 2.0);
        assert!(
            (gain - 1.5).abs() < 0.02,
            "expected gain near 1.5, got {gain}"
        );
    }

    #[test]
    fn anti_phase_path_reduces_simple_leakage_after_convergence() {
        let mut canceller = NlmsCanceller::new(&CancelConfig::default());
        let local = AudioFrame::new(1, now_micros(), 48_000, vec![0.24; 480]);
        let peer = AudioFrame::new(1, now_micros(), 48_000, vec![0.2; 480]);

        let mut final_output_rms = 1.0;
        for _ in 0..24 {
            let (output, _) = canceller.process(&local, &peer);
            final_output_rms = output.rms();
        }

        assert!(
            final_output_rms < local.rms() * 0.4,
            "expected anti-phase + NLMS to reduce leakage, got {final_output_rms}"
        );
    }
}
