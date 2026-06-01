import { useEffect, useRef, useState } from 'react';
import { api, DeckSnapshot, formatTime, Track } from '../api';
import { Waveform } from './Waveform';
import { Minimap } from './Minimap';
import { BEGIN_COLOR, CUE_COLORS, textOn } from '../cueColors';

// Loop lengths as bar fractions → beats (1 bar = 4 beats).
const LOOP_OPTIONS: { label: string; beats: number }[] = [
  { label: '4', beats: 16 },
  { label: '2', beats: 8 },
  { label: '1', beats: 4 },
  { label: '½', beats: 2 },
  { label: '¼', beats: 1 },
  { label: '⅛', beats: 0.5 },
  { label: '1/16', beats: 0.25 },
  { label: '1/32', beats: 0.125 },
  { label: '1/64', beats: 0.0625 },
];

interface DeckProps {
  deckId: number;
  label: string;
  color: string;
  track: Track | null;
  snapshot: DeckSnapshot | undefined;
  globalBpm: number;
  synced: boolean;
  onLoad: () => void;
}

export function Deck({
  deckId,
  label,
  color,
  track,
  snapshot,
  globalBpm,
  synced,
  onLoad,
}: DeckProps) {
  const [bpm, setBpm] = useState<string>('');
  const [firstBeat, setFirstBeat] = useState(0);
  // Zoom = visible window width in samples (0 → whole track). The window itself
  // is always re-centered on the playhead below, so the waveform scrolls.
  const [viewWidth, setViewWidth] = useState(0);
  const [activeLoop, setActiveLoop] = useState<number | null>(null);
  const [loopStart, setLoopStart] = useState<number | null>(null);
  // Peaks for a buffer a bit wider than the view, so scrolling/zoom stay smooth
  // without refetching every frame. `start`/`end` are the samples it covers.
  const [buffer, setBuffer] = useState<{ peaks: number[]; start: number; end: number; width: number }>({
    peaks: [],
    start: 0,
    end: 0,
    width: 0,
  });
  const [minimapRms, setMinimapRms] = useState<number[]>([]);
  const [cues, setCues] = useState<(number | null)[]>(() => Array(8).fill(null));

  // Reset per-track state whenever a different track is loaded.
  useEffect(() => {
    setBpm(track?.bpm != null ? String(track.bpm) : '');
    setFirstBeat(track?.first_beat ?? 0);
    setViewWidth(track?.duration_samples ?? 0);
    const hasLoop = !!(track?.loop_beats && track.loop_beats > 0);
    setActiveLoop(hasLoop ? track!.loop_beats : null);
    setLoopStart(hasLoop ? track!.loop_start : null);
    const restored = track?.cues ?? [];
    setCues(Array.from({ length: 8 }, (_, i) => restored[i] ?? null));
  }, [track?.id, track?.bpm, track?.first_beat, track?.duration_samples, track?.loop_beats]);

  const sampleRate = track?.sample_rate ?? 44100;
  const position = snapshot?.position ?? 0;
  const duration = snapshot?.duration ?? track?.duration_samples ?? 0;
  const bpmNum = Number(bpm) || 0;

  // Don't allow zooming tighter than ~0.25s of audio across the canvas.
  const minWidth = Math.min(Math.max(1024, Math.floor(sampleRate * 0.25)), duration || 1);
  const clampedWidth =
    duration > 0 ? Math.min(Math.max(viewWidth || duration, minWidth), duration) : 0;
  // The visible window is centered on the playhead, clamped to the track.
  const viewStart =
    duration > 0 ? Math.max(0, Math.min(position - clampedWidth / 2, duration - clampedWidth)) : 0;
  const viewEnd = viewStart + clampedWidth;
  const viewLen = clampedWidth;

  // factor < 1 zooms in (narrower window), > 1 zooms out.
  const onZoom = (factor: number) => {
    if (duration <= 0) return;
    setViewWidth(Math.min(duration, Math.max(minWidth, clampedWidth * factor)));
  };
  const zoomFit = () => setViewWidth(duration);

  // Read the latest buffer via a ref so it isn't an effect dependency (which
  // would loop: the effect calls setBuffer).
  const bufferRef = useRef(buffer);
  bufferRef.current = buffer;

  // Keep a peaks buffer (~3× the view) covering the visible window; refetch only
  // when zoom changes or the scrolling view approaches the buffer's edges.
  useEffect(() => {
    if (!track || duration <= 0 || clampedWidth <= 0) {
      if (bufferRef.current.peaks.length > 0) {
        setBuffer({ peaks: [], start: 0, end: 0, width: 0 });
      }
      return;
    }
    const buf = bufferRef.current;
    const margin = clampedWidth;
    const nearLeft = buf.start > 0 && viewStart - buf.start < margin * 0.4;
    const nearRight = buf.end < duration && buf.end - viewEnd < margin * 0.4;
    const needFetch =
      buf.peaks.length === 0 ||
      buf.width !== clampedWidth ||
      viewStart < buf.start ||
      viewEnd > buf.end ||
      nearLeft ||
      nearRight;
    if (!needFetch) return;

    const bs = Math.max(0, Math.floor(viewStart - margin));
    const be = Math.min(duration, Math.ceil(viewEnd + margin));
    let cancelled = false;
    api
      .getWaveform(deckId, bs, be, 3000)
      .then((p) => {
        if (!cancelled) setBuffer({ peaks: p, start: bs, end: be, width: clampedWidth });
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, [deckId, track?.id, viewStart, viewEnd, clampedWidth, duration]);

  // Full-track RMS for the minimap — fetched once per loaded track.
  useEffect(() => {
    const dur = track?.duration_samples ?? 0;
    if (!track || dur <= 0) {
      setMinimapRms([]);
      return;
    }
    let cancelled = false;
    api
      .getWaveformRms(deckId, 0, dur, 1000)
      .then((r) => {
        if (!cancelled) setMinimapRms(r);
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, [deckId, track?.id, track?.duration_samples]);

  // Snap the loop start to the nearest beat grid line (robust to the ~33ms
  // staleness of the polled playhead, since a beat is far longer than that).
  const computeLoopStart = () => {
    const spb = (sampleRate * 60) / bpmNum;
    const k = Math.round((position - firstBeat) / spb);
    let start = firstBeat + k * spb;
    while (start < 0) start += spb;
    return Math.round(start);
  };

  const onLoop = (beats: number) => {
    if (!track || bpmNum <= 0) return;
    if (activeLoop === beats) {
      api.clearDeckLoop(deckId, track.id);
      setActiveLoop(null);
      setLoopStart(null);
      return;
    }
    // Keep the existing start when changing length on an active loop; otherwise
    // anchor a new loop at the current playhead (snapped to the beat grid).
    const start = activeLoop != null && loopStart != null ? loopStart : computeLoopStart();
    api.setDeckLoop(deckId, track.id, start, beats);
    setActiveLoop(beats);
    setLoopStart(start);
  };

  // The bar grid line at or before the current playhead (where a cue is set).
  const barBeforePlayhead = () => {
    const bar = ((sampleRate * 60) / bpmNum) * 4;
    const k = Math.max(0, Math.floor((position - firstBeat) / bar));
    return Math.round(firstBeat + k * bar);
  };

  const persistCues = (next: (number | null)[]) => {
    setCues(next);
    if (track) api.setCues(track.id, next);
  };

  // Press: set the cue (to the bar before the playhead) if empty, else jump to it.
  const onCue = (n: number) => {
    if (!track || bpmNum <= 0) return;
    if (cues[n] == null) {
      const next = [...cues];
      next[n] = barBeforePlayhead();
      persistCues(next);
    } else {
      api.deckCue(deckId, cues[n] as number);
    }
  };

  const clearCue = (n: number) => {
    if (cues[n] == null) return;
    const next = [...cues];
    next[n] = null;
    persistCues(next);
  };

  const jumpToFirstBar = () => {
    if (!track) return;
    api.deckCue(deckId, Math.round(firstBeat));
  };

  // Minimap click: trigger whichever cue's colored section it lands in (the
  // "first bar" beginning counts as a section too) and play from there;
  // otherwise just seek to the clicked position.
  const onMinimapClick = (sample: number) => {
    if (!track) return;
    let bestPos = -Infinity;
    let target: number | null = null;
    // Beginning ("first bar") section.
    if (firstBeat <= sample) {
      bestPos = firstBeat;
      target = Math.round(firstBeat);
    }
    cues.forEach((c) => {
      if (c != null && c <= sample && c > bestPos) {
        bestPos = c;
        target = c;
      }
    });
    if (target != null) {
      api.deckCue(deckId, target);
      api.playDeck(deckId);
    } else {
      api.seekDeck(deckId, Math.max(0, Math.floor(sample)));
    }
  };

  // Pitch ratio is driven by the engine when synced; otherwise the manual
  // slider lives in the central Mixer (so no local readout here).
  const effectivePitch = synced ? snapshot?.pitch ?? 1 : 1;
  const pitchPercent = ((effectivePitch - 1) * 100).toFixed(1);

  const onBpm = (value: string) => {
    setBpm(value);
    const parsed = Number(value);
    if (track && Number.isFinite(parsed) && parsed > 0) {
      api.setTrackBpm(deckId, track.id, parsed);
    }
  };

  const onSeek = (sample: number) => {
    api.seekDeck(deckId, Math.max(0, Math.floor(sample)));
  };

  const nudgeGrid = (deltaMs: number) => {
    if (!track) return;
    // Offset may go negative (downbeat before the audio → pre-roll silence).
    const next = firstBeat + Math.round((sampleRate * deltaMs) / 1000);
    setFirstBeat(next);
    api.setFirstBeat(deckId, track.id, next);
  };

  // Reference-app style "1:23 / -2:34" elapsed / remaining display.
  const remaining = Math.max(0, duration - position);

  return (
    <div className="deck">
      {/* Compact top header: identity, transport, key readouts, and the
         zoom / load controls pushed to the right. Mirrors the layout in
         the reference UI where the deck strip is dominated by the waveform
         and everything else sits in a single line above it. */}
      <div className="deck-header">
        <span className="deck-title" style={{ color }}>
          {label}
        </span>

        <div className="deck-transport">
          <button
            className="transport-btn play"
            style={{ background: color, color: textOn(color) }}
            onClick={() => api.playDeck(deckId)}
            title="Play"
          >
            ▶
          </button>
          <button
            className="transport-btn"
            onClick={() => api.pauseDeck(deckId)}
            title="Pause"
          >
            ❙❙
          </button>
          <button
            className="transport-btn"
            onClick={() => api.stopDeck(deckId)}
            title="Stop"
          >
            ■
          </button>
        </div>

        <span className="deck-bpm">
          {bpmNum > 0 ? bpmNum.toFixed(1) : '—'} <em>BPM</em>
        </span>

        <span className="deck-time">
          {formatTime(position, sampleRate)} /{' '}
          <span className="deck-time-remaining">
            −{formatTime(remaining, sampleRate)}
          </span>
        </span>

        <div className="track-info">
          {track ? (
            <span className="track-name">{track.title ?? track.path}</span>
          ) : (
            <span className="track-name empty">No track loaded</span>
          )}
        </div>

        <div className="deck-header-right">
          <button
            className="nudge"
            disabled={!track}
            onClick={() => onZoom(2)}
            title="Zoom out"
          >
            −
          </button>
          <button className="zoom-fit" disabled={!track} onClick={zoomFit}>
            fit
          </button>
          <button
            className="nudge"
            disabled={!track}
            onClick={() => onZoom(0.5)}
            title="Zoom in"
          >
            +
          </button>
          <span className="zoom-readout">
            {viewLen > 0 ? (viewLen / sampleRate).toFixed(1) : '0.0'}s
          </span>
          <button className="btn outline" onClick={onLoad}>
            Load…
          </button>
        </div>
      </div>

      {/* BPM input + grid nudge — kept since they need persistent editable
         fields. Compressed onto one row instead of three. */}
      <div className="bpm-row deck-meta-row">
        <span>BPM</span>
        <input
          type="number"
          className="bpm-input"
          min={20}
          max={300}
          step={0.1}
          value={bpm}
          placeholder="—"
          disabled={!track}
          onChange={(e) => onBpm(e.target.value)}
        />
        <span className="bpm-target">→ {globalBpm.toFixed(0)} global</span>
        {synced && <span className="pitch-readout">{pitchPercent}%</span>}

        <span className="meta-sep" />

        <span>Grid</span>
        <button
          className="nudge"
          disabled={!track}
          onClick={() => nudgeGrid(-10)}
        >
          −
        </button>
        <span className="bpm-target">
          {((firstBeat / sampleRate) * 1000).toFixed(0)} ms
        </span>
        <button
          className="nudge"
          disabled={!track}
          onClick={() => nudgeGrid(10)}
        >
          +
        </button>
      </div>

      {/* Big main waveform — the visually dominant element. */}
      <Waveform
        peaks={buffer.peaks}
        peaksStart={buffer.start}
        peaksEnd={buffer.end}
        position={position}
        viewStart={viewStart}
        viewEnd={viewEnd}
        color={color}
        onSeek={onSeek}
        onZoom={onZoom}
        bpm={bpmNum}
        sampleRate={sampleRate}
        firstBeat={firstBeat}
        loopActive={snapshot?.loop_active ?? false}
        loopStart={snapshot?.loop_start ?? 0}
        loopEnd={snapshot?.loop_end ?? 0}
        cues={cues}
      />

      {/* Minimap sits directly under the waveform in the reference layout,
         carrying the cue-colored sections and the viewport box. */}
      <Minimap
        rms={minimapRms}
        durationSamples={duration}
        position={position}
        color={color}
        cues={cues}
        beginPos={firstBeat}
        beginColor={BEGIN_COLOR}
        viewStart={viewStart}
        viewEnd={viewEnd}
        onClickSample={onMinimapClick}
      />

      {/* Cue buttons — directly below the minimap, no row label. */}
      <div className="bpm-row cue-row">
        <button
          className="cue-btn first-bar"
          style={
            track
              ? {
                  background: BEGIN_COLOR,
                  borderColor: BEGIN_COLOR,
                  color: textOn(BEGIN_COLOR),
                }
              : undefined
          }
          disabled={!track}
          onClick={jumpToFirstBar}
          title="Jump to the first bar (start)"
        >
          ⇤
        </button>
        {cues.map((c, n) => (
          <button
            key={n}
            className={`cue-btn ${c != null ? 'set' : ''}`}
            style={
              c != null
                ? {
                    background: CUE_COLORS[n],
                    borderColor: CUE_COLORS[n],
                    color: textOn(CUE_COLORS[n]),
                  }
                : undefined
            }
            disabled={!track || bpmNum <= 0}
            onClick={() => onCue(n)}
            onContextMenu={(e) => {
              e.preventDefault();
              clearCue(n);
            }}
            title={
              c != null
                ? `Jump to cue ${n + 1} (right-click to clear)`
                : `Set cue ${n + 1}`
            }
          >
            {n + 1}
          </button>
        ))}
      </div>

      {/* Loop buttons at the bottom, no row label. */}
      <div className="bpm-row loop-row">
        {LOOP_OPTIONS.map((o) => (
          <button
            key={o.beats}
            className={`loop-btn ${activeLoop === o.beats ? 'active' : ''}`}
            disabled={!track || bpmNum <= 0}
            onClick={() => onLoop(o.beats)}
            title={`${o.label} bar loop`}
          >
            {o.label}
          </button>
        ))}
      </div>
    </div>
  );
}
