#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::Path;
use std::sync::{Arc, Mutex};

use rp_app::AudioDecoder;
use rp_audio_engine::commands::EngineState;
use rp_audio_engine::deck::AudioBuffer;
use rp_audio_engine::{command_channel, AudioEngine, Command, CommandSender, start_audio};
use rp_core::{AudioFormat, DeckId, Quantize, SyncMode, TrackId, TrackMetadata};
use rp_library::TrackDatabase;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_dialog::DialogExt;

/// Shared application state available to every Tauri command.
struct AppState {
    /// Sends real-time commands to the audio thread.
    command_sender: Mutex<CommandSender>,
    /// Latest engine state, published by the audio thread.
    engine_state: Arc<Mutex<EngineState>>,
    /// Persistent track library.
    database: Mutex<TrackDatabase>,
    /// Decoded audio per deck (shared `Arc` with the audio thread, so no extra
    /// memory) used to compute waveform peaks over any sample range on demand.
    waveforms: Mutex<[Option<Arc<AudioBuffer>>; 4]>,
}

impl AppState {
    /// Push a command onto the audio thread's ring buffer.
    fn send(&self, cmd: Command) -> Result<(), String> {
        let mut sender = self.command_sender.lock().map_err(|e| e.to_string())?;
        if !sender.send(cmd) {
            return Err("audio command queue is full".into());
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Serializable data transferred to the frontend
// ---------------------------------------------------------------------------

#[derive(serde::Serialize)]
struct DeckSnapshot {
    position: u64,
    duration: u64,
    is_playing: bool,
    pitch: f64,
    peak_left: f32,
    peak_right: f32,
    loop_active: bool,
    loop_start: u64,
    loop_end: u64,
}

#[derive(serde::Serialize)]
struct EngineSnapshot {
    position: u64,
    tempo: f64,
    is_playing: bool,
    beat_in_bar: u32,
    beat_phase: f32,
    /// XY crossfader position (x, y), each ∈ [-1, 1].
    crossfader_xy: [f32; 2],
    master_left: f32,
    master_right: f32,
    decks: Vec<DeckSnapshot>,
}

#[derive(serde::Serialize, Clone)]
struct TrackDto {
    id: u64,
    path: String,
    title: Option<String>,
    artist: Option<String>,
    album: Option<String>,
    duration_samples: u64,
    sample_rate: u32,
    channels: u16,
    bpm: Option<f64>,
    /// First-beat (downbeat) offset in samples for the beat grid; 0 if unset.
    /// May be negative (downbeat before sample 0 → pre-roll silence).
    first_beat: i64,
    /// Saved loop start in samples (0 if none).
    loop_start: i64,
    /// Saved loop length in beats (0 = no saved loop).
    loop_beats: f64,
    /// Saved cue points (hot cues 1-8) in source samples; null = unset.
    cues: Vec<Option<i64>>,
    /// Low-res RMS overview (0..1 per bucket) for the library thumbnail.
    waveform: Vec<f32>,
}

impl From<&TrackMetadata> for TrackDto {
    fn from(t: &TrackMetadata) -> Self {
        Self {
            id: t.id.0,
            path: t.path.to_string_lossy().into_owned(),
            title: t.title.clone(),
            artist: t.artist.clone(),
            album: t.album.clone(),
            duration_samples: t.duration_samples,
            sample_rate: t.format.sample_rate,
            channels: t.format.channels,
            bpm: t.bpm,
            first_beat: t.first_beat.unwrap_or(0),
            loop_start: t.loop_start.unwrap_or(0),
            loop_beats: t.loop_beats.unwrap_or(0.0),
            cues: t.cues.to_vec(),
            waveform: t
                .waveform
                .as_ref()
                .map(|b| b.iter().map(|&v| v as f32 / 255.0).collect())
                .unwrap_or_default(),
        }
    }
}

/// Compute interleaved min/max waveform peaks for a sample range.
///
/// `[start, end)` is in source frames; the range is split into `pixels` buckets
/// and each bucket's min and max (mono-mixed, -1.0..=1.0) are emitted as a pair.
/// The UI fills each column from the max down to the zero line and from the min
/// up to it, giving a continuous filled bipolar waveform. Buckets shrink as the
/// window narrows, so zooming in yields progressively finer detail — down to
/// per-sample at full zoom. To stay fast at coarse zoom, each bucket examines at
/// most ~`MAX_BUCKET_SAMPLES` frames (invisible subsampling, still catches the
/// envelope on continuous audio); at fine zoom buckets are small and fully scanned.
fn peaks_for_range(samples: &[f32], channels: u16, start: u64, end: u64, pixels: u32) -> Vec<f32> {
    const MAX_BUCKET_SAMPLES: usize = 1024;
    let ch = channels.max(1) as usize;
    let total = samples.len() / ch;
    let start = (start as usize).min(total);
    let end = (end as usize).clamp(start, total);
    let span = end - start;
    let pixels = pixels.clamp(1, 8000) as usize;

    let mut out = Vec::with_capacity(pixels * 2);
    if span == 0 {
        return out;
    }

    let frame_mono = |f: usize| -> f32 {
        let base = f * ch;
        let mut sum = 0.0f32;
        for c in 0..ch {
            sum += samples[base + c];
        }
        sum / ch as f32
    };

    for p in 0..pixels {
        let b0 = start + (p * span) / pixels;
        let mut b1 = start + ((p + 1) * span) / pixels;
        if b1 <= b0 {
            b1 = (b0 + 1).min(end); // very deep zoom: at least one frame per pixel
        }
        let step = ((b1 - b0) / MAX_BUCKET_SAMPLES).max(1);

        let mut min = f32::MAX;
        let mut max = f32::MIN;
        let mut f = b0;
        while f < b1 {
            let v = frame_mono(f);
            if v < min {
                min = v;
            }
            if v > max {
                max = v;
            }
            f += step;
        }
        if min > max {
            min = 0.0;
            max = 0.0;
        }
        out.push(min.clamp(-1.0, 1.0));
        out.push(max.clamp(-1.0, 1.0));
    }
    out
}

// ---------------------------------------------------------------------------
// Library + loading
// ---------------------------------------------------------------------------

/// Open a native file picker and return the chosen path, if any.
///
/// This is an `async` command (so Tauri runs it off the main thread) and the
/// blocking dialog call is further moved onto a blocking worker. The native
/// panel must be shown from the main thread; if we blocked the main thread
/// waiting for it here, the app would deadlock.
#[tauri::command]
async fn open_track_dialog(app: AppHandle) -> Option<String> {
    tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .add_filter("Audio", &["mp3", "flac", "wav", "aac", "ogg", "m4a", "aiff"])
            .blocking_pick_file()
            .and_then(|fp| fp.as_path().map(|p| p.to_string_lossy().into_owned()))
    })
    .await
    .ok()
    .flatten()
}

/// Decode a file, index it in the library, keep its samples for waveform
/// queries, and hand the decoded buffer to the requested deck.
#[tauri::command]
fn load_track(deck: u8, path: String, state: State<AppState>) -> Result<TrackDto, String> {
    let path_buf = Path::new(&path).to_path_buf();

    // Decode off the audio thread. This reads the whole file into memory. Wrap
    // in an Arc so the same samples back both playback and waveform rendering.
    let buffer = Arc::new(AudioDecoder::decode(&path_buf).map_err(|e| e.to_string())?);

    let sample_rate = buffer.sample_rate;
    let channels = buffer.channels;
    let frame_count = buffer.frame_count() as u64;

    // Low-res RMS overview cached for the library thumbnail (one byte/bucket).
    let overview: Vec<u8> = rms_for_range(&buffer.samples, channels, 0, frame_count, 400)
        .iter()
        .map(|r| (r * 255.0).round().clamp(0.0, 255.0) as u8)
        .collect();

    // Index the track in the library (filename stem as a fallback title).
    let title = path_buf
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned());

