use rustfft::{num_complex::Complex, FftPlanner};
use std::collections::VecDeque;

const WINDOW: usize = 1024;
const HOP: usize = 512;
const MIN_BPM: i32 = 60;
const MAX_BPM: i32 = 200;
/// Top kick-band frequency (Hz). Bins above this are dropped from the ODF —
/// hi-hats / shakers add huge flux at the wrong rate and trick the detector.
const KICK_BAND_HZ: f64 = 250.0;
const PRIOR_CENTER: f64 = 120.0;
const PRIOR_SIGMA_OCT: f64 = 1.2;
const MIN_TRACK_SECS: f64 = 5.0;
const PHASE_STEPS: usize = 96;
const ANALYSIS_WINDOW_SECS: f64 = 150.0;
/// When two integer BPMs score within this ratio of each other, prefer the
/// lower one — keeps 90 BPM tracks from snapping to 180 on perfectly clean
/// signals where the two octaves are mathematically equivalent.
const OCTAVE_TIE_TOLERANCE: f32 = 0.985;

/// Detects BPM from PCM samples using spectral-flux onset envelope +
/// prior-weighted autocorrelation + octave correction. Tuned for electronic
/// music with strong, consistent percussive content; returns an integer BPM.
pub struct BpmDetector {
    sample_rate: u32,
}

impl BpmDetector {
    pub fn new(sample_rate: u32) -> Self {
        Self { sample_rate }
    }

    /// `samples` is interleaved if `channels > 1`. Returns `None` when the
    /// input is too short, silent, or lacks confidently periodic onsets.
    pub fn detect(&self, samples: &[f32], channels: u16) -> Option<f64> {
        let sr = self.sample_rate as f64;
        if sr <= 0.0 {
            return None;
        }
        let ch = channels.max(1) as usize;
        let total_frames = samples.len() / ch;
        if (total_frames as f64) < MIN_TRACK_SECS * sr {
            return None;
        }

        let (start, end) = analysis_window(total_frames, sr);
        let mono = downmix(&samples[start * ch..end * ch], ch);

        let odf = kick_band_flux_odf(&mono, self.sample_rate);
        let odf_max = odf.iter().fold(0.0_f32, |a, &b| a.max(b));
        if odf_max < 1e-6 {
            return None;
        }
        let odf = smooth_and_rectify(&odf, sr);

        let hop_rate = sr / HOP as f64;
        pick_integer_bpm(&odf, hop_rate)
    }
}

/// Analyze the middle of the track. We use a long window (~120 s) so the
/// phase-synchrony refinement step has many beats to discriminate adjacent
/// integer BPMs (drift over N beats grows with N, so longer = sharper).
fn analysis_window(total_frames: usize, sr: f64) -> (usize, usize) {
    let target = (ANALYSIS_WINDOW_SECS * sr) as usize;
    // For tracks shorter than ~window+30s use the whole thing (no skip).
    if total_frames < target + (30.0 * sr) as usize {
        (0, total_frames.max(WINDOW + HOP))
    } else {
        let start = (30.0 * sr) as usize;
        let end = (start + target).min(total_frames);
        (start, end)
    }
}

fn downmix(interleaved: &[f32], ch: usize) -> Vec<f32> {
    if ch == 1 {
        return interleaved.to_vec();
    }
    let n = interleaved.len() / ch;
    let mut out = Vec::with_capacity(n);
    let inv = 1.0 / ch as f32;
    for i in 0..n {
        let base = i * ch;
        let mut s = 0.0;
        for c in 0..ch {
            s += interleaved[base + c];
        }
        out.push(s * inv);
    }
    out
}

