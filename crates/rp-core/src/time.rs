use serde::{Deserialize, Serialize};

/// A position in samples (absolute)
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct SamplePosition(pub u64);

impl SamplePosition {
    pub const ZERO: SamplePosition = SamplePosition(0);

    /// Convert to seconds given a sample rate
    pub fn to_seconds(self, sample_rate: u32) -> f64 {
        self.0 as f64 / sample_rate as f64
    }

    /// Create from seconds and sample rate
    pub fn from_seconds(seconds: f64, sample_rate: u32) -> Self {
        Self((seconds * sample_rate as f64) as u64)
    }

    /// Convert to beat position given tempo and sample rate
    pub fn to_beats(self, bpm: f64, sample_rate: u32) -> BeatPosition {
        let seconds = self.to_seconds(sample_rate);
        let beats = seconds * bpm / 60.0;
        BeatPosition(beats)
    }
}

impl std::ops::Add<u64> for SamplePosition {
    type Output = Self;
    fn add(self, rhs: u64) -> Self {
        Self(self.0 + rhs)
    }
}

impl std::ops::Sub for SamplePosition {
    type Output = i64;
    fn sub(self, rhs: Self) -> i64 {
        self.0 as i64 - rhs.0 as i64
    }
}

/// A position in beats (fractional)
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct BeatPosition(pub f64);

impl BeatPosition {
    pub const ZERO: BeatPosition = BeatPosition(0.0);

    /// Get the current beat number (floor)
    pub fn beat_number(self) -> u32 {
        self.0.floor() as u32
    }

    /// Get the phase within the current beat (0.0 to 1.0)
    pub fn phase(self) -> f64 {
        self.0.fract()
    }

    /// Get the bar number (assuming 4/4 time)
    pub fn bar_number(self) -> u32 {
        (self.0 / 4.0).floor() as u32
    }

    /// Get the beat within the bar (0-3 for 4/4)
    pub fn beat_in_bar(self) -> u32 {
        self.beat_number() % 4
    }

    /// Convert to sample position given tempo and sample rate
    pub fn to_samples(self, bpm: f64, sample_rate: u32) -> SamplePosition {
        let seconds = self.0 * 60.0 / bpm;
        SamplePosition::from_seconds(seconds, sample_rate)
    }
}

/// Beatgrid information for a track
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BeatGrid {
    /// Tempo in BPM
    pub bpm: f64,
    /// Position of the first beat in samples
    pub first_beat: SamplePosition,
    /// Optional tempo changes (for variable tempo tracks)
    pub markers: Option<Vec<BeatMarker>>,
}

impl BeatGrid {
    /// Create a constant-tempo beatgrid
    pub fn constant(bpm: f64, first_beat: SamplePosition) -> Self {
        Self {
            bpm,
            first_beat,
            markers: None,
        }
    }

    /// Get samples per beat at the initial tempo
    pub fn samples_per_beat(&self, sample_rate: u32) -> f64 {
        sample_rate as f64 * 60.0 / self.bpm
    }

    /// Calculate the beat position at a given sample position
    pub fn beat_at_sample(&self, sample: SamplePosition, sample_rate: u32) -> BeatPosition {
        let samples_from_first = sample.0 as f64 - self.first_beat.0 as f64;
        let beats = samples_from_first / self.samples_per_beat(sample_rate);
        BeatPosition(beats)
    }

    /// Calculate the sample position for a given beat
    pub fn sample_at_beat(&self, beat: BeatPosition, sample_rate: u32) -> SamplePosition {
        let samples = self.first_beat.0 as f64 + beat.0 * self.samples_per_beat(sample_rate);
        SamplePosition(samples as u64)
    }
}

/// A tempo change marker for variable-tempo tracks
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BeatMarker {
    /// Position in samples where this tempo starts
    pub position: SamplePosition,
    /// New tempo from this point forward
    pub bpm: f64,
}