    let metadata = TrackMetadata {
        id: TrackId(0),
        path: path_buf,
        title,
        artist: None,
        album: None,
        duration_samples: frame_count,
        format: AudioFormat {
            sample_rate,
            channels,
            bits_per_sample: 16,
        },
        bpm: None,
        first_beat: None,
        loop_start: None,
        loop_beats: None,
        cues: [None; 8],
        waveform: Some(overview),
    };

    // Upsert and read back the persisted row, which may carry a BPM the user
    // assigned on a previous load (preserved by the COALESCE in `upsert`).
    let track = {
        let db = state.database.lock().map_err(|e| e.to_string())?;
        let id = db.upsert(&metadata).map_err(|e| e.to_string())?;
        db.get(id).map_err(|e| e.to_string())?.unwrap_or(metadata)
    };

    // First time we see this track (no BPM persisted): auto-detect, write it
    // back, and re-read the row so the DTO + downstream engine command see it.
    let track = if track.bpm.is_none() {
        let detected = rp_analysis::BpmDetector::new(sample_rate)
            .detect(&buffer.samples, channels);
        if let Some(bpm) = detected {
            let db = state.database.lock().map_err(|e| e.to_string())?;
            db.set_bpm(track.id, bpm).map_err(|e| e.to_string())?;
            db.get(track.id).map_err(|e| e.to_string())?.unwrap_or(track)
        } else {
            track
        }
    } else {
        track
    };

