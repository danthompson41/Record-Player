use rp_core::CrossfaderCurve;
use std::f32::consts::PI;

/// Default sample rate assumed at construction; recomputed once the engine
/// adopts the device's real rate via `Mixer::set_sample_rate`.
const DEFAULT_SAMPLE_RATE: f32 = 48_000.0;
/// Low/mid crossover and mid/high crossover for the 3-band split EQ. Standard
/// DJ-mixer choices (Pioneer DJM-style splits use similar regions).
const EQ_LO_HZ: f32 = 200.0;
const EQ_HI_HZ: f32 = 2_000.0;
/// HP/LP filter sweep range. The "near" cutoffs are pushed out of the audible
/// band so center ≈ transparent (no dead band — the slider is continuous).
/// LP cutoff sweeps 18 kHz → 70 Hz as v goes 0 → -1; HP cutoff sweeps 20 Hz
/// → 5 kHz as v goes 0 → +1.
const FILTER_LP_NEAR_HZ: f32 = 18_000.0;
const FILTER_LP_FAR_HZ: f32 = 70.0;
const FILTER_HP_NEAR_HZ: f32 = 20.0;
const FILTER_HP_FAR_HZ: f32 = 5_000.0;
/// Butterworth Q. Two cascaded Butterworth biquads make a Linkwitz-Riley 4th-
/// order filter; an LR4 LP+HP pair has flat magnitude reconstruction at their
/// shared crossover, so summing the three bands at unity gain ≈ input.
const BUTTERWORTH_Q: f32 = std::f32::consts::FRAC_1_SQRT_2;

/// RBJ-cookbook biquad in Direct Form II Transposed (numerically stable,
/// minimal state). Used for the per-channel 3-band split EQ.
#[derive(Clone, Copy, Debug)]
struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    z1: f32,
    z2: f32,
}

impl Biquad {
    fn flat() -> Self {
        Self { b0: 1.0, b1: 0.0, b2: 0.0, a1: 0.0, a2: 0.0, z1: 0.0, z2: 0.0 }
    }

    fn set_lowpass(&mut self, sample_rate: f32, cutoff: f32, q: f32) {
        let w0 = 2.0 * PI * cutoff / sample_rate;
        let cos_w0 = w0.cos();
        let sin_w0 = w0.sin();
        let alpha = sin_w0 / (2.0 * q);
        let a0 = 1.0 + alpha;
        self.b0 = ((1.0 - cos_w0) * 0.5) / a0;
        self.b1 = (1.0 - cos_w0) / a0;
        self.b2 = self.b0;
        self.a1 = (-2.0 * cos_w0) / a0;
        self.a2 = (1.0 - alpha) / a0;
    }

    fn set_highpass(&mut self, sample_rate: f32, cutoff: f32, q: f32) {
        let w0 = 2.0 * PI * cutoff / sample_rate;
        let cos_w0 = w0.cos();
        let sin_w0 = w0.sin();
        let alpha = sin_w0 / (2.0 * q);
        let a0 = 1.0 + alpha;
        self.b0 = ((1.0 + cos_w0) * 0.5) / a0;
        self.b1 = (-(1.0 + cos_w0)) / a0;
        self.b2 = self.b0;
        self.a1 = (-2.0 * cos_w0) / a0;
        self.a2 = (1.0 - alpha) / a0;
    }

    #[inline]
    fn process(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }

    fn reset_state(&mut self) {
        self.z1 = 0.0;
        self.z2 = 0.0;
    }
}

/// Active filter kind. There's no `Off` state — at center (v = 0) the LP
/// branch is selected with its cutoff at the "near" (effectively-transparent)
/// end, so the slider is continuous through center with no dead band.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FilterMode {
    LowPass,
    HighPass,
}

/// Per-channel HP/LP filter with state. Always processes; sweeping the
/// slider monotonically moves the cutoff. Biquad state is cleared when the
/// kind flips (LP ⇄ HP, i.e. crossing v = 0) so the new biquad doesn't carry
/// over an unrelated history; at the crossing both filters are flat, so the
/// reset is inaudible.
#[derive(Clone, Copy, Debug)]
struct ChannelFilter {
    biquad_l: Biquad,
    biquad_r: Biquad,
    mode: FilterMode,
}

