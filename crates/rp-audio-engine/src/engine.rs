use crate::{
    commands::{Command, CommandReceiver, DeckState, EngineState, MixerState},
    deck::{AudioBuffer, Deck},
    mixer::Mixer,
    transport::GlobalTransport,
};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Device, Stream, StreamConfig};
use rp_core::{DeckId, PlaybackState, Quantize, RecordPlayerError, Result};
use rp_link::{LinkAudio, LinkAudioSink, SessionState};
use std::sync::{Arc, Mutex};

/// Link "quantum" — the bar length in beats that all peers agree on for phase
/// alignment. 4 matches our 4/4 grid and our existing beat-in-bar UI.
const LINK_QUANTUM: f64 = 4.0;

/// Offset (in beats, must be a multiple of `LINK_QUANTUM`) added to Link's
/// beat timeline before mapping it back into our `u64` `sample_position`.
/// Link's beat-at-time can be slightly negative just after creation; this
/// offset keeps the converted sample position well into positive territory
/// without disturbing the bar phase used by the UI.
const LINK_BEAT_OFFSET: f64 = 4096.0; // 1024 bars

/// A short percussive click voice for the metronome. A single sine burst with a
/// linear decay, retriggered on every beat. Cheap enough to run in the audio
/// callback (no allocation, no locking).
struct Metronome {
    /// Whether the click is audible. The clock itself always runs.
    enabled: bool,
    /// Output sample rate.
    sample_rate: f32,
    /// Length of a click in samples.
    click_len: u32,
    /// Samples remaining in the current click (0 = silent).
    remaining: u32,
    /// Oscillator phase in cycles (0.0..1.0).
    phase: f32,
    /// Oscillator frequency for the current click.
    freq: f32,
    /// Output gain of the click.
    gain: f32,
}

impl Metronome {
    fn new(sample_rate: u32) -> Self {
        let sr = sample_rate as f32;
        Self {
            enabled: false,
            sample_rate: sr,
            click_len: (sr * 0.035) as u32, // 35 ms click
            remaining: 0,
            phase: 0.0,
            freq: 1000.0,
            gain: 0.4,
        }
    }

    fn set_sample_rate(&mut self, sample_rate: u32) {
        self.sample_rate = sample_rate as f32;
        self.click_len = (self.sample_rate * 0.035) as u32;
    }

    /// Begin a click. Downbeats (beat 0 of a bar) get a higher pitch.
    fn trigger(&mut self, downbeat: bool) {
        self.remaining = self.click_len;
        self.phase = 0.0;
        self.freq = if downbeat { 1568.0 } else { 988.0 };
    }

    /// Produce the next click sample, advancing the envelope. Returns 0.0 when
    /// no click is sounding.
    fn next_sample(&mut self) -> f32 {
        if self.remaining == 0 {
            return 0.0;
        }
        let env = self.remaining as f32 / self.click_len as f32;
        let sample = (self.phase * std::f32::consts::TAU).sin() * env * self.gain;
        self.phase += self.freq / self.sample_rate;
        if self.phase >= 1.0 {
            self.phase -= 1.0;
        }
        self.remaining -= 1;
        sample
    }
}

/// The main audio engine that manages playback
pub struct AudioEngine {
    // Drop order matters: `link_sinks` must drop before `link` (the sinks hold
    // raw handles that are only valid while LinkAudio is alive). Rust drops
    // fields in declaration order, so sinks come first.
    /// Per-deck LinkAudio sinks (broadcast). `None` until the device sample
    /// rate is known and the sinks have been created.
    link_sinks: [Option<LinkAudioSink>; 4],
    /// Scratch buffers for the f32→i16 conversion handed to LinkAudio. Sized
    /// to the largest cpal block we've seen.
    link_pcm_scratch: [Vec<i16>; 4],
    /// Pre-allocated Link session state. Created off the audio thread (the C
    /// API rejects construction from the audio thread); reused every block.
    link_state: SessionState,
    /// Shared handle to the LinkAudio instance — same `Arc` is held by the UI
    /// thread (AppState) so it can toggle enable/disable without going through
    /// the command channel.
    link: Arc<LinkAudio>,

