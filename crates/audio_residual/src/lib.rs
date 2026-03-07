use common_types::{AudioFrame, ResidualConfig, VadDecision};

pub struct ResidualSuppressor {
    enabled: bool,
    strength: f32,
}

impl ResidualSuppressor {
    pub fn new(config: &ResidualConfig) -> Self {
        Self {
            enabled: config.enabled,
            strength: config.strength.clamp(0.0, 1.0),
        }
    }

    pub fn process(
        &mut self,
        canceled: &AudioFrame,
        local_vad: VadDecision,
        peer_vad: VadDecision,
    ) -> AudioFrame {
        if !self.enabled {
            return canceled.clone();
        }

        let attenuation = if local_vad.is_speech {
            1.0
        } else {
            1.0 - (self.strength * 0.4 * peer_vad.score.clamp(0.0, 1.0))
        };
        let attenuation = attenuation.clamp(0.6, 1.0);

        let samples = canceled
            .samples
            .iter()
            .map(|sample| {
                if sample.abs() < 0.05 {
                    sample * attenuation
                } else {
                    *sample
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
}