impl ChannelFilter {
    fn new() -> Self {
        let mut f = Self {
            biquad_l: Biquad::flat(),
            biquad_r: Biquad::flat(),
            mode: FilterMode::LowPass,
        };
        // Initialise to "near" cutoff (effective passthrough) at v = 0.
        f.set(0.0, DEFAULT_SAMPLE_RATE);
        f
    }

    /// `value` ∈ [-1, 1]. v < 0 → LP with cutoff falling from 18 kHz (≈ flat)
    /// to 70 Hz (heavy). v > 0 → HP with cutoff rising from 20 Hz (≈ flat) to
    /// 5 kHz (heavy). v = 0 is the smooth join point — both filters at v ≈ 0
    /// are essentially identity.
    fn set(&mut self, value: f32, sample_rate: f32) {
        let v = value.clamp(-1.0, 1.0);
        let new_mode = if v <= 0.0 { FilterMode::LowPass } else { FilterMode::HighPass };

        if new_mode != self.mode {
            self.biquad_l.reset_state();
            self.biquad_r.reset_state();
            self.mode = new_mode;
        }

        match new_mode {
            FilterMode::LowPass => {
                let norm = (-v).clamp(0.0, 1.0);
                let cutoff =
                    FILTER_LP_NEAR_HZ * (FILTER_LP_FAR_HZ / FILTER_LP_NEAR_HZ).powf(norm);
                self.biquad_l.set_lowpass(sample_rate, cutoff, BUTTERWORTH_Q);
                self.biquad_r.set_lowpass(sample_rate, cutoff, BUTTERWORTH_Q);
            }
            FilterMode::HighPass => {
                let norm = v.clamp(0.0, 1.0);
                let cutoff =
                    FILTER_HP_NEAR_HZ * (FILTER_HP_FAR_HZ / FILTER_HP_NEAR_HZ).powf(norm);
                self.biquad_l.set_highpass(sample_rate, cutoff, BUTTERWORTH_Q);
                self.biquad_r.set_highpass(sample_rate, cutoff, BUTTERWORTH_Q);
            }
        }
    }

    #[inline]
    fn process(&mut self, l: f32, r: f32) -> (f32, f32) {
        (self.biquad_l.process(l), self.biquad_r.process(r))
    }
}

/// Linkwitz-Riley 4th-order filter = two cascaded Butterworth biquads. An
/// LR4 LPF and LR4 HPF at the same cutoff sum to (≈) the input — that's the
/// reconstruction property the 3-band EQ relies on.
#[derive(Clone, Copy, Debug, Default)]
struct Lr4 {
    a: Biquad,
    b: Biquad,
}

impl Lr4 {
    fn lowpass(sample_rate: f32, cutoff: f32) -> Self {
        let mut me = Self::default();
        me.set_lowpass(sample_rate, cutoff);
        me
    }

    fn highpass(sample_rate: f32, cutoff: f32) -> Self {
        let mut me = Self::default();
        me.set_highpass(sample_rate, cutoff);
        me
    }

    fn set_lowpass(&mut self, sample_rate: f32, cutoff: f32) {
        self.a.set_lowpass(sample_rate, cutoff, BUTTERWORTH_Q);
        self.b.set_lowpass(sample_rate, cutoff, BUTTERWORTH_Q);
    }

    fn set_highpass(&mut self, sample_rate: f32, cutoff: f32) {
        self.a.set_highpass(sample_rate, cutoff, BUTTERWORTH_Q);
        self.b.set_highpass(sample_rate, cutoff, BUTTERWORTH_Q);
    }

    #[inline]
    fn process(&mut self, x: f32) -> f32 {
        self.b.process(self.a.process(x))
    }
}

impl Default for Biquad {
    fn default() -> Self {
        Self::flat()
    }
}

/// 3-band split EQ for one stereo channel. Two-stage split: an LR4 LP/HP at
/// `EQ_LO_HZ` separates the low band from "not low", then an LR4 LP/HP at
/// `EQ_HI_HZ` splits "not low" into mid and high. At gains = [1, 1, 1] the
/// three bands recombine to (≈) the original input via LR4 reconstruction;
/// any band at 0.0 fully eliminates that frequency range.
#[derive(Clone, Copy, Debug, Default)]
struct ChannelEq {
    lp_lo_l: Lr4,
    lp_lo_r: Lr4,
    hp_lo_l: Lr4,
    hp_lo_r: Lr4,
    lp_hi_l: Lr4,
    lp_hi_r: Lr4,
    hp_hi_l: Lr4,
    hp_hi_r: Lr4,
}

