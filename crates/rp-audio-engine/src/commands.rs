use crate::deck::AudioBuffer;
use rp_core::{CrossfaderCurve, DeckId, Quantize, SyncMode};
use std::sync::Arc;

/// Commands sent from the UI thread to the audio thread.
/// These are sent through a lock-free ring buffer for real-time safety.
#[derive(Debug, Clone)]
pub enum Command {
    // Transport commands
    Play,
    Stop,
    SetTempo(f64),

    // Deck commands
    /// Load a decoded audio buffer into a deck. The buffer is allocated and
    /// decoded off the audio thread, then handed over as a shared pointer so
    /// the hand-off itself is cheap.
    DeckLoad(DeckId, Arc<AudioBuffer>),
    DeckPlay(DeckId),
    DeckPause(DeckId),
    DeckStop(DeckId),
    DeckSeek(DeckId, u64),
    DeckSetPitch(DeckId, f64),
    DeckSetVolume(DeckId, f32),
    DeckSetSync(DeckId, SyncMode),
    /// Assign the track's tempo (BPM) for tempo syncing.
    DeckSetBpm(DeckId, f64),
    /// Set the grid's first-beat offset (source samples, may be negative). The
    /// deck starts playback from this position; negative values pre-roll silence.
    DeckSetFirstBeat(DeckId, i64),
    /// Activate a loop: grid-aligned start (source samples) + length in beats.
    DeckSetLoop(DeckId, i64, f64),
    /// Deactivate the loop.
    DeckClearLoop(DeckId),
    /// Jump to a cue point (source samples, may be negative). Quantized to the
    /// next bar boundary if the deck is playing, immediate if stopped.
    DeckCue(DeckId, i64),

    // Mixer commands
    SetChannelGain(usize, f32),
    SetChannelTrim(usize, f32),
    SetChannelEQ(usize, f32, f32, f32), // low, mid, high
    /// DJM-style HP/LP "colour" filter slider, value ∈ [-1, 1]; 0 = bypass,
    /// negative = LP (more negative = lower cutoff), positive = HP.
    SetChannelFilter(usize, f32),
    /// 2-D XY crossfader position: (x, y) each ∈ [-1, 1]. Decks A, B, C, D
    /// are pinned to the four corners (A=top-left, B=top-right, C=bottom-left,
    /// D=bottom-right); see `Mixer::xy_corner_gains` for the gain math.
    SetCrossfader(f32, f32),
    SetCrossfaderCurve(CrossfaderCurve),
    SetMasterGain(f32),

    // Clock / metronome
    /// Enable or disable the audible metronome click.
    SetMetronome(bool),
    /// Set the quantization grid for deck-start scheduling.
    SetQuantize(Quantize),

    /// Link Audio send routing: false (default) = post EQ/filter, true =
    /// raw deck output. Local monitoring is unchanged either way.
    SetLinkSendBypassEqFilter(bool),
}

/// Sender for commands (used by UI thread)
pub struct CommandSender {
    producer: rtrb::Producer<Command>,
}

impl CommandSender {
    pub fn new(producer: rtrb::Producer<Command>) -> Self {
        Self { producer }
    }

    /// Send a command to the audio thread.
    /// Returns true if successful, false if the buffer is full.
    pub fn send(&mut self, cmd: Command) -> bool {
        self.producer.push(cmd).is_ok()
    }
}

/// Receiver for commands (used by audio thread)
pub struct CommandReceiver {
    consumer: rtrb::Consumer<Command>,
}

impl CommandReceiver {
    pub fn new(consumer: rtrb::Consumer<Command>) -> Self {
        Self { consumer }
    }

    /// Process all pending commands, calling the handler for each.
    pub fn process<F>(&mut self, mut handler: F)
    where
        F: FnMut(Command),
    {
        while let Ok(cmd) = self.consumer.pop() {
            handler(cmd);
        }
    }
}

/// State updates sent from audio thread to UI thread
#[derive(Debug, Clone, Default)]
pub struct EngineState {
    /// Global transport position in samples
    pub position: u64,
    /// Current tempo
    pub tempo: f64,
    /// Whether transport is playing
    pub is_playing: bool,
    /// Current beat within the bar (0-3 in 4/4).
    pub beat_in_bar: u32,
    /// Phase within the current beat (0.0 to 1.0).
    pub beat_phase: f32,
    /// Deck states
    pub decks: [DeckState; 4],
    /// Mixer state
    pub mixer: MixerState,
    /// Ableton Link: whether the session is enabled (tempo/phase sync active).
    pub link_enabled: bool,
    /// Ableton Link: whether per-deck audio broadcast is active.
    pub link_audio_enabled: bool,
    /// Ableton Link: number of peers in the current session.
    pub link_peers: u64,
    /// Link Audio send routing: true = raw deck output (EQ/filter bypassed
    /// before the send), false = post-EQ/filter.
    pub link_send_bypass_eq_filter: bool,
}

#[derive(Debug, Clone, Default)]
pub struct DeckState {
    /// Current position in samples
    pub position: u64,
    /// Duration in samples
    pub duration: u64,
    /// Whether playing
    pub is_playing: bool,
    /// Current pitch ratio
    pub pitch: f64,
    /// Whether a loop is active
    pub loop_active: bool,
    /// Loop bounds in source samples (valid when loop_active)
    pub loop_start: u64,
    pub loop_end: u64,
}

#[derive(Debug, Clone, Default)]
pub struct MixerState {
    /// Channel peak meters [left, right] for each deck
    pub channel_peaks: [[f32; 2]; 4],
    /// Master peak meters
    pub master_peaks: [f32; 2],
    /// Current XY crossfader position (x, y), each ∈ [-1, 1].
    pub crossfader_xy: [f32; 2],
}

/// Create a command channel with the specified capacity
pub fn command_channel(capacity: usize) -> (CommandSender, CommandReceiver) {
    let (producer, consumer) = rtrb::RingBuffer::new(capacity);
    (CommandSender::new(producer), CommandReceiver::new(consumer))
}