/// Kick-band spectral flux ODF. Same STFT pipeline as standard spectral flux,
/// but restricted to bins covering ≈ 30 – 250 Hz so it only responds to
/// kick-drum onsets. Full-band flux is dominated by hi-hats / shakers on
/// electronic tracks, which fire at 8th or 16th notes and trick the tempo
/// estimator into octave doubling — restricting to the kick band removes
/// that bias entirely.
fn kick_band_flux_odf(mono: &[f32], sample_rate: u32) -> Vec<f32> {
    if mono.len() < WINDOW {
        return Vec::new();
    }
    let frames = (mono.len() - WINDOW) / HOP + 1;
    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(WINDOW);

    let win: Vec<f32> = (0..WINDOW)
        .map(|i| {
            let x = i as f32 / (WINDOW - 1) as f32;
            0.5 - 0.5 * (2.0 * std::f32::consts::PI * x).cos()
        })
        .collect();

    let n_bins = WINDOW / 2 + 1;
    // Kick band: skip bin 0 (DC) and cap at KICK_BAND_HZ.
    let bin_hz = sample_rate as f64 / WINDOW as f64;
    let band_lo = 1usize;
    let band_hi = (((KICK_BAND_HZ / bin_hz).round() as usize) + 1).min(n_bins);
    if band_hi <= band_lo {
        return Vec::new();
    }

    let mut buf: Vec<Complex<f32>> = vec![Complex { re: 0.0, im: 0.0 }; WINDOW];
    let mut prev = vec![0.0_f32; band_hi - band_lo];
    let mut odf = Vec::with_capacity(frames);

    for f in 0..frames {
        let s = f * HOP;
        for i in 0..WINDOW {
            buf[i] = Complex {
                re: mono[s + i] * win[i],
                im: 0.0,
            };
        }
        fft.process(&mut buf);
        let mut flux = 0.0_f32;
        for (j, k) in (band_lo..band_hi).enumerate() {
            let mag = (1.0 + buf[k].norm()).ln();
            if f > 0 {
                let d = mag - prev[j];
                if d > 0.0 {
                    flux += d;
                }
            }
            prev[j] = mag;
        }
        odf.push(flux);
    }
    odf
}

/// Subtract a ~0.5 s moving mean, half-wave rectify, then normalize by the
/// remaining mean so the autocorrelation is scale-invariant.
fn smooth_and_rectify(odf: &[f32], sr: f64) -> Vec<f32> {
    let hop_rate = sr / HOP as f64;
    let ma_len = ((0.5 * hop_rate) as usize).max(1);
    let n = odf.len();
    let mut out = vec![0.0_f32; n];
    let mut q: VecDeque<f32> = VecDeque::with_capacity(ma_len);
    let mut acc = 0.0_f32;
    for i in 0..n {
        acc += odf[i];
        q.push_back(odf[i]);
        if q.len() > ma_len {
            acc -= q.pop_front().unwrap();
        }
        let mean = acc / q.len() as f32;
        let v = odf[i] - mean;
        out[i] = if v > 0.0 { v } else { 0.0 };
    }
    let mean = out.iter().sum::<f32>() / (out.len() as f32).max(1.0);
    if mean > 1e-9 {
        for v in out.iter_mut() {
            *v /= mean;
        }
    }
    out
}

/// Weak log-Gaussian tempo prior centered on 120 BPM. Used only to break
/// near-ties between octaves on tracks whose ODF is genuinely octave-
/// ambiguous (e.g. a kick on every other beat); strong signals override it.
fn tempo_prior(bpm: f64) -> f32 {
    let log_diff = (bpm / PRIOR_CENTER).log2();
    (-0.5 * (log_diff / PRIOR_SIGMA_OCT).powi(2)).exp() as f32
}

/// Score every integer BPM in [MIN_BPM, MAX_BPM] by phase synchrony against
/// the ODF, then pick the integer with the best `score × prior`.
///
/// Why this approach: every wrong integer BPM (off by ≥ 1) drifts off the
/// actual beats over the analysis window (tens or hundreds of beats) — its
/// beat positions land on increasingly-off-beat ODF samples and the sum
/// collapses. Only the exact integer hits the same on-beat sample every
/// time, so it stands out clearly even on noisy real-music ODFs.
///
/// Why no autocorrelation crutch: autocorrelation argmax can lock onto the
/// half-period lag (where every click pair contributes, doubling the raw
/// score) and trap the search in the wrong octave. Phase synchrony rewards
/// the *true* rate — wrong octaves either halve the number of beats or hit
/// only every other ODF peak — so it handles octave choice on its own.
fn pick_integer_bpm(odf: &[f32], hop_rate: f64) -> Option<f64> {
    if odf.is_empty() {
        return None;
    }
    let mut best_bpm = 0;
    let mut best_score = f32::NEG_INFINITY;
    for b in MIN_BPM..=MAX_BPM {
        let period = 60.0 * hop_rate / b as f64;
        if period < 4.0 || (period as usize) >= odf.len() {
            continue;
        }
        let raw = best_phase_score(odf, period);
        let weighted = raw * tempo_prior(b as f64);
        if weighted > best_score {
            best_score = weighted;
            best_bpm = b;
        }
    }
    if best_bpm == 0 {
        return None;
    }

    // Octave-down tiebreaker: a clean half-rate signal scores arithmetically
    // identical to its true rate. Walk downward through 0.5× / 0.5× and pick
    // the smallest integer that's still within the tie tolerance.
    let resolved = resolve_octave_tie(odf, hop_rate, best_bpm, best_score);
    Some(resolved as f64)
}