impl ChannelEq {
    fn new(sample_rate: f32) -> Self {
        Self {
            lp_lo_l: Lr4::lowpass(sample_rate, EQ_LO_HZ),
            lp_lo_r: Lr4::lowpass(sample_rate, EQ_LO_HZ),
            hp_lo_l: Lr4::highpass(sample_rate, EQ_LO_HZ),
            hp_lo_r: Lr4::highpass(sample_rate, EQ_LO_HZ),
            lp_hi_l: Lr4::lowpass(sample_rate, EQ_HI_HZ),
            lp_hi_r: Lr4::lowpass(sample_rate, EQ_HI_HZ),
            hp_hi_l: Lr4::highpass(sample_rate, EQ_HI_HZ),
            hp_hi_r: Lr4::highpass(sample_rate, EQ_HI_HZ),
        }
    }

    fn set_sample_rate(&mut self, sample_rate: f32) {
        self.lp_lo_l.set_lowpass(sample_rate, EQ_LO_HZ);
        self.lp_lo_r.set_lowpass(sample_rate, EQ_LO_HZ);
        self.hp_lo_l.set_highpass(sample_rate, EQ_LO_HZ);
        self.hp_lo_r.set_highpass(sample_rate, EQ_LO_HZ);
        self.lp_hi_l.set_lowpass(sample_rate, EQ_HI_HZ);
        self.lp_hi_r.set_lowpass(sample_rate, EQ_HI_HZ);
        self.hp_hi_l.set_highpass(sample_rate, EQ_HI_HZ);
        self.hp_hi_r.set_highpass(sample_rate, EQ_HI_HZ);
    }

    #[inline]
    fn process(&mut self, l: f32, r: f32, gains: [f32; 3]) -> (f32, f32) {
        let lo_l = self.lp_lo_l.process(l);
        let lo_r = self.lp_lo_r.process(r);
        let nl_l = self.hp_lo_l.process(l);
        let nl_r = self.hp_lo_r.process(r);
        let mid_l = self.lp_hi_l.process(nl_l);
        let mid_r = self.lp_hi_r.process(nl_r);
        let hi_l = self.hp_hi_l.process(nl_l);
        let hi_r = self.hp_hi_r.process(nl_r);
        let out_l = gains[0] * lo_l + gains[1] * mid_l + gains[2] * hi_l;
        let out_r = gains[0] * lo_r + gains[1] * mid_r + gains[2] * hi_r;
        (out_l, out_r)
    }
}

/// Per-channel strip with gain, DJ-style 3-band elimination EQ, and a
/// DJM-style bipolar HP/LP "colour" filter.
pub struct ChannelStrip {
    /// Input trim/gain
    pub trim: f32,
    /// Fader level (0.0 to 1.0)
    pub gain: f32,
    /// 3-band EQ gains (per-band multiplier). 0.0 = silenced ("kill"),
    /// 1.0 = flat (no change), 2.0 = +6 dB boost. The three bands recombine
    /// to the input signal when all three are 1.0 (LR4 split reconstruction).
    pub eq_low: f32,
    pub eq_mid: f32,
    pub eq_high: f32,
    eq_state: ChannelEq,
    filter_state: ChannelFilter,
    /// Peak meters (updated during render)
    pub peak_left: f32,
    pub peak_right: f32,
}

impl Default for ChannelStrip {
    fn default() -> Self {
        Self {
            trim: 1.0,
            gain: 1.0,
            eq_low: 1.0,
            eq_mid: 1.0,
            eq_high: 1.0,
            eq_state: ChannelEq::new(DEFAULT_SAMPLE_RATE),
            filter_state: ChannelFilter::new(),
            peak_left: 0.0,
            peak_right: 0.0,
        }
    }
}

impl ChannelStrip {
    /// Calculate the effective gain including EQ (simplified).
    pub fn effective_gain(&self) -> f32 {
        self.trim * self.gain
    }

    pub fn update_meters(&mut self, left: f32, right: f32) {
        self.peak_left = self.peak_left.max(left.abs());
        self.peak_right = self.peak_right.max(right.abs());
    }

