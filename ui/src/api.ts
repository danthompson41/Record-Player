import { invoke } from '@tauri-apps/api/core';

// ---------------------------------------------------------------------------
// Types mirroring the Rust DTOs in src-tauri/src/main.rs
// ---------------------------------------------------------------------------

export interface DeckSnapshot {
  position: number;
  duration: number;
  is_playing: boolean;
  pitch: number;
  peak_left: number;
  peak_right: number;
  loop_active: boolean;
  loop_start: number;
  loop_end: number;
}

export interface EngineSnapshot {
  position: number;
  tempo: number;
  is_playing: boolean;
  beat_in_bar: number;
  beat_phase: number;
  /** XY crossfader position (x, y), each ∈ [-1, 1]. */
  crossfader_xy: [number, number];
  master_left: number;
  master_right: number;
  decks: DeckSnapshot[];
}

export type Quantize = 'off' | 'beat' | 'bar';

export interface Track {
  id: number;
  path: string;
  title: string | null;
  artist: string | null;
  album: string | null;
  duration_samples: number;
  sample_rate: number;
  channels: number;
  bpm: number | null;
  first_beat: number;
  loop_start: number;
  loop_beats: number;
  cues: (number | null)[];
  /** Low-res RMS overview (0..1 per bucket) for the library thumbnail. */
  waveform: number[];
}

export type SyncMode = 'off' | 'tempo' | 'phase';

// ---------------------------------------------------------------------------
// Command wrappers
// ---------------------------------------------------------------------------

export const api = {
  openTrackDialog: () => invoke<string | null>('open_track_dialog'),
  loadTrack: (deck: number, path: string) =>
    invoke<Track>('load_track', { deck, path }),
  /** Interleaved [min, max, …] peaks for a sample range (zoomable waveform). */
  getWaveform: (deck: number, start: number, end: number, pixels: number) =>
    invoke<number[]>('get_waveform', { deck, start, end, pixels }),
  /** RMS level per bucket for a sample range (full-track minimap). */
  getWaveformRms: (deck: number, start: number, end: number, pixels: number) =>
    invoke<number[]>('get_waveform_rms', { deck, start, end, pixels }),
  searchLibrary: (query: string) => invoke<Track[]>('search_library', { query }),
  getEngineState: () => invoke<EngineSnapshot>('get_engine_state'),

  playDeck: (deck: number) => invoke('play_deck', { deck }),
  pauseDeck: (deck: number) => invoke('pause_deck', { deck }),
  stopDeck: (deck: number) => invoke('stop_deck', { deck }),
  seekDeck: (deck: number, position: number) =>
    invoke('seek_deck', { deck, position }),
  setDeckVolume: (deck: number, volume: number) =>
    invoke('set_deck_volume', { deck, volume }),
  setDeckPitch: (deck: number, pitch: number) =>
    invoke('set_deck_pitch', { deck, pitch }),
  setDeckSync: (deck: number, mode: SyncMode) =>
    invoke('set_deck_sync', { deck, mode }),
  setTrackBpm: (deck: number, trackId: number, bpm: number) =>
    invoke('set_track_bpm', { deck, trackId, bpm }),
  setFirstBeat: (deck: number, trackId: number, firstBeat: number) =>
    invoke('set_first_beat', { deck, trackId, firstBeat }),
  setDeckLoop: (deck: number, trackId: number, loopStart: number, loopBeats: number) =>
    invoke('set_deck_loop', { deck, trackId, loopStart, loopBeats }),
  clearDeckLoop: (deck: number, trackId: number) =>
    invoke('clear_deck_loop', { deck, trackId }),
  deckCue: (deck: number, position: number) =>
    invoke('deck_cue', { deck, position }),
  setCues: (trackId: number, cues: (number | null)[]) =>
    invoke('set_cues', { trackId, cues }),
  setChannelEq: (deck: number, low: number, mid: number, high: number) =>
    invoke('set_channel_eq', { deck, low, mid, high }),
  /** DJ "colour" filter slider: value ∈ [-1, 1]; 0 = bypass, − LP, + HP. */
  setChannelFilter: (deck: number, value: number) =>
    invoke('set_channel_filter', { deck, value }),

  setCrossfader: (x: number, y: number) => invoke('set_crossfader', { x, y }),
  setMasterGain: (gain: number) => invoke('set_master_gain', { gain }),
  setTempo: (bpm: number) => invoke('set_tempo', { bpm }),
  setMetronome: (enabled: boolean) => invoke('set_metronome', { enabled }),
  setQuantize: (mode: Quantize) => invoke('set_quantize', { mode }),
};

/** Format a sample count into mm:ss given a sample rate. */
export function formatTime(samples: number, sampleRate: number): string {
  if (!sampleRate) return '0:00';
  const totalSeconds = Math.floor(samples / sampleRate);
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = totalSeconds % 60;
  return `${minutes}:${seconds.toString().padStart(2, '0')}`;
}
