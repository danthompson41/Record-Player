use rp_core::{CrossfaderAssign, CrossfaderCurve};
use std::f32::consts::PI;

/// Per-channel strip with gain and EQ
pub struct ChannelStrip {
    /// Input trim/gain
    pub trim: f32,
    /// Fader level (0.0 to 1.0)
    pub gain: f32,
    /// 3-band EQ: low, mid, high (-1.0 to 1.0, 0.0 = flat)
    pub eq_low: f32,
    pub eq_mid: f32,
    pub eq_high: f32,
    /// Crossfader assignment
    pub crossfader_assign: CrossfaderAssign,
    /// Peak meters (updated during render)
    pub peak_left: f32,
    pub peak_right: f32,
}

impl Default for ChannelStrip {
    fn default() -> Self {
        Self {
            trim: 1.0,
            gain: 1.0,
            eq_low: 0.0,
            eq_mid: 0.0,
            eq_high: 0.0,
            crossfader_assign: CrossfaderAssign::Thru,
            peak_left: 0.0,
            peak_right: 0.0,
        }
    }
}

impl ChannelStrip {
    /// Calculate the effective gain including EQ (simplified)
    pub fn effective_gain(&self) -> f32 {
        self.trim * self.gain
    }

    /// Update peak meters
    pub fn update_meters(&mut self, left: f32, right: f32) {
        self.peak_left = self.peak_left.max(left.abs());
        self.peak_right = self.peak_right.max(right.abs());
    }

    /// Decay peak meters (call periodically)
    pub fn decay_meters(&mut self, decay: f32) {
        self.peak_left *= decay;
        self.peak_right *= decay;
    }
}

/// Main mixer with crossfader
pub struct Mixer {
    /// Channel strips for each deck
    pub channels: [ChannelStrip; 4],
    /// Crossfader position (-1.0 = A, 0.0 = center, 1.0 = B)
    pub crossfader: f32,
    /// Crossfader curve type
    pub crossfader_curve: CrossfaderCurve,
    /// Master gain
    pub master_gain: f32,
    /// Master peak meters
    pub master_peak_left: f32,
    pub master_peak_right: f32,
}

impl Default for Mixer {
    fn default() -> Self {
        Self {
            channels: [
                ChannelStrip {
                    crossfader_assign: CrossfaderAssign::A,
                    ..Default::default()
                },
                ChannelStrip {
                    crossfader_assign: CrossfaderAssign::B,
                    ..Default::default()
                },
                ChannelStrip::default(),
                ChannelStrip::default(),
            ],
            crossfader: 0.0,
            crossfader_curve: CrossfaderCurve::ConstantPower,
            master_gain: 1.0,
            master_peak_left: 0.0,
            master_peak_right: 0.0,
        }
    }
}

impl Mixer {
    /// Calculate crossfade gains for A and B sides
    pub fn crossfade_gains(&self) -> (f32, f32) {
        let pos = (self.crossfader + 1.0) / 2.0; // Convert to 0.0-1.0 range

        match self.crossfader_curve {
            CrossfaderCurve::Linear => {
                // Linear crossfade
                (1.0 - pos, pos)
            }
            CrossfaderCurve::ConstantPower => {
                // Equal power (constant power) crossfade
                let angle = pos * PI / 2.0;
                (angle.cos(), angle.sin())
            }
            CrossfaderCurve::Scratch => {
                // Sharp cut - useful for scratching
                let a = if pos < 0.1 { 1.0 } else { 0.0 };
                let b = if pos > 0.9 { 1.0 } else { 0.0 };
                (a, b)
            }
            CrossfaderCurve::Transition => {
                // Smooth S-curve for mixing
                let s = pos * pos * (3.0 - 2.0 * pos); // Smoothstep
                (1.0 - s, s)
            }
        }
    }

    /// Get the crossfade multiplier for a specific channel
    pub fn channel_crossfade_gain(&self, channel: usize) -> f32 {
        let (gain_a, gain_b) = self.crossfade_gains();

        match self.channels[channel].crossfader_assign {
            CrossfaderAssign::A => gain_a,
            CrossfaderAssign::B => gain_b,
            CrossfaderAssign::Thru => 1.0,
        }
    }

    /// Mix multiple deck outputs into a stereo master output
    pub fn mix(&mut self, deck_outputs: &[[f32; 2]; 4], output: &mut [f32; 2]) {
        let mut left = 0.0f32;
        let mut right = 0.0f32;

        // Pre-calculate crossfade gains to avoid borrow issues
        let (gain_a, gain_b) = self.crossfade_gains();
        let crossfade_gains: [f32; 4] = [
            match self.channels[0].crossfader_assign {
                CrossfaderAssign::A => gain_a,
                CrossfaderAssign::B => gain_b,
                CrossfaderAssign::Thru => 1.0,
            },
            match self.channels[1].crossfader_assign {
                CrossfaderAssign::A => gain_a,
                CrossfaderAssign::B => gain_b,
                CrossfaderAssign::Thru => 1.0,
            },
            match self.channels[2].crossfader_assign {
                CrossfaderAssign::A => gain_a,
                CrossfaderAssign::B => gain_b,
                CrossfaderAssign::Thru => 1.0,
            },
            match self.channels[3].crossfader_assign {
                CrossfaderAssign::A => gain_a,
                CrossfaderAssign::B => gain_b,
                CrossfaderAssign::Thru => 1.0,
            },
        ];

        for (i, deck_out) in deck_outputs.iter().enumerate() {
            let channel = &mut self.channels[i];
            let gain = channel.effective_gain() * crossfade_gains[i];

            let sample_l = deck_out[0] * gain;
            let sample_r = deck_out[1] * gain;

            channel.update_meters(sample_l, sample_r);

            left += sample_l;
            right += sample_r;
        }

        // Apply master gain
        left *= self.master_gain;
        right *= self.master_gain;

        // Soft clip to prevent harsh clipping
        left = soft_clip(left);
        right = soft_clip(right);

        // Update master meters
        self.master_peak_left = self.master_peak_left.max(left.abs());
        self.master_peak_right = self.master_peak_right.max(right.abs());

        output[0] = left;
        output[1] = right;
    }

    /// Decay all meters
    pub fn decay_meters(&mut self, decay: f32) {
        for channel in &mut self.channels {
            channel.decay_meters(decay);
        }
        self.master_peak_left *= decay;
        self.master_peak_right *= decay;
    }
}

/// Soft clipping function using tanh
fn soft_clip(x: f32) -> f32 {
    if x.abs() < 0.5 {
        x
    } else {
        x.signum() * (0.5 + (x.abs() - 0.5).tanh() * 0.5)
    }
}
