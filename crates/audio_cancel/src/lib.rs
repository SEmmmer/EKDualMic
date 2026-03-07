use common_types::{AudioFrame, CancelConfig, CancelReport};

pub struct NlmsCanceller {
    weights: Vec<f32>,
    history: Vec<f32>,
    cursor: usize,
    step_size: f32,
    leakage: f32,
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
        self.last_report = CancelReport::default();
    }

    pub fn process(
        &mut self,
        local_raw: &AudioFrame,
        peer_aligned: &AudioFrame,
    ) -> (AudioFrame, CancelReport) {
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

            let error = local_sample - predicted;
            output.push(error);
            estimate.push(predicted);

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
}