    /// Global transport for sync
    transport: Arc<GlobalTransport>,
    /// The four decks
    decks: [Deck; 4],
    /// Main mixer
    mixer: Mixer,
    /// Command receiver for RT-safe communication
    commands: Option<CommandReceiver>,
    /// Shared snapshot of engine state, published for the UI thread to read.
    state_out: Option<Arc<Mutex<EngineState>>>,
    /// Metronome click voice, locked to the global clock.
    metronome: Metronome,
    /// Quantization grid for scheduling deck starts.
    quantize: Quantize,
    /// Pending quantized start position (global samples) for each deck, if any.
    pending_play: [Option<u64>; 4],
    /// Pending quantized cue jump per deck: (global-sample boundary, target frame).
    pending_seek: [Option<(u64, i64)>; 4],
    /// When true, the Link Audio send broadcasts the raw deck output
    /// (post-pitch/volume, pre-mixer). Default: the send is *post* EQ + filter,
    /// matching the audible booth feed — bypass switches it to the raw cue
    /// signal so remote peers receive an unprocessed channel they can shape
    /// themselves.
    link_send_bypass_eq_filter: bool,
}

impl AudioEngine {
    /// Create a new audio engine. The `link` argument is shared with the UI
    /// thread (see `AppState` in `src-tauri/src/main.rs`) so the UI can
    /// enable/disable Link directly. Link Audio sinks are created later in
    /// `set_output_sample_rate` once the device block size is known.
    pub fn new(sample_rate: u32, link: Arc<LinkAudio>) -> Self {
        Self {
            link_sinks: [const { None }; 4],
            link_pcm_scratch: [const { Vec::new() }; 4],
            link_state: SessionState::new(),
            link,

            transport: Arc::new(GlobalTransport::new(sample_rate)),
            decks: [
                Deck::new(DeckId::DECK_A),
                Deck::new(DeckId::DECK_B),
                Deck::new(DeckId::DECK_C),
                Deck::new(DeckId::DECK_D),
            ],
            mixer: Mixer::default(),
            commands: None,
            state_out: None,
            metronome: Metronome::new(sample_rate),
            quantize: Quantize::Off,
            pending_play: [None; 4],
            pending_seek: [None; 4],
            link_send_bypass_eq_filter: false,
        }
    }

    /// Set the command receiver for real-time communication
    pub fn set_command_receiver(&mut self, receiver: CommandReceiver) {
        self.commands = Some(receiver);
    }

    /// Adopt the real output device sample rate, propagating it to the clock,
    /// the metronome, and every deck so timing and resampling are correct.
    /// Also creates the per-deck LinkAudio sinks now that we know the device
    /// block-size class (sink creation is not realtime-safe and must happen
    /// before the audio thread is running).
    pub fn set_output_sample_rate(&mut self, sample_rate: u32) {
        self.transport.set_sample_rate(sample_rate);
        self.metronome.set_sample_rate(sample_rate);
        self.mixer.set_sample_rate(sample_rate);
        for deck in &mut self.decks {
            deck.set_output_sample_rate(sample_rate);
        }

        // Create one Link Audio sink per deck. `MAX_BLOCK_SAMPLES` is generous
        // (8192 i16 = 4096 stereo frames, ≈85 ms at 48 kHz) so any realistic
        // cpal buffer fits without us having to grow the pool at runtime.
        const MAX_BLOCK_SAMPLES: usize = 8192;
        const DECK_NAMES: [&str; 4] = [
            "recordplayer Deck A",
            "recordplayer Deck B",
            "recordplayer Deck C",
            "recordplayer Deck D",
        ];
        for d in 0..4 {
            if self.link_sinks[d].is_none() {
                self.link_sinks[d] =
                    Some(LinkAudioSink::new(&self.link, DECK_NAMES[d], MAX_BLOCK_SAMPLES));
                self.link_pcm_scratch[d] = vec![0i16; MAX_BLOCK_SAMPLES];
            }
        }
    }