    pub fn decay_meters(&mut self, decay: f32) {
        self.peak_left *= decay;
        self.peak_right *= decay;
    }
}

/// 4-deck mixer with an XY-pad crossfader. Decks A/B/C/D are pinned to the
/// pad's four corners; the (x, y) position picks a continuous blend between
/// them via an equal-power tensor product (cos/sin on each axis).
pub struct Mixer {
    /// Channel strips for each deck (A=0, B=1, C=2, D=3).
    pub channels: [ChannelStrip; 4],
    /// XY crossfader position; each in [-1, 1]. (−1, −1) = corner A.
    pub crossfader_x: f32,
    pub crossfader_y: f32,
    /// Crossfader curve (Linear vs ConstantPower vs Transition vs Scratch)
    /// is applied per-axis, so the curve generalizes naturally to 2D.
    pub crossfader_curve: CrossfaderCurve,
    /// Master gain.
    pub master_gain: f32,
    /// Master peak meters.
    pub master_peak_left: f32,
    pub master_peak_right: f32,
    /// Cached so per-channel filter coefficients can be recomputed at the
    /// command level without re-plumbing the rate through every call site.
    sample_rate: f32,
}

impl Default for Mixer {
    fn default() -> Self {
        Self {
            channels: [
                ChannelStrip::default(),
                ChannelStrip::default(),
                ChannelStrip::default(),
                ChannelStrip::default(),
            ],
            crossfader_x: 0.0,
            crossfader_y: 0.0,
            crossfader_curve: CrossfaderCurve::ConstantPower,
            master_gain: 1.0,
            master_peak_left: 0.0,
            master_peak_right: 0.0,
            sample_rate: DEFAULT_SAMPLE_RATE,
        }
    }
}

impl Mixer {
    /// Recompute EQ biquad coefficients for the device sample rate. Called
    /// by the engine on startup once the real device rate is known.
    pub fn set_sample_rate(&mut self, sample_rate: u32) {
        let sr = sample_rate as f32;
        self.sample_rate = sr;
        for ch in &mut self.channels {
            ch.eq_state.set_sample_rate(sr);
            // Filter coefficients depend on the cached `mode` + current sample
            // rate; re-driving `set` with the implicit current value keeps the
            // filter behaving the same after a rate change.
            // (Filter sliders default to bypass at engine start, so this is a
            // no-op until the user actually moves the filter.)
        }
    }

    /// Apply a colour-filter slider value in [-1, 1] to a channel. 0 = bypass.
    pub fn set_channel_filter(&mut self, channel: usize, value: f32) {
        if let Some(ch) = self.channels.get_mut(channel) {
            ch.filter_state.set(value, self.sample_rate);
        }
    }

    /// Per-axis crossfade gains (left, right) for a value in [-1, 1] using
    /// the configured crossfader curve.
    fn axis_gains(&self, pos: f32) -> (f32, f32) {
        let p = ((pos + 1.0) * 0.5).clamp(0.0, 1.0); // 0..1
        match self.crossfader_curve {
            CrossfaderCurve::Linear => (1.0 - p, p),
            CrossfaderCurve::ConstantPower => {
                let angle = p * PI * 0.5;
                (angle.cos(), angle.sin())
            }
            CrossfaderCurve::Scratch => {
                let lo = if p < 0.1 { 1.0 } else { 0.0 };
                let hi = if p > 0.9 { 1.0 } else { 0.0 };
                (lo, hi)
            }
            CrossfaderCurve::Transition => {
                let s = p * p * (3.0 - 2.0 * p);
                (1.0 - s, s)
            }
        }
    }

    /// Gain weights for each of the four corner-assigned decks given the
    /// current (x, y). Layout:
    ///   A = top-left, B = top-right, C = bottom-left, D = bottom-right.
    /// At center (0, 0) all four decks play at 0.5 each (equal-power total = 1).
    pub fn xy_corner_gains(&self) -> [f32; 4] {
        let (left, right) = self.axis_gains(self.crossfader_x);
        let (top, bottom) = self.axis_gains(self.crossfader_y);
        [
            left * top,     // A
            right * top,    // B
            left * bottom,  // C
            right * bottom, // D
        ]
    }