    // Keep a shared handle to the samples for waveform range queries.
    {
        let mut waveforms = state.waveforms.lock().map_err(|e| e.to_string())?;
        waveforms[deck as usize] = Some(buffer.clone());
    }

    // Hand the decoded audio to the deck and (re)assign its tempo + grid offset.
    // The 0 defaults clear any stale values from a previously-loaded track.
    state.send(Command::DeckLoad(DeckId(deck), buffer))?;
    state.send(Command::DeckSetBpm(DeckId(deck), track.bpm.unwrap_or(0.0)))?;
    state.send(Command::DeckSetFirstBeat(
        DeckId(deck),
        track.first_beat.unwrap_or(0),
    ))?;
    // Restore a saved loop, or clear any stale loop from the deck's prior track.
    match (track.loop_start, track.loop_beats) {
        (Some(start), Some(beats)) if beats > 0.0 => {
            state.send(Command::DeckSetLoop(DeckId(deck), start, beats))?;
        }
        _ => state.send(Command::DeckClearLoop(DeckId(deck)))?,
    }

    Ok(TrackDto::from(&track))
}

/// Compute RMS levels (0.0..=1.0, one per bucket) for a sample range — used by
/// the full-track minimap, which renders them mirrored across the zero line.
fn rms_for_range(samples: &[f32], channels: u16, start: u64, end: u64, pixels: u32) -> Vec<f32> {
    const MAX_BUCKET_SAMPLES: usize = 2048;
    let ch = channels.max(1) as usize;
    let total = samples.len() / ch;
    let start = (start as usize).min(total);
    let end = (end as usize).clamp(start, total);
    let span = end - start;
    let pixels = pixels.clamp(1, 8000) as usize;

    let mut out = Vec::with_capacity(pixels);
    if span == 0 {
        return out;
    }

    let frame_mono = |f: usize| -> f32 {
        let base = f * ch;
        let mut sum = 0.0f32;
        for c in 0..ch {
            sum += samples[base + c];
        }
        sum / ch as f32
    };

    for p in 0..pixels {
        let b0 = start + (p * span) / pixels;
        let mut b1 = start + ((p + 1) * span) / pixels;
        if b1 <= b0 {
            b1 = (b0 + 1).min(end);
        }
        let step = ((b1 - b0) / MAX_BUCKET_SAMPLES).max(1);

        let mut sum_sq = 0.0f64;
        let mut count = 0u32;
        let mut f = b0;
        while f < b1 {
            let v = frame_mono(f);
            sum_sq += (v as f64) * (v as f64);
            count += 1;
            f += step;
        }
        let rms = if count > 0 {
            (sum_sq / count as f64).sqrt() as f32
        } else {
            0.0
        };
        out.push(rms.clamp(0.0, 1.0));
    }
    out
}