    /// Provide a shared cell the engine publishes its latest state into so the
    /// UI thread can poll playback position, peaks, etc.
    pub fn set_state_output(&mut self, state_out: Arc<Mutex<EngineState>>) {
        self.state_out = Some(state_out);
    }

    /// Get a reference to the global transport
    pub fn transport(&self) -> &Arc<GlobalTransport> {
        &self.transport
    }

    /// Load audio into a deck
    pub fn load_deck(&mut self, deck_id: DeckId, audio: Arc<AudioBuffer>) {
        self.decks[deck_id.0 as usize].load(audio, None);
    }

    /// Process commands from the UI thread
    fn process_commands(&mut self) {
        // Collect commands first to avoid borrow issues
        let mut pending_commands = Vec::new();
        if let Some(ref mut commands) = self.commands {
            commands.process(|cmd| pending_commands.push(cmd));
        }
        // Then process them
        for cmd in pending_commands {
            self.handle_command(cmd);
        }
    }

    /// Handle a single command
    fn handle_command(&mut self, cmd: Command) {
        match cmd {
            Command::Play => self.transport.play(),
            Command::Stop => self.transport.stop(),
            Command::SetTempo(bpm) => {
                // With Link enabled, the local clock is reseeded from Link
                // every block — push tempo changes to the session so peers
                // follow them. The transport will pick up the new tempo on
                // the next render block's Link capture.
                if self.link.is_enabled() {
                    let host_us = self.link.clock_micros();
                    self.link_state.set_tempo(bpm, host_us);
                    self.link.commit_audio_session_state(&self.link_state);
                } else {
                    self.transport.set_tempo_bpm(bpm);
                }
            }

            Command::DeckLoad(id, audio) => {
                let d = id.0 as usize;
                self.decks[d].load(audio, None);
                self.pending_play[d] = None;
                self.pending_seek[d] = None;
            }
            Command::DeckPlay(id) => self.schedule_deck_play(id),
            Command::DeckPause(id) => {
                let d = id.0 as usize;
                self.decks[d].pause();
                self.pending_play[d] = None;
                self.pending_seek[d] = None;
            }
            Command::DeckStop(id) => {
                let d = id.0 as usize;
                self.decks[d].stop();
                self.pending_play[d] = None;
                self.pending_seek[d] = None;
            }
            Command::DeckSeek(id, pos) => self.decks[id.0 as usize].seek(pos),
            Command::DeckSetPitch(id, pitch) => self.decks[id.0 as usize].set_pitch(pitch),
            Command::DeckSetVolume(id, vol) => self.decks[id.0 as usize].set_volume(vol),
            Command::DeckSetSync(id, mode) => self.decks[id.0 as usize].set_sync_mode(mode),
            Command::DeckSetBpm(id, bpm) => self.decks[id.0 as usize].set_track_bpm(Some(bpm)),
            Command::DeckSetFirstBeat(id, fb) => self.decks[id.0 as usize].set_first_beat(fb),
            Command::DeckSetLoop(id, start, beats) => {
                self.decks[id.0 as usize].set_loop(start, beats)
            }
            Command::DeckClearLoop(id) => self.decks[id.0 as usize].clear_loop(),
            Command::DeckCue(id, target) => self.cue_deck(id, target),

            Command::SetChannelGain(ch, gain) => self.mixer.channels[ch].gain = gain,
            Command::SetChannelTrim(ch, trim) => self.mixer.channels[ch].trim = trim,
            Command::SetChannelEQ(ch, low, mid, high) => {
                self.mixer.channels[ch].eq_low = low;
                self.mixer.channels[ch].eq_mid = mid;
                self.mixer.channels[ch].eq_high = high;
            }
            Command::SetChannelFilter(ch, value) => {
                self.mixer.set_channel_filter(ch, value);
            }
            Command::SetCrossfader(x, y) => {
                self.mixer.crossfader_x = x;
                self.mixer.crossfader_y = y;
            }
            Command::SetCrossfaderCurve(curve) => self.mixer.crossfader_curve = curve,
            Command::SetMasterGain(gain) => self.mixer.master_gain = gain,

            Command::SetMetronome(on) => self.metronome.enabled = on,
            Command::SetQuantize(q) => self.quantize = q,

            Command::SetLinkSendBypassEqFilter(b) => self.link_send_bypass_eq_filter = b,
        }
    }