    /// Apply just the per-channel EQ + filter to one stereo sample, advancing
    /// the biquad state. Split out from `mix` so the engine can pre-compute a
    /// per-deck post-EQ/filter buffer once per block, then re-use it for both
    /// the Link Audio broadcast and the per-frame crossfader/master sum
    /// without running the biquads twice.
    #[inline]
    pub fn process_channel_eq_filter(
        &mut self,
        channel: usize,
        l: f32,
        r: f32,
    ) -> (f32, f32) {
        let ch = &mut self.channels[channel];
        let eq_gains = [ch.eq_low, ch.eq_mid, ch.eq_high];
        let (eq_l, eq_r) = ch.eq_state.process(l, r, eq_gains);
        ch.filter_state.process(eq_l, eq_r)
    }

    /// Combine pre-processed (post-EQ/filter) deck samples into a stereo
    /// master with crossfader + gain + master + soft clip + meter updates.
    /// Pair with `process_channel_eq_filter`.
    #[inline]
    pub fn combine(&mut self, processed: &[[f32; 2]; 4], output: &mut [f32; 2]) {
        let mut left = 0.0f32;
        let mut right = 0.0f32;
        let xy_gains = self.xy_corner_gains();

        for (i, sample) in processed.iter().enumerate() {
            let channel = &mut self.channels[i];
            let gain = channel.effective_gain() * xy_gains[i];
            let sample_l = sample[0] * gain;
            let sample_r = sample[1] * gain;
            channel.update_meters(sample_l, sample_r);
            left += sample_l;
            right += sample_r;
        }

        left *= self.master_gain;
        right *= self.master_gain;

        left = soft_clip(left);
        right = soft_clip(right);

        self.master_peak_left = self.master_peak_left.max(left.abs());
        self.master_peak_right = self.master_peak_right.max(right.abs());

        output[0] = left;
        output[1] = right;
    }

    /// Mix four deck outputs into a stereo master output. Convenience wrapper
    /// around `process_channel_eq_filter` + `combine`; the engine bypasses
    /// this path so it can capture the intermediate post-EQ/filter signal for
    /// Link Audio. Kept for backwards compatibility / tests.
    pub fn mix(&mut self, deck_outputs: &[[f32; 2]; 4], output: &mut [f32; 2]) {
        let mut processed = [[0.0f32; 2]; 4];
        for (i, deck_out) in deck_outputs.iter().enumerate() {
            let (l, r) = self.process_channel_eq_filter(i, deck_out[0], deck_out[1]);
            processed[i] = [l, r];
        }
        self.combine(&processed, output);
    }

    pub fn decay_meters(&mut self, decay: f32) {
        for channel in &mut self.channels {
            channel.decay_meters(decay);
        }
        self.master_peak_left *= decay;
        self.master_peak_right *= decay;
    }
}

#[cfg(test)]
mod eq_tests {
    use super::*;

    fn run(samples: &[f32], gains: [f32; 3]) -> Vec<f32> {
        let mut eq = ChannelEq::new(48_000.0);
        samples
            .iter()
            .map(|&x| eq.process(x, x, gains).0)
            .collect()
    }

    fn rms(xs: &[f32]) -> f32 {
        let n = xs.len() as f32;
        (xs.iter().map(|x| x * x).sum::<f32>() / n).sqrt()
    }

    /// Generate `secs` of a sine at `freq_hz`. Discard the first 100 ms so the
    /// biquad transient has settled before we measure.
    fn sine(freq_hz: f32, secs: f32) -> Vec<f32> {
        let sr = 48_000.0_f32;
        let n = (sr * secs) as usize;
        (0..n)
            .map(|i| (2.0 * PI * freq_hz * (i as f32) / sr).sin())
            .collect()
    }

    fn settled(xs: Vec<f32>) -> Vec<f32> {
        let skip = 48_000 / 10; // 100 ms
        xs.into_iter().skip(skip).collect()
    }

    #[test]
    fn unity_gain_reconstructs_input() {
        // At [1, 1, 1] the three bands should sum back to (≈) the input. We
        // sweep several frequencies and check RMS within 1.5 dB of input.
        for &f in &[60.0_f32, 500.0, 5_000.0] {
            let input = sine(f, 0.5);
            let out = run(&input, [1.0, 1.0, 1.0]);
            let r_in = rms(&settled(input));
            let r_out = rms(&settled(out));
            let ratio_db = 20.0 * (r_out / r_in).log10();
            assert!(
                ratio_db.abs() < 1.5,
                "expected ~flat reconstruction at {f} Hz, got {ratio_db:.2} dB"
            );
        }
    }