fn resolve_octave_tie(odf: &[f32], hop_rate: f64, start_bpm: i32, start_score: f32) -> i32 {
    let mut bpm = start_bpm;
    let mut score = start_score;
    loop {
        let half = bpm / 2;
        if half < MIN_BPM {
            break;
        }
        let period = 60.0 * hop_rate / half as f64;
        if period as usize >= odf.len() {
            break;
        }
        let half_score = best_phase_score(odf, period) * tempo_prior(half as f64);
        if half_score >= OCTAVE_TIE_TOLERANCE * score {
            bpm = half;
            score = half_score;
        } else {
            break;
        }
    }
    bpm
}

/// Maximum over phase offsets in [0, period) of the sum of linearly-
/// interpolated ODF samples at beat positions.
fn best_phase_score(odf: &[f32], period: f64) -> f32 {
    let mut best = 0.0_f32;
    for ps in 0..PHASE_STEPS {
        let phase = (ps as f64 / PHASE_STEPS as f64) * period;
        let mut score = 0.0_f32;
        let mut pos = phase;
        while (pos as usize) < odf.len() {
            let idx = pos as usize;
            let frac = (pos - idx as f64) as f32;
            let v0 = odf[idx];
            let v1 = if idx + 1 < odf.len() { odf[idx + 1] } else { 0.0 };
            score += v0 * (1.0 - frac) + v1 * frac;
            pos += period;
        }
        if score > best {
            best = score;
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: u32 = 44100;

    /// Synthesize a "kick" click train at `bpm`. Each click is a short
    /// exponentially-decaying tone so it has broadband spectral content (a
    /// pure dirac per sample wouldn't excite the spectral-flux ODF as well).
    fn click_train(bpm: f64, secs: f64, sr: u32) -> Vec<f32> {
        let period = (sr as f64 * 60.0 / bpm) as usize;
        let total = (sr as f64 * secs) as usize;
        let mut s = vec![0.0_f32; total];
        let click_len = 200; // ~4.5 ms at 44.1k
        let mut t = 0;
        while t < total {
            for i in 0..click_len.min(total - t) {
                let env = (-(i as f32) / 40.0).exp();
                let tone = (2.0 * std::f32::consts::PI * 80.0 * (i as f32) / sr as f32).sin();
                s[t + i] += env * tone * 0.9;
            }
            t += period;
        }
        s
    }

    fn assert_detects_exact(bpm_in: f64, bpm_out_expected: f64) {
        // 60s of audio so the refinement step has hundreds of beats; phase
        // drift over that span is what disambiguates adjacent integer BPMs.
        let secs = 60.0;
        let audio = click_train(bpm_in, secs, SR);
        let detected = BpmDetector::new(SR).detect(&audio, 1);
        let detected = detected.unwrap_or_else(|| panic!("no detection for {} BPM", bpm_in));
        assert_eq!(
            detected, bpm_out_expected,
            "expected exact {} BPM, got {} (input {})",
            bpm_out_expected, detected, bpm_in
        );
    }

    #[test]
    fn detects_90_bpm() {
        assert_detects_exact(90.0, 90.0);
    }

    #[test]
    fn detects_120_bpm() {
        assert_detects_exact(120.0, 120.0);
    }

    #[test]
    fn detects_124_bpm() {
        assert_detects_exact(124.0, 124.0);
    }

    #[test]
    fn detects_128_bpm() {
        assert_detects_exact(128.0, 128.0);
    }

    #[test]
    fn detects_140_bpm() {
        assert_detects_exact(140.0, 140.0);
    }

    #[test]
    fn detects_174_bpm() {
        assert_detects_exact(174.0, 174.0);
    }

    #[test]
    fn detects_175_bpm() {
        assert_detects_exact(175.0, 175.0);
    }

    /// Clicks at `fast_bpm`, but every other click is louder. The fundamental
    /// autocorrelation period is the *slow* (half) rate; the *fast* rate also
    /// has substantial autocorrelation. Used to exercise octave correction.
    fn emphasized_click_train(fast_bpm: f64, secs: f64, sr: u32) -> Vec<f32> {
        let period = (sr as f64 * 60.0 / fast_bpm) as usize;
        let total = (sr as f64 * secs) as usize;
        let mut s = vec![0.0_f32; total];
        let click_len = 200;
        let mut t = 0;
        let mut idx = 0;
        while t < total {
            let amp = if idx % 2 == 0 { 1.0 } else { 0.55 };
            for i in 0..click_len.min(total - t) {
                let env = (-(i as f32) / 40.0).exp();
                let tone = (2.0 * std::f32::consts::PI * 80.0 * (i as f32) / sr as f32).sin();
                s[t + i] += env * tone * 0.9 * amp;
            }
            t += period;
            idx += 1;
        }
        s
    }

    #[test]
    fn octave_correction_lifts_into_preferred_band() {
        // Pulses at 65 BPM (slow) with weaker pulses at 130 BPM (fast) added.
        // Naive argmax tends to pick the 65 BPM period; octave correction
        // should lift it to 130 because the 130-lag autocorr is also strong
        // AND 130 is in the preferred electronic-music band.
        let secs = 30.0;
        let audio = emphasized_click_train(130.0, secs, SR);
        let detected = BpmDetector::new(SR).detect(&audio, 1).expect("detection");
        assert!(
            (detected - 130.0).abs() <= 1.0,
            "expected ~130 BPM after octave correction, got {}",
            detected
        );
    }

    /// Kick (low-frequency) clicks at `kick_bpm`, plus a high-frequency
    /// "shaker" at 4× the kick rate. The full-spectrum spectral flux is
    /// dominated by the shaker; the kick-band ODF should ignore it.
    fn kick_plus_shaker(kick_bpm: f64, secs: f64, sr: u32) -> Vec<f32> {
        let kick_period = (sr as f64 * 60.0 / kick_bpm) as usize;
        let shaker_period = kick_period / 4;
        let total = (sr as f64 * secs) as usize;
        let mut s = vec![0.0_f32; total];
        let mut t = 0;
        while t < total {
            // Kick: ~80 Hz exponential decay.
            for i in 0..200.min(total - t) {
                let env = (-(i as f32) / 40.0).exp();
                let tone = (2.0 * std::f32::consts::PI * 80.0 * (i as f32) / sr as f32).sin();
                s[t + i] += env * tone * 0.9;
            }
            t += kick_period;
        }
        let mut t = 0;
        while t < total {
            // Shaker: high-frequency noise burst.
            for i in 0..120.min(total - t) {
                let env = (-(i as f32) / 25.0).exp();
                let tone = (2.0 * std::f32::consts::PI * 6000.0 * (i as f32) / sr as f32).sin();
                // Make it louder than the kick to really test the band restriction.
                s[t + i] += env * tone * 1.4;
            }
            t += shaker_period;
        }
        s
    }

    #[test]
    fn ignores_shaker_at_four_times_kick_rate() {
        // 128 BPM kick + 512 BPM shaker (which is way above MAX so the only
        // way detection succeeds is if the shaker is filtered out of the ODF).
        let audio = kick_plus_shaker(128.0, 60.0, SR);
        let detected = BpmDetector::new(SR).detect(&audio, 1).expect("detection");
        assert_eq!(detected, 128.0, "kick-band filter let the shaker dominate");
    }

    #[test]
    fn returns_none_for_silence() {
        let audio = vec![0.0_f32; SR as usize * 10];
        assert!(BpmDetector::new(SR).detect(&audio, 1).is_none());
    }

    #[test]
    fn returns_none_for_too_short_input() {
        let audio = vec![0.5_f32; SR as usize * 2]; // 2 seconds
        assert!(BpmDetector::new(SR).detect(&audio, 1).is_none());
    }
}