    /// Start a deck, snapping to the quantization grid when enabled. With
    /// quantization off (or the clock stopped) the deck starts immediately;
    /// otherwise we record the next grid boundary in global samples and the
    /// render loop flips the deck to playing exactly at that sample.
    fn schedule_deck_play(&mut self, id: DeckId) {
        let d = id.0 as usize;
        let beats = self.quantize.beats();

        if beats == 0.0 || !self.transport.is_playing() {
            self.decks[d].play();
            self.pending_play[d] = None;
            return;
        }

        let grid = self.transport.samples_per_beat() * beats;
        let pos = self.transport.sample_position() as f64;
        // First grid boundary strictly after the current position.
        let next = ((pos / grid).floor() + 1.0) * grid;
        self.pending_play[d] = Some(next.round() as u64);
    }

    /// Jump a deck to a cue point. Immediate when the deck is stopped; when it's
    /// playing, the jump is quantized to the next boundary (the global Quantize
    /// grid, defaulting to a bar) so phrasing stays aligned to the clock.
    fn cue_deck(&mut self, id: DeckId, target: i64) {
        let d = id.0 as usize;
        if self.decks[d].state() != PlaybackState::Playing {
            self.decks[d].cue(target);
            self.pending_seek[d] = None;
            return;
        }
        let beats = match self.quantize {
            Quantize::Beat => 1.0,
            _ => 4.0, // Bar (and a sensible default when quantize is Off)
        };
        let grid = self.transport.samples_per_beat() * beats;
        let pos = self.transport.sample_position() as f64;
        let boundary = ((pos / grid).floor() + 1.0) * grid;
        self.pending_seek[d] = Some((boundary.round() as u64, target));
    }