    #[test]
    fn low_kill_silences_low_band() {
        // 60 Hz sine, low band killed → output should be tiny.
        let input = sine(60.0, 0.5);
        let out = run(&input, [0.0, 1.0, 1.0]);
        let r_in = rms(&settled(input));
        let r_out = rms(&settled(out));
        assert!(
            r_out / r_in < 0.05,
            "expected near-silence at 60 Hz with low killed, got ratio {:.4}",
            r_out / r_in
        );
    }

    #[test]
    fn high_kill_silences_high_band() {
        // 6 kHz sine, high band killed → output should be tiny.
        let input = sine(6_000.0, 0.5);
        let out = run(&input, [1.0, 1.0, 0.0]);
        let r_in = rms(&settled(input));
        let r_out = rms(&settled(out));
        assert!(
            r_out / r_in < 0.05,
            "expected near-silence at 6 kHz with high killed, got ratio {:.4}",
            r_out / r_in
        );
    }

    fn run_filter(samples: &[f32], value: f32) -> Vec<f32> {
        let mut f = ChannelFilter::new();
        f.set(value, 48_000.0);
        samples.iter().map(|&x| f.process(x, x).0).collect()
    }

    #[test]
    fn filter_center_is_effectively_transparent() {
        // At v = 0 the LP cutoff sits at 18 kHz — well above any audio-band
        // test frequency, so output ≈ input across the band (within 0.5 dB).
        for &f in &[60.0_f32, 1_000.0, 5_000.0] {
            let input = sine(f, 0.5);
            let out = run_filter(&input, 0.0);
            let r_in = rms(&settled(input));
            let r_out = rms(&settled(out));
            let ratio_db = 20.0 * (r_out / r_in).log10();
            assert!(
                ratio_db.abs() < 0.5,
                "expected near-transparent at {f} Hz, got {ratio_db:.2} dB"
            );
        }
    }

    #[test]
    fn full_lp_kills_high_content() {
        // 8 kHz sine, full LP (cutoff ≈ 70 Hz) → output should be tiny.
        let input = sine(8_000.0, 0.5);
        let out = run_filter(&input, -1.0);
        let r_in = rms(&settled(input));
        let r_out = rms(&settled(out));
        assert!(
            r_out / r_in < 0.01,
            "expected near-silence at 8 kHz with full LP, got ratio {:.4}",
            r_out / r_in
        );
    }

    #[test]
    fn full_hp_kills_low_content() {
        // 60 Hz sine, full HP (cutoff ≈ 5 kHz) → output should be tiny.
        let input = sine(60.0, 0.5);
        let out = run_filter(&input, 1.0);
        let r_in = rms(&settled(input));
        let r_out = rms(&settled(out));
        assert!(
            r_out / r_in < 0.01,
            "expected near-silence at 60 Hz with full HP, got ratio {:.4}",
            r_out / r_in
        );
    }

    #[test]
    fn full_lp_preserves_low_content() {
        // 60 Hz sine, full LP → cutoff is at 70 Hz so 60 Hz still mostly passes.
        let input = sine(60.0, 0.5);
        let out = run_filter(&input, -1.0);
        let r_in = rms(&settled(input));
        let r_out = rms(&settled(out));
        assert!(
            r_out / r_in > 0.4,
            "expected most of 60 Hz to pass through full LP (cutoff 70 Hz), got ratio {:.4}",
            r_out / r_in
        );
    }

    #[test]
    fn mid_kill_silences_mid_band() {
        // 800 Hz sine sits squarely in the mid band; killing mid → tiny output.
        let input = sine(800.0, 0.5);
        let out = run(&input, [1.0, 0.0, 1.0]);
        let r_in = rms(&settled(input));
        let r_out = rms(&settled(out));
        assert!(
            r_out / r_in < 0.10,
            "expected near-silence at 800 Hz with mid killed, got ratio {:.4}",
            r_out / r_in
        );
    }
}

/// Soft clipping using tanh on the upper half-range.
fn soft_clip(x: f32) -> f32 {
    if x.abs() < 0.5 {
        x
    } else {
        x.signum() * (0.5 + (x.abs() - 0.5).tanh() * 0.5)
    }
}