/// Waveform min/max peaks for a sample range on a deck (drives zoomable,
/// sample-accurate rendering). Returns interleaved [min, max, …] pairs.
#[tauri::command]
fn get_waveform(
    deck: u8,
    start: u64,
    end: u64,
    pixels: u32,
    state: State<AppState>,
) -> Result<Vec<f32>, String> {
    let waveforms = state.waveforms.lock().map_err(|e| e.to_string())?;
    match waveforms.get(deck as usize).and_then(|b| b.as_ref()) {
        Some(buffer) => Ok(peaks_for_range(
            &buffer.samples,
            buffer.channels,
            start,
            end,
            pixels,
        )),
        None => Ok(Vec::new()),
    }
}

/// Waveform RMS levels for a sample range on a deck (drives the minimap).
#[tauri::command]
fn get_waveform_rms(
    deck: u8,
    start: u64,
    end: u64,
    pixels: u32,
    state: State<AppState>,
) -> Result<Vec<f32>, String> {
    let waveforms = state.waveforms.lock().map_err(|e| e.to_string())?;
    match waveforms.get(deck as usize).and_then(|b| b.as_ref()) {
        Some(buffer) => Ok(rms_for_range(
            &buffer.samples,
            buffer.channels,
            start,
            end,
            pixels,
        )),
        None => Ok(Vec::new()),
    }
}

/// Full-text-ish search over the library.
#[tauri::command]
fn search_library(query: String, state: State<AppState>) -> Result<Vec<TrackDto>, String> {
    let db = state.database.lock().map_err(|e| e.to_string())?;
    let tracks = db.search(&query).map_err(|e| e.to_string())?;
    Ok(tracks.iter().map(TrackDto::from).collect())
}

// ---------------------------------------------------------------------------
// Live state
// ---------------------------------------------------------------------------

#[tauri::command]
fn get_engine_state(state: State<AppState>) -> Result<EngineSnapshot, String> {
    let s = state.engine_state.lock().map_err(|e| e.to_string())?;
    let decks = (0..4)
        .map(|i| DeckSnapshot {
            position: s.decks[i].position,
            duration: s.decks[i].duration,
            is_playing: s.decks[i].is_playing,
            pitch: s.decks[i].pitch,
            peak_left: s.mixer.channel_peaks[i][0],
            peak_right: s.mixer.channel_peaks[i][1],
            loop_active: s.decks[i].loop_active,
            loop_start: s.decks[i].loop_start,
            loop_end: s.decks[i].loop_end,
        })
        .collect();

    Ok(EngineSnapshot {
        position: s.position,
        tempo: s.tempo,
        is_playing: s.is_playing,
        beat_in_bar: s.beat_in_bar,
        beat_phase: s.beat_phase,
        crossfader_xy: s.mixer.crossfader_xy,
        master_left: s.mixer.master_peaks[0],
        master_right: s.mixer.master_peaks[1],
        decks,
    })
}

// ---------------------------------------------------------------------------
// Deck / mixer / transport controls
// ---------------------------------------------------------------------------

#[tauri::command]
fn play_deck(deck: u8, state: State<AppState>) -> Result<(), String> {
    state.send(Command::DeckPlay(DeckId(deck)))
}

#[tauri::command]
fn pause_deck(deck: u8, state: State<AppState>) -> Result<(), String> {
    state.send(Command::DeckPause(DeckId(deck)))
}