    /// Render audio samples. Called from the audio callback.
    pub fn render(&mut self, output: &mut [f32], frames: usize) {
        // ─── Adopt Link timeline (must happen BEFORE process_commands so any
        // SetTempo command sees the freshly-captured session_state) ──────────
        //
        // When Link is enabled the GlobalTransport stops being a free-running
        // counter — every block we re-derive its tempo and sample_position
        // from the captured Link session state. Downstream beat/bar math and
        // the quantize/pending_play/pending_seek machinery then "just works"
        // against the reseeded clock without any further changes.
        let link_enabled = self.link.is_enabled();
        let link_host_us = if link_enabled {
            let host_us = self.link.clock_micros();
            self.link.capture_audio_session_state(&mut self.link_state);
            let bpm = self.link_state.tempo();
            self.transport.set_tempo_bpm(bpm);
            // Map Link beats → our u64 sample_position. The offset is a
            // multiple of LINK_QUANTUM so bar phase (what the UI shows) is
            // preserved across the conversion.
            let sr = self.transport.sample_rate() as f64;
            let spb = sr * 60.0 / bpm;
            let beat = self.link_state.beat_at_time(host_us, LINK_QUANTUM);
            let positive_beat = (beat + LINK_BEAT_OFFSET).max(0.0);
            self.transport
                .set_sample_position((positive_beat * spb).round() as u64);
            Some(host_us)
        } else {
            None
        };

        // Process any pending commands
        self.process_commands();

        // Global clock position at the start of this block and the beat length.
        let block_start = self.transport.sample_position();
        let playing = self.transport.is_playing();
        let samples_per_beat = self.transport.samples_per_beat();

        // Tempo-synced decks track the global BPM (recomputed each block so they
        // follow tempo changes).
        let global_bpm = self.transport.tempo_bpm();
        for deck in &mut self.decks {
            deck.apply_tempo_sync(global_bpm);
        }

        // Scratch buffers for each deck (frames * 2 for stereo)
        let mut deck_scratch: [Vec<f32>; 4] = [
            vec![0.0; frames * 2],
            vec![0.0; frames * 2],
            vec![0.0; frames * 2],
            vec![0.0; frames * 2],
        ];

        // Render each deck, honoring any pending quantized start. A deck with a
        // start scheduled inside this block renders silence up to that sample,
        // then flips to playing, so playback begins sample-accurately on the grid.
        let block_end = block_start + frames as u64;
        for d in 0..4 {
            // Quantized start: a stopped deck begins playback on the grid.
            if let Some(start) = self.pending_play[d] {
                if playing && start < block_end {
                    if start <= block_start {
                        // Boundary already reached; start at the top of the block.
                        self.decks[d].play();
                        self.decks[d].render(&mut deck_scratch[d], frames);
                    } else {
                        let split = (start - block_start) as usize;
                        // Silence before the boundary (deck is still stopped).
                        self.decks[d].render(&mut deck_scratch[d][..split * 2], split);
                        self.decks[d].play();
                        self.decks[d].render(&mut deck_scratch[d][split * 2..], frames - split);
                    }
                    self.pending_play[d] = None;
                    continue;
                }
            }

            // Quantized cue jump: a playing deck seeks to the cue on the grid.
            if let Some((boundary, target)) = self.pending_seek[d] {
                if boundary < block_end {
                    if boundary <= block_start {
                        self.decks[d].cue(target);
                        self.decks[d].render(&mut deck_scratch[d], frames);
                    } else {
                        let split = (boundary - block_start) as usize;
                        // Play from the current position up to the boundary…
                        self.decks[d].render(&mut deck_scratch[d][..split * 2], split);
                        // …then jump to the cue and continue from there.
                        self.decks[d].cue(target);
                        self.decks[d].render(&mut deck_scratch[d][split * 2..], frames - split);
                    }
                    self.pending_seek[d] = None;
                    continue;
                }
            }

            self.decks[d].render(&mut deck_scratch[d], frames);
        }

        // ─── Pre-process each deck through EQ + filter ───────────────────────
        //
        // We need the post-EQ/filter signal for both the Link Audio send (so
        // remote peers hear the booth feed, not the dry cue) and the per-frame
        // crossfader/master sum below. Running the biquads once here and
        // re-using the result avoids double-processing.
        let mut processed_scratch: [Vec<f32>; 4] = [
            vec![0.0; frames * 2],
            vec![0.0; frames * 2],
            vec![0.0; frames * 2],
            vec![0.0; frames * 2],
        ];
        for d in 0..4 {
            for frame in 0..frames {
                let l = deck_scratch[d][frame * 2];
                let r = deck_scratch[d][frame * 2 + 1];
                let (pl, pr) = self.mixer.process_channel_eq_filter(d, l, r);
                processed_scratch[d][frame * 2] = pl;
                processed_scratch[d][frame * 2 + 1] = pr;
            }
        }

        // ─── Broadcast per-deck audio over Link Audio ────────────────────────
        //
        // Default source is the post-EQ/filter signal (what the user hears in
        // the booth, minus crossfader/master). Toggling
        // `link_send_bypass_eq_filter` switches the send back to the raw deck
        // output so peers can apply their own processing. Idle (no remote
        // subscriber) is cheap: `retain_buffer` returns None and we bail.
        if link_enabled && self.link.is_link_audio_enabled() {
            if let Some(host_us) = link_host_us {
                let beats_at_buffer_begin =
                    self.link_state.beat_at_time(host_us, LINK_QUANTUM);
                let sample_rate = self.transport.sample_rate();
                let bypass = self.link_send_bypass_eq_filter;
                for d in 0..4 {
                    let Some(sink) = self.link_sinks[d].as_ref() else {
                        continue;
                    };
                    let Some(mut buf) = sink.retain_buffer() else {
                        continue; // no remote source subscribed
                    };
                    let needed = frames * 2;
                    let max = buf.max_num_samples().min(self.link_pcm_scratch[d].len());
                    if needed > max {
                        // Cpal handed us a larger block than our pool — skip
                        // this block rather than truncate misaligned audio.
                        continue;
                    }
                    let src: &[f32] = if bypass {
                        &deck_scratch[d][..needed]
                    } else {
                        &processed_scratch[d][..needed]
                    };
                    let dst = &mut self.link_pcm_scratch[d][..needed];
                    for (i, s) in src.iter().enumerate() {
                        // f32 [-1, 1] → i16 with saturation. 32767 (not 32768)
                        // on the negative side mirrors libsndfile/cpal’s convention.
                        let clamped = s.clamp(-1.0, 1.0);
                        dst[i] = (clamped * 32767.0) as i16;
                    }
                    buf.samples()[..needed].copy_from_slice(dst);
                    let _ = buf.commit(
                        &self.link_state,
                        beats_at_buffer_begin,
                        LINK_QUANTUM,
                        frames,
                        2,
                        sample_rate,
                    );
                }
            }
        }

        // Mix decks and overlay the metronome, frame by frame.
        for frame in 0..frames {
            let processed_samples: [[f32; 2]; 4] = [
                [processed_scratch[0][frame * 2], processed_scratch[0][frame * 2 + 1]],
                [processed_scratch[1][frame * 2], processed_scratch[1][frame * 2 + 1]],
                [processed_scratch[2][frame * 2], processed_scratch[2][frame * 2 + 1]],
                [processed_scratch[3][frame * 2], processed_scratch[3][frame * 2 + 1]],
            ];

            let mut mixed = [0.0f32; 2];
            self.mixer.combine(&processed_samples, &mut mixed);

            if playing {
                // Trigger a click at each beat boundary crossed in this frame.
                let g = block_start + frame as u64;
                let cur_beat = (g as f64 / samples_per_beat).floor() as i64;
                let prev_beat = if g == 0 {
                    -1
                } else {
                    ((g - 1) as f64 / samples_per_beat).floor() as i64
                };
                if cur_beat != prev_beat && self.metronome.enabled {
                    self.metronome.trigger(cur_beat.rem_euclid(4) == 0);
                }
                let click = self.metronome.next_sample();
                mixed[0] += click;
                mixed[1] += click;
            }

            output[frame * 2] = mixed[0];
            output[frame * 2 + 1] = mixed[1];
        }

        // ─── Local output mute when broadcasting over Link Audio ─────────────
        //
        // When Link Audio is on, recordplayer is acting as the Link source —
        // peers monitor the audio, and the local speakers should stay silent
        // so the operator isn't doubling on a downstream monitor. The deck
        // broadcast loop above ran on the pre-mute signal, so peers still get
        // their full audio; only the cpal output is zeroed. Metronome and
        // master peak meters were already computed before this point, so
        // beat-dot animation and meter values keep working.
        if link_enabled && self.link.is_link_audio_enabled() {
            for s in output[..frames * 2].iter_mut() {
                *s = 0.0;
            }
        }

        // Advance the free-running clock.
        if playing {
            self.transport.advance(frames as u64);
        }

        // Decay meters
        self.mixer.decay_meters(0.95);

        // Publish a fresh state snapshot for the UI thread. We never block the
        // audio callback: if the reader holds the lock we simply skip this tick.
        if let Some(out) = &self.state_out {
            if let Ok(mut snapshot) = out.try_lock() {
                *snapshot = self.state();
            }
        }
    }

