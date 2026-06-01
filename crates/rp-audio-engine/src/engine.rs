use crate::{
    commands::{Command, CommandReceiver, DeckState, EngineState, MixerState},
    deck::{AudioBuffer, Deck},
    mixer::Mixer,
    transport::GlobalTransport,
};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Device, Stream, StreamConfig};
use rp_core::{DeckId, PlaybackState, Quantize, RecordPlayerError, Result};
use std::sync::{Arc, Mutex};

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
}

impl AudioEngine {
    /// Create a new audio engine
    pub fn new(sample_rate: u32) -> Self {
        Self {
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
        }
    }

    /// Set the command receiver for real-time communication
    pub fn set_command_receiver(&mut self, receiver: CommandReceiver) {
        self.commands = Some(receiver);
    }

    /// Adopt the real output device sample rate, propagating it to the clock,
    /// the metronome, and every deck so timing and resampling are correct.
    pub fn set_output_sample_rate(&mut self, sample_rate: u32) {
        self.transport.set_sample_rate(sample_rate);
        self.metronome.set_sample_rate(sample_rate);
        self.mixer.set_sample_rate(sample_rate);
        for deck in &mut self.decks {
            deck.set_output_sample_rate(sample_rate);
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
            Command::SetTempo(bpm) => self.transport.set_tempo_bpm(bpm),

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

        // Mix decks and overlay the metronome, frame by frame.
        for frame in 0..frames {
            let deck_samples: [[f32; 2]; 4] = [
                [deck_scratch[0][frame * 2], deck_scratch[0][frame * 2 + 1]],
                [deck_scratch[1][frame * 2], deck_scratch[1][frame * 2 + 1]],
                [deck_scratch[2][frame * 2], deck_scratch[2][frame * 2 + 1]],
                [deck_scratch[3][frame * 2], deck_scratch[3][frame * 2 + 1]],
            ];

            let mut mixed = [0.0f32; 2];
            self.mixer.mix(&deck_samples, &mut mixed);

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
