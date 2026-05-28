use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

/// Global transport clock for sample-accurate synchronization.
/// All atomic operations use Relaxed ordering since we don't need
/// synchronization with other memory operations.
pub struct GlobalTransport {
    /// Master clock position in samples (monotonically increasing when playing)
    sample_position: AtomicU64,

    /// Tempo in BPM * 1000 for precision without floating point atomics
    tempo_bpm_scaled: AtomicU32,

    /// Sample rate (set once the real output device rate is known)
    sample_rate: AtomicU32,

    /// Whether the transport is playing
    is_playing: std::sync::atomic::AtomicBool,
}

impl GlobalTransport {
    pub fn new(sample_rate: u32) -> Self {
        Self {
            sample_position: AtomicU64::new(0),
            tempo_bpm_scaled: AtomicU32::new(120_000), // 120.0 BPM default
            sample_rate: AtomicU32::new(sample_rate),
            is_playing: std::sync::atomic::AtomicBool::new(false),
        }
    }

    /// Get the current sample position
    pub fn sample_position(&self) -> u64 {
        self.sample_position.load(Ordering::Relaxed)
    }

    /// Set the sample position (for seeking)
    pub fn set_sample_position(&self, position: u64) {
        self.sample_position.store(position, Ordering::Relaxed);
    }

    /// Advance the transport by a number of samples
    pub fn advance(&self, samples: u64) {
        self.sample_position.fetch_add(samples, Ordering::Relaxed);
    }

    /// Get the tempo in BPM
    pub fn tempo_bpm(&self) -> f64 {
        self.tempo_bpm_scaled.load(Ordering::Relaxed) as f64 / 1000.0
    }

    /// Set the tempo in BPM
    pub fn set_tempo_bpm(&self, bpm: f64) {
        let scaled = (bpm * 1000.0) as u32;
        self.tempo_bpm_scaled.store(scaled, Ordering::Relaxed);
    }

    /// Get the sample rate
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate.load(Ordering::Relaxed)
    }

    /// Set the sample rate (once the real output device rate is known)
    pub fn set_sample_rate(&self, sample_rate: u32) {
        self.sample_rate.store(sample_rate, Ordering::Relaxed);
    }

    /// Check if transport is playing
    pub fn is_playing(&self) -> bool {
        self.is_playing.load(Ordering::Relaxed)
    }

    /// Start playback
    pub fn play(&self) {
        self.is_playing.store(true, Ordering::Relaxed);
    }

    /// Stop playback
    pub fn stop(&self) {
        self.is_playing.store(false, Ordering::Relaxed);
    }

    /// Get samples per beat at current tempo
    pub fn samples_per_beat(&self) -> f64 {
        self.sample_rate() as f64 * 60.0 / self.tempo_bpm()
    }

    /// Get the current beat position (fractional)
    pub fn beat_position(&self) -> f64 {
        self.sample_position() as f64 / self.samples_per_beat()
    }

    /// Get the phase within the current beat (0.0 to 1.0)
    pub fn beat_phase(&self) -> f64 {
        self.beat_position().fract()
    }
}