    /// Get the current engine state for UI updates
    pub fn state(&self) -> EngineState {
        let beat_position = self.transport.beat_position();
        EngineState {
            position: self.transport.sample_position(),
            tempo: self.transport.tempo_bpm(),
            is_playing: self.transport.is_playing(),
            beat_in_bar: (beat_position.floor() as i64).rem_euclid(4) as u32,
            beat_phase: beat_position.fract() as f32,
            decks: [
                self.deck_state(0),
                self.deck_state(1),
                self.deck_state(2),
                self.deck_state(3),
            ],
            mixer: MixerState {
                channel_peaks: [
                    [
                        self.mixer.channels[0].peak_left,
                        self.mixer.channels[0].peak_right,
                    ],
                    [
                        self.mixer.channels[1].peak_left,
                        self.mixer.channels[1].peak_right,
                    ],
                    [
                        self.mixer.channels[2].peak_left,
                        self.mixer.channels[2].peak_right,
                    ],
                    [
                        self.mixer.channels[3].peak_left,
                        self.mixer.channels[3].peak_right,
                    ],
                ],
                master_peaks: [self.mixer.master_peak_left, self.mixer.master_peak_right],
                crossfader_xy: [self.mixer.crossfader_x, self.mixer.crossfader_y],
            },
            link_enabled: self.link.is_enabled(),
            link_audio_enabled: self.link.is_link_audio_enabled(),
            link_peers: self.link.num_peers(),
            link_send_bypass_eq_filter: self.link_send_bypass_eq_filter,
        }
    }

