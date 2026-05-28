use rp_core::{BeatGrid, DeckId, PlaybackState, SyncMode};
use std::sync::Arc;

/// Audio buffer containing decoded samples for a track
pub struct AudioBuffer {
    /// Interleaved stereo samples (L, R, L, R, ...)
    pub samples: Vec<f32>,
    /// Sample rate of the audio
    pub sample_rate: u32,
    /// Number of channels (typically 2)
    pub channels: u16,
}

impl std::fmt::Debug for AudioBuffer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Avoid printing the (potentially huge) sample buffer.
        f.debug_struct("AudioBuffer")
            .field("frames", &self.frame_count())
            .field("sample_rate", &self.sample_rate)
            .field("channels", &self.channels)
            .finish()
    }
}

impl AudioBuffer {
    pub fn new(samples: Vec<f32>, sample_rate: u32, channels: u16) -> Self {
        Self {
            samples,
            sample_rate,
            channels,
        }
    }

    /// Get the total number of frames (samples per channel)
    pub fn frame_count(&self) -> usize {
        self.samples.len() / self.channels as usize
    }

    /// Get a frame at a given index
    pub fn get_frame(&self, index: usize) -> Option<(f32, f32)> {
        let idx = index * self.channels as usize;
        if idx + 1 < self.samples.len() {
            Some((self.samples[idx], self.samples[idx + 1]))
        } else {
            None
        }
    }
}

/// A deck for playing back audio with sync support
pub struct Deck {
    /// Deck identifier
    pub id: DeckId,

    /// Loaded audio buffer (None if no track loaded)
    audio: Option<Arc<AudioBuffer>>,

    /// Current playback position in source frames (fractional, for resampling)
    position: f64,

    /// Output device sample rate, used for sample-rate conversion
    output_sample_rate: u32,

    /// Assigned track tempo (BPM), used for tempo sync
    track_bpm: Option<f64>,

    /// Grid first-beat (downbeat) offset in source samples. Playback homes
    /// here: positive skips the lead-in, negative pre-rolls silence.
    first_beat: i64,

    /// Loop state. Bounds are in source frames; the read cursor wraps from
    /// `loop_end` back to `loop_start` while active.
    loop_active: bool,
    loop_start: f64,
    loop_end: f64,

    /// Playback state
    state: PlaybackState,

    /// Pitch/tempo ratio (1.0 = normal speed)
    pitch_ratio: f64,

    /// Sync mode
    sync_mode: SyncMode,

    /// Offset from global clock when synced (in samples)
    sync_offset: i64,

    /// Beatgrid for this track
    beatgrid: Option<BeatGrid>,

    /// Volume (0.0 to 1.0)
    volume: f32,
}

impl Deck {
    pub fn new(id: DeckId) -> Self {
        Self {
            id,
            audio: None,
            position: 0.0,
            output_sample_rate: 44100,
            track_bpm: None,
            first_beat: 0,
            loop_active: false,
            loop_start: 0.0,
            loop_end: 0.0,
            state: PlaybackState::Stopped,
            pitch_ratio: 1.0,
            sync_mode: SyncMode::Off,
            sync_offset: 0,
            beatgrid: None,
            volume: 1.0,
        }
    }

    /// Load audio into this deck
    pub fn load(&mut self, audio: Arc<AudioBuffer>, beatgrid: Option<BeatGrid>) {
        self.audio = Some(audio);
        self.beatgrid = beatgrid;
        self.position = self.first_beat as f64;
        self.loop_active = false;
        self.state = PlaybackState::Stopped;
    }

    /// Unload the current track
    pub fn unload(&mut self) {
        self.audio = None;
        self.beatgrid = None;
        self.position = 0.0;
        self.state = PlaybackState::Stopped;
    }

    /// Start playback
    pub fn play(&mut self) {
        if self.audio.is_some() {
            self.state = PlaybackState::Playing;
        }
    }

    /// Pause playback
    pub fn pause(&mut self) {
        self.state = PlaybackState::Paused;
    }

    /// Stop playback and reset to the grid home position (the downbeat).
    pub fn stop(&mut self) {
        self.state = PlaybackState::Stopped;
        self.position = self.first_beat as f64;
    }

    /// Seek to a position in source frames
    pub fn seek(&mut self, position: u64) {
        self.position = position as f64;
    }

    /// Jump to a cue position in source frames (may be negative → pre-roll).
    pub fn cue(&mut self, position: i64) {
        self.position = position as f64;
    }

    /// Set the pitch ratio (1.0 = original tempo/pitch). Allows half/double time.
    pub fn set_pitch(&mut self, ratio: f64) {
        self.pitch_ratio = ratio.clamp(0.25, 4.0);
    }

    /// Set the sync mode
    pub fn set_sync_mode(&mut self, mode: SyncMode) {
        self.sync_mode = mode;
    }

    /// Set the output device sample rate (for sample-rate conversion).
    pub fn set_output_sample_rate(&mut self, sample_rate: u32) {
        self.output_sample_rate = sample_rate;
    }

    /// Assign the track's tempo, used when tempo-syncing to the global clock.
    pub fn set_track_bpm(&mut self, bpm: Option<f64>) {
        self.track_bpm = bpm;
    }

    /// The assigned track tempo, if any.
    pub fn track_bpm(&self) -> Option<f64> {
        self.track_bpm
    }

