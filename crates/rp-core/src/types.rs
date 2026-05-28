use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Unique identifier for a track in the library
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TrackId(pub u64);

/// Unique identifier for a deck (0-3 for 4 decks)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DeckId(pub u8);

impl DeckId {
    pub const DECK_A: DeckId = DeckId(0);
    pub const DECK_B: DeckId = DeckId(1);
    pub const DECK_C: DeckId = DeckId(2);
    pub const DECK_D: DeckId = DeckId(3);
}

/// Audio format information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioFormat {
    pub sample_rate: u32,
    pub channels: u16,
    pub bits_per_sample: u16,
}

impl Default for AudioFormat {
    fn default() -> Self {
        Self {
            sample_rate: 44100,
            channels: 2,
            bits_per_sample: 16,
        }
    }
}

/// Track metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackMetadata {
    pub id: TrackId,
    pub path: PathBuf,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub duration_samples: u64,
    pub format: AudioFormat,
    pub bpm: Option<f64>,
    /// Position of the first beat (downbeat) relative to the start of the audio,
    /// in source samples. May be negative (downbeat falls before sample 0, i.e.
    /// the track needs pre-roll silence). `None` means not set (treated as 0).
    pub first_beat: Option<i64>,
    /// Saved loop start in source samples (grid-aligned). `None` = no saved loop.
    pub loop_start: Option<i64>,
    /// Saved loop length in beats (e.g. 16 = 4 bars, 0.5 = 1/8 bar). `None`/0 =
    /// no saved loop.
    pub loop_beats: Option<f64>,
    /// Saved cue points (hot cues 1-8) in source samples; `None` = unset.
    pub cues: [Option<i64>; 8],
    /// Cached low-res RMS overview (one byte per bucket, 0-255) for library
    /// thumbnails, computed when the track is loaded. `None` = not yet computed.
    pub waveform: Option<Vec<u8>>,
}

/// Playback state for a deck
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlaybackState {
    Stopped,
    Playing,
    Paused,
}

/// Quantization grid for starting deck playback, aligned to the global clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Quantize {
    /// Start immediately, no quantization.
    Off,
    /// Snap the start to the next quarter-note beat.
    Beat,
    /// Snap the start to the next bar (4 beats).
    Bar,
}

impl Quantize {
    /// Number of beats in this quantization grid (0 when off).
    pub fn beats(self) -> f64 {
        match self {
            Quantize::Off => 0.0,
            Quantize::Beat => 1.0,
            Quantize::Bar => 4.0,
        }
    }
}

/// Sync mode for a deck
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SyncMode {
    /// Free play - no synchronization
    Off,
    /// Match tempo only
    Tempo,
    /// Match tempo and phase-lock to beats
    Phase,
}

/// Crossfader assignment for a channel
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CrossfaderAssign {
    /// Assigned to side A (left)
    A,
    /// Assigned to side B (right)
    B,
    /// Bypasses crossfader entirely
    Thru,
}

/// Crossfader curve type
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CrossfaderCurve {
    /// Linear: -3dB at center
    Linear,
    /// Constant power: 0dB at center
    ConstantPower,
    /// Sharp cut for scratching
    Scratch,
    /// Smooth S-curve for mixing
    Transition,
}
