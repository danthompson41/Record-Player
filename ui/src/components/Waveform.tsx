import { useEffect, useRef } from 'react';
import { CUE_COLORS } from '../cueColors';

interface WaveformProps {
  /** Interleaved [min, max, …] peaks covering [peaksStart, peaksEnd). */
  peaks: number[];
  peaksStart: number;
  peaksEnd: number;
  /** Playback position in source samples. */
  position: number;
  /** Visible window in source samples (centered on the playhead by the Deck). */
  viewStart: number;
  viewEnd: number;
  color: string;
  /** Beat grid inputs (source samples / BPM). Grid drawn when bpm > 0. */
  bpm?: number;
  sampleRate?: number;
  firstBeat?: number;
  /** Active loop region in source samples (shaded when loopActive). */
  loopActive?: boolean;
  loopStart?: number;
  loopEnd?: number;
  /** Cue points (hot cues 1-8) in source samples; null = unset. */
  cues?: (number | null)[];
  onSeek: (sample: number) => void;
  onZoom: (factor: number) => void;
}

export function Waveform({
  peaks,
  peaksStart,
  peaksEnd,
  position,
  viewStart,
  viewEnd,
  color,
  bpm = 0,
  sampleRate = 0,
  firstBeat = 0,
  loopActive = false,
  loopStart = 0,
  loopEnd = 0,
  cues,
  onSeek,
  onZoom,
}: WaveformProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null);

  // Keep the latest onZoom in a ref so the (once-bound) wheel listener can use it.
  const zoomRef = useRef(onZoom);
  zoomRef.current = onZoom;

  // Wheel-to-zoom. Bound manually so we can preventDefault (React onWheel is
  // passive). The view re-centers on the playhead, so there's no cursor anchor.
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const handler = (e: WheelEvent) => {
      e.preventDefault();
      zoomRef.current(e.deltaY < 0 ? 0.8 : 1.25);
    };
    canvas.addEventListener('wheel', handler, { passive: false });
    return () => canvas.removeEventListener('wheel', handler);
  }, []);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const ctx = canvas.getContext('2d');
    if (!ctx) return;

    const dpr = window.devicePixelRatio || 1;
    const width = canvas.clientWidth;
    const height = canvas.clientHeight;
    canvas.width = width * dpr;
    canvas.height = height * dpr;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);

    ctx.clearRect(0, 0, width, height);
    const mid = height / 2;
    const half = height / 2 - 2;
    const viewLen = viewEnd - viewStart;
    const buckets = peaks ? Math.floor(peaks.length / 2) : 0;

    if (buckets === 0 || viewLen <= 0) {
      ctx.fillStyle = '#555';
      ctx.font = '13px sans-serif';
      ctx.textAlign = 'center';
      ctx.textBaseline = 'middle';
      ctx.fillText('Drop a track or click Load', width / 2, mid);
      return;
    }

    const sampleToX = (s: number) => ((s - viewStart) / viewLen) * width;

    // Cue sections: each set cue colors the waveform from its position until the
    // next cue. Build the set cues sorted by position with their global colors.
    const sections = (cues ?? [])
      .map((c, n) => (c == null ? null : { pos: c, color: CUE_COLORS[n] ?? '#888' }))
      .filter((s): s is { pos: number; color: string } => s != null)
      .sort((a, b) => a.pos - b.pos);

    // Faint full-height band per section, so even quiet sections show their hue.
    if (sections.length > 0) {
      ctx.globalAlpha = 0.16;
      for (let i = 0; i < sections.length; i++) {
        const x0 = sampleToX(sections[i].pos);
        const x1 = i + 1 < sections.length ? sampleToX(sections[i + 1].pos) : width;
        ctx.fillStyle = sections[i].color;
        ctx.fillRect(x0, 0, x1 - x0, height);
      }
      ctx.globalAlpha = 1;
    }

    // Filled bipolar envelope: each column is filled from the max down to the
    // zero line and from the min up to it. Within a cue section the bars take
    // the cue's color; before the first cue they keep the played/unplayed look.
    const peaksLen = peaksEnd - peaksStart;
    let si = 0;
    let sectionColor: string | null = null;
    for (let x = 0; x < width; x++) {
      const sample = viewStart + (x / width) * viewLen;
      while (si < sections.length && sections[si].pos <= sample) {
        sectionColor = sections[si].color;
        si++;
      }
      // Map the sample into the (wider) peaks buffer, so the waveform stays
      // aligned to the playhead while scrolling without refetching per frame.
      const t = peaksLen > 0 ? (sample - peaksStart) / peaksLen : 0;
      const i = Math.min(buckets - 1, Math.max(0, Math.floor(t * buckets)));
      const min = peaks[i * 2];
      const max = peaks[i * 2 + 1];
      const yTop = mid - Math.max(0, max) * half; // above center (or center)
      const yBottom = mid - Math.min(0, min) * half; // below center (or center)
      ctx.fillStyle = sectionColor ?? (sample <= position ? color : '#3a3a52');
      ctx.fillRect(x, yTop, 1, Math.max(1, yBottom - yTop));
    }

    // Active loop region: shade + boundary lines.
    if (loopActive && loopEnd > loopStart) {
      const lx0 = sampleToX(loopStart);
      const lx1 = sampleToX(loopEnd);
      ctx.fillStyle = 'rgba(233, 217, 78, 0.16)';
      ctx.fillRect(lx0, 0, lx1 - lx0, height);
      ctx.strokeStyle = 'rgba(233, 217, 78, 0.85)';
      ctx.lineWidth = 1;
      ctx.beginPath();
      ctx.moveTo(Math.round(lx0) + 0.5, 0);
      ctx.lineTo(Math.round(lx0) + 0.5, height);
      ctx.moveTo(Math.round(lx1) + 0.5, 0);
      ctx.lineTo(Math.round(lx1) + 0.5, height);
      ctx.stroke();
    }

    // Beat grid (bars — every 4th beat — drawn brighter, and numbered).
    if (bpm > 0 && sampleRate > 0) {
      const samplesPerBeat = (sampleRate * 60) / bpm;
      // Only label bars when they're far enough apart to stay readable.
      const barPx = ((samplesPerBeat * 4) / viewLen) * width;
      const showBarNumbers = barPx >= 28;
      ctx.font = '9px sans-serif';
      ctx.textAlign = 'left';
      ctx.textBaseline = 'top';

      // Start at the first beat at or before the visible window.
      let beat = Math.max(0, Math.floor((viewStart - firstBeat) / samplesPerBeat));
      for (
        let pos = firstBeat + beat * samplesPerBeat;
        pos <= viewEnd;
        pos += samplesPerBeat, beat++
      ) {
        if (pos < viewStart) continue;
        const isBar = beat % 4 === 0;
        const x = Math.round(sampleToX(pos)) + 0.5;
        ctx.strokeStyle = isBar ? 'rgba(255,255,255,0.5)' : 'rgba(255,255,255,0.15)';
        ctx.lineWidth = 1;
        ctx.beginPath();
        ctx.moveTo(x, 0);
        ctx.lineTo(x, height);
        ctx.stroke();

        // Bar number (1-indexed) above the measure line.
        if (isBar && showBarNumbers) {
          ctx.fillStyle = 'rgba(255,255,255,0.55)';
          ctx.fillText(String(beat / 4 + 1), x + 3, 2);
        }
      }
    }

    // Cue markers: vertical line + number (bottom-left, distinct from bar
    // numbers at the top).
    if (cues) {
      ctx.font = '9px sans-serif';
      ctx.textAlign = 'left';
      ctx.textBaseline = 'bottom';
      cues.forEach((c, n) => {
        if (c == null || c < viewStart || c > viewEnd) return;
        const cueColor = CUE_COLORS[n] ?? '#fff';
        const x = Math.round(sampleToX(c)) + 0.5;
        ctx.strokeStyle = cueColor;
        ctx.lineWidth = 1;
        ctx.beginPath();
        ctx.moveTo(x, 0);
        ctx.lineTo(x, height);
        ctx.stroke();
        ctx.fillStyle = cueColor;
        ctx.fillText(String(n + 1), x + 2, height - 1);
      });
    }

    // Playhead, if within the window.
    if (position >= viewStart && position <= viewEnd) {
      const px = sampleToX(position);
      ctx.strokeStyle = '#fff';
      ctx.lineWidth = 1;
      ctx.beginPath();
      ctx.moveTo(px, 0);
      ctx.lineTo(px, height);
      ctx.stroke();
    }
  }, [
    peaks,
    peaksStart,
    peaksEnd,
    position,
    viewStart,
    viewEnd,
    color,
    bpm,
    sampleRate,
    firstBeat,
    loopActive,
    loopStart,
    loopEnd,
    cues,
  ]);

  // Click to seek (which re-centers the view on the new playhead position).
  const onClick = (e: React.MouseEvent<HTMLCanvasElement>) => {
    const rect = e.currentTarget.getBoundingClientRect();
    const sample = viewStart + ((e.clientX - rect.left) / rect.width) * (viewEnd - viewStart);
    onSeek(sample);
  };

  return <canvas ref={canvasRef} className="waveform" onClick={onClick} />;
}