    /// Set the grid first-beat offset (source samples, may be negative). While
    /// stopped, re-home the cursor to it so the next play begins on the downbeat.
    pub fn set_first_beat(&mut self, first_beat: i64) {
        self.first_beat = first_beat;
        if self.state == PlaybackState::Stopped {
            self.position = first_beat as f64;
        }
    }

    /// When tempo-synced, set the pitch ratio so the track plays at `global_bpm`.
    /// With sync off, the manually-set pitch is left untouched.
    pub fn apply_tempo_sync(&mut self, global_bpm: f64) {
        if self.sync_mode == SyncMode::Off {
            return;
        }
        if let Some(track_bpm) = self.track_bpm {
            if track_bpm > 0.0 {
                self.set_pitch(global_bpm / track_bpm);
            }
        }
    }

    /// Activate a loop of `beats` beats starting at `start` (source samples,
    /// grid-aligned by the caller). Length is derived from the track BPM, so the
    /// loop is sample-accurate to the grid. No-op without a loaded track + BPM.
    pub fn set_loop(&mut self, start: i64, beats: f64) {
        let (sr, total) = match &self.audio {
            Some(a) => (a.sample_rate as f64, a.frame_count() as f64),
            None => return,
        };
        let bpm = match self.track_bpm {
            Some(b) if b > 0.0 => b,
            _ => return,
        };
        if beats <= 0.0 {
            return;
        }
        let samples_per_beat = sr * 60.0 / bpm;
        let start = (start as f64).max(0.0);
        let end = (start + beats * samples_per_beat).min(total);
        if end <= start {
            return;
        }
        self.loop_start = start;
        self.loop_end = end;
        self.loop_active = true;
    }

    /// Deactivate the loop.
    pub fn clear_loop(&mut self) {
        self.loop_active = false;
    }

    pub fn loop_active(&self) -> bool {
        self.loop_active
    }

    pub fn loop_start(&self) -> u64 {
        self.loop_start.max(0.0) as u64
    }

    pub fn loop_end(&self) -> u64 {
        self.loop_end.max(0.0) as u64
    }

    /// Set the volume
    pub fn set_volume(&mut self, volume: f32) {
        self.volume = volume.clamp(0.0, 1.0);
    }

    /// Get the current position in source frames (clamped to >= 0; negative
    /// during pre-roll silence reports as 0).
    pub fn position(&self) -> u64 {
        self.position.max(0.0) as u64
    }

    /// Get the playback state
    pub fn state(&self) -> PlaybackState {
        self.state
    }

    /// Get the pitch ratio
    pub fn pitch_ratio(&self) -> f64 {
        self.pitch_ratio
    }

    /// Check if a track is loaded
    pub fn is_loaded(&self) -> bool {
        self.audio.is_some()
    }

    /// Get the duration in samples
    pub fn duration_samples(&self) -> u64 {
        self.audio
            .as_ref()
            .map(|a| a.frame_count() as u64)
            .unwrap_or(0)
    }

    /// Render audio samples into the output buffer.
    /// Returns the number of frames rendered.
    pub fn render(&mut self, output: &mut [f32], frames: usize) -> usize {
        let Some(audio) = &self.audio else {
            // No audio loaded, fill with silence
            output[..frames * 2].fill(0.0);
            return frames;
        };

        if self.state != PlaybackState::Playing {
            // Not playing, fill with silence
            output[..frames * 2].fill(0.0);
            return frames;
        }

        let mut rendered = 0;
        let total_frames = audio.frame_count();

        // Read increment per output frame: corrects for the source-vs-output
        // sample-rate difference, then applies the pitch/tempo ratio.
        let step = (audio.sample_rate as f64 / self.output_sample_rate as f64) * self.pitch_ratio;

        for i in 0..frames {
            // Sample-accurate loop wrap: when the cursor reaches the loop end,
            // fold it back, preserving the fractional overshoot for a seamless
            // (click-free) loop.
            if self.loop_active && self.loop_end > self.loop_start {
                while self.position >= self.loop_end {
                    self.position -= self.loop_end - self.loop_start;
                }
            }

            // Pre-roll silence for a negative grid offset: the downbeat sits
            // before the audio, so play silence until the cursor reaches 0.
            if self.position < 0.0 {
                output[i * 2] = 0.0;
                output[i * 2 + 1] = 0.0;
                self.position += step;
                continue;
            }

            let idx = self.position.floor() as usize;
            if idx >= total_frames {
                // End of track
                output[i * 2] = 0.0;
                output[i * 2 + 1] = 0.0;
                continue;
            }

            // Linear interpolation between the two surrounding source frames.
            let frac = (self.position - idx as f64) as f32;
            let (l0, r0) = audio.get_frame(idx).unwrap_or((0.0, 0.0));
            let (l1, r1) = audio
                .get_frame((idx + 1).min(total_frames - 1))
                .unwrap_or((l0, r0));
            let left = l0 + (l1 - l0) * frac;
            let right = r0 + (r1 - r0) * frac;

            output[i * 2] = left * self.volume;
            output[i * 2 + 1] = right * self.volume;
            rendered += 1;

            self.position += step;
        }

        rendered
    }

    /// Calculate the effective playback position considering sync
    pub fn calculate_sync_position(&self, global_sample_pos: u64) -> u64 {
        match self.sync_mode {
            SyncMode::Off => self.position.max(0.0) as u64,
            SyncMode::Tempo | SyncMode::Phase => {
                // Apply sync offset to global position
                let synced = global_sample_pos as i64 + self.sync_offset;
                synced.max(0) as u64
            }
        }
    }
}