#[tauri::command]
fn stop_deck(deck: u8, state: State<AppState>) -> Result<(), String> {
    state.send(Command::DeckStop(DeckId(deck)))
}

#[tauri::command]
fn seek_deck(deck: u8, position: u64, state: State<AppState>) -> Result<(), String> {
    state.send(Command::DeckSeek(DeckId(deck), position))
}

#[tauri::command]
fn set_deck_volume(deck: u8, volume: f32, state: State<AppState>) -> Result<(), String> {
    state.send(Command::DeckSetVolume(DeckId(deck), volume))
}

#[tauri::command]
fn set_deck_pitch(deck: u8, pitch: f64, state: State<AppState>) -> Result<(), String> {
    state.send(Command::DeckSetPitch(DeckId(deck), pitch))
}

#[tauri::command]
fn set_track_bpm(
    deck: u8,
    track_id: u64,
    bpm: f64,
    state: State<AppState>,
) -> Result<(), String> {
    {
        let db = state.database.lock().map_err(|e| e.to_string())?;
        db.set_bpm(TrackId(track_id), bpm).map_err(|e| e.to_string())?;
    }
    state.send(Command::DeckSetBpm(DeckId(deck), bpm))
}

#[tauri::command]
fn set_first_beat(
    deck: u8,
    track_id: u64,
    first_beat: i64,
    state: State<AppState>,
) -> Result<(), String> {
    {
        let db = state.database.lock().map_err(|e| e.to_string())?;
        db.set_first_beat(TrackId(track_id), first_beat)
            .map_err(|e| e.to_string())?;
    }
    state.send(Command::DeckSetFirstBeat(DeckId(deck), first_beat))
}

#[tauri::command]
fn set_deck_loop(
    deck: u8,
    track_id: u64,
    loop_start: i64,
    loop_beats: f64,
    state: State<AppState>,
) -> Result<(), String> {
    {
        let db = state.database.lock().map_err(|e| e.to_string())?;
        db.set_loop(TrackId(track_id), loop_start, loop_beats)
            .map_err(|e| e.to_string())?;
    }
    state.send(Command::DeckSetLoop(DeckId(deck), loop_start, loop_beats))
}

#[tauri::command]
fn clear_deck_loop(deck: u8, track_id: u64, state: State<AppState>) -> Result<(), String> {
    {
        let db = state.database.lock().map_err(|e| e.to_string())?;
        db.clear_loop(TrackId(track_id)).map_err(|e| e.to_string())?;
    }
    state.send(Command::DeckClearLoop(DeckId(deck)))
}

/// Jump a deck to a cue point (source samples). The engine quantizes to the bar
/// when the deck is playing, or jumps immediately when stopped.
#[tauri::command]
fn deck_cue(deck: u8, position: i64, state: State<AppState>) -> Result<(), String> {
    state.send(Command::DeckCue(DeckId(deck), position))
}

/// Persist a track's cue points (hot cues 1-8; values beyond 8 ignored).
#[tauri::command]
fn set_cues(track_id: u64, cues: Vec<Option<i64>>, state: State<AppState>) -> Result<(), String> {
    let mut arr = [None; 8];
    for (slot, value) in arr.iter_mut().zip(cues.into_iter()) {
        *slot = value;
    }
    let db = state.database.lock().map_err(|e| e.to_string())?;
    db.set_cues(TrackId(track_id), &arr).map_err(|e| e.to_string())
}

#[tauri::command]
fn set_deck_sync(deck: u8, mode: String, state: State<AppState>) -> Result<(), String> {
    let mode = match mode.as_str() {
        "tempo" => SyncMode::Tempo,
        "phase" => SyncMode::Phase,
        _ => SyncMode::Off,
    };
    state.send(Command::DeckSetSync(DeckId(deck), mode))
}