    fn deck_state(&self, index: usize) -> DeckState {
        let deck = &self.decks[index];
        DeckState {
            position: deck.position(),
            duration: deck.duration_samples(),
            is_playing: deck.state() == PlaybackState::Playing,
            pitch: deck.pitch_ratio(),
            loop_active: deck.loop_active(),
            loop_start: deck.loop_start(),
            loop_end: deck.loop_end(),
        }
    }
}

/// Start the audio engine and return the stream
pub fn start_audio(mut engine: AudioEngine) -> Result<(Stream, Arc<GlobalTransport>)> {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or_else(|| RecordPlayerError::AudioDevice("No output device found".into()))?;

    let config = device
        .default_output_config()
        .map_err(|e| RecordPlayerError::AudioDevice(e.to_string()))?;

    // Adopt the device's real sample rate so the clock and resampling are correct.
    engine.set_output_sample_rate(config.sample_rate().0);
    let transport = engine.transport().clone();

    let stream = match config.sample_format() {
        cpal::SampleFormat::F32 => build_stream::<f32>(&device, &config.into(), engine)?,
        cpal::SampleFormat::I16 => build_stream::<i16>(&device, &config.into(), engine)?,
        cpal::SampleFormat::U16 => build_stream::<u16>(&device, &config.into(), engine)?,
        _ => return Err(RecordPlayerError::UnsupportedFormat("Unknown sample format".into())),
    };

    stream
        .play()
        .map_err(|e| RecordPlayerError::AudioDevice(e.to_string()))?;

    Ok((stream, transport))
}

fn build_stream<T: cpal::Sample + cpal::SizedSample + cpal::FromSample<f32>>(
    device: &Device,
    config: &StreamConfig,
    mut engine: AudioEngine,
) -> Result<Stream> {
    let channels = config.channels as usize;
    let mut buffer = vec![0.0f32; 4096];

    let stream = device
        .build_output_stream(
            config,
            move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
                let frames = data.len() / channels;

                // Ensure buffer is large enough
                if buffer.len() < frames * 2 {
                    buffer.resize(frames * 2, 0.0);
                }

                // Render audio
                engine.render(&mut buffer, frames);

                // Convert to output format
                for (i, sample) in data.iter_mut().enumerate() {
                    *sample = T::from_sample(buffer[i]);
                }
            },
            |err| eprintln!("Audio error: {}", err),
            None,
        )
        .map_err(|e| RecordPlayerError::AudioDevice(e.to_string()))?;

    Ok(stream)
}