#[tauri::command]
fn set_channel_eq(
    deck: u8,
    low: f32,
    mid: f32,
    high: f32,
    state: State<AppState>,
) -> Result<(), String> {
    state.send(Command::SetChannelEQ(deck as usize, low, mid, high))
}

#[tauri::command]
fn set_crossfader(x: f32, y: f32, state: State<AppState>) -> Result<(), String> {
    state.send(Command::SetCrossfader(x, y))
}

#[tauri::command]
fn set_channel_filter(deck: u8, value: f32, state: State<AppState>) -> Result<(), String> {
    state.send(Command::SetChannelFilter(deck as usize, value))
}

#[tauri::command]
fn set_master_gain(gain: f32, state: State<AppState>) -> Result<(), String> {
    state.send(Command::SetMasterGain(gain))
}

#[tauri::command]
fn set_tempo(bpm: f64, state: State<AppState>) -> Result<(), String> {
    state.send(Command::SetTempo(bpm))
}

#[tauri::command]
fn set_metronome(enabled: bool, state: State<AppState>) -> Result<(), String> {
    state.send(Command::SetMetronome(enabled))
}

#[tauri::command]
fn set_quantize(mode: String, state: State<AppState>) -> Result<(), String> {
    let q = match mode.as_str() {
        "beat" => Quantize::Beat,
        "bar" => Quantize::Bar,
        _ => Quantize::Off,
    };
    state.send(Command::SetQuantize(q))
}

fn main() {
    // Channel from the UI thread to the audio thread.
    let (command_sender, command_receiver) = command_channel(256);

    // Shared snapshot the audio thread publishes into.
    let engine_state = Arc::new(Mutex::new(EngineState::default()));

    // Build and start the audio engine. `start_audio` moves the engine into the
    // audio callback, so all later interaction goes through the command channel
    // and the shared state snapshot.
    let mut engine = AudioEngine::new(44100);
    engine.set_command_receiver(command_receiver);
    engine.set_state_output(engine_state.clone());

    // The global transport is the master clock: it free-runs from launch so the
    // metronome and beat display are always live and decks can quantize to it.
    engine.transport().play();

    let _audio_stream = match start_audio(engine) {
        Ok((stream, _transport)) => Some(stream),
        Err(e) => {
            eprintln!("Failed to start audio: {}", e);
            None
        }
    };

    let command_sender = Mutex::new(command_sender);

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(move |app| {
            // Open (or create) the library database in the app data directory.
            // If the file can't be opened we fall back to in-memory, but say so
            // loudly: in that mode BPM/grid edits do NOT persist across restarts.
            let data_dir = app
                .path()
                .app_data_dir()
                .unwrap_or_else(|_| std::env::temp_dir());
            std::fs::create_dir_all(&data_dir).ok();

            let db_path = data_dir.join("library.db");
            let database = match TrackDatabase::open(&db_path) {
                Ok(db) => {
                    println!("Library database: {}", db_path.display());
                    db
                }
                Err(e) => {
                    eprintln!(
                        "WARNING: could not open library database at {} ({e}); \
                         falling back to in-memory — BPM/grid edits will NOT persist.",
                        db_path.display()
                    );
                    TrackDatabase::open_in_memory().expect("failed to open in-memory database")
                }
            };

            app.manage(AppState {
                command_sender,
                engine_state,
                database: Mutex::new(database),
                waveforms: Mutex::new([None, None, None, None]),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            open_track_dialog,
            load_track,
            get_waveform,
            get_waveform_rms,
            search_library,
            get_engine_state,
            play_deck,
            pause_deck,
            stop_deck,
            seek_deck,
            set_deck_volume,
            set_deck_pitch,
            set_deck_sync,
            set_deck_loop,
            clear_deck_loop,
            deck_cue,
            set_cues,
            set_track_bpm,
            set_first_beat,
            set_channel_eq,
            set_channel_filter,
            set_crossfader,
            set_master_gain,
            set_tempo,
            set_metronome,
            set_quantize,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
