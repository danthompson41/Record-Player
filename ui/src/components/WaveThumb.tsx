import { useEffect, useRef } from 'react';
import { BEGIN_COLOR, CUE_COLORS } from '../cueColors';

interface WaveThumbProps {
  /** RMS level (0..1) per bucket across the whole track. */
  rms: number[];
  durationSamples: number;
  /** Cue points (samples) and beginning, used to color sections like the minimap. */
  cues: (number | null)[];
  beginPos: number;
  /** Default color for the region before the first marker. */
  color: string;
}

const RMS_GAIN = 3.5;

/** A compact, static RMS waveform thumbnail (mirrored across zero), colored by
 * cue sections — matching the deck minimap. */
export function WaveThumb({ rms, durationSamples, cues, beginPos, color }: WaveThumbProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const ctx = canvas.getContext('2d');
    if (!ctx) return;

    const dpr = window.devicePixelRatio || 1;
    const width = canvas.clientWidth;
    const height = canvas.clientHeight;
    if (width === 0) return;
    canvas.width = width * dpr;
    canvas.height = height * dpr;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, width, height);

    const n = rms.length;
    if (n === 0 || durationSamples <= 0) return;
    const mid = height / 2;
    const half = height / 2 - 1;
    const sampleToX = (s: number) => (s / durationSamples) * width;

    // Cue sections (beginning + numbered cues), same model as the minimap.
    const sections = [
      { pos: beginPos, color: BEGIN_COLOR },
      ...cues
        .map((c, i) => (c == null ? null : { pos: c, color: CUE_COLORS[i] ?? '#888' }))
        .filter((s): s is { pos: number; color: string } => s != null),
    ].sort((a, b) => a.pos - b.pos);

    // Faint section bands.
    ctx.globalAlpha = 0.16;
    for (let i = 0; i < sections.length; i++) {
      const x0 = sampleToX(sections[i].pos);
      const x1 = i + 1 < sections.length ? sampleToX(sections[i + 1].pos) : width;
      ctx.fillStyle = sections[i].color;
      ctx.fillRect(x0, 0, x1 - x0, height);
    }
    ctx.globalAlpha = 1;

    // RMS bars, mirrored across zero, colored by section.
    let si = 0;
    let sectionColor: string | null = null;
    for (let x = 0; x < width; x++) {
      const sample = (x / width) * durationSamples;
      while (si < sections.length && sections[si].pos <= sample) {
        sectionColor = sections[si].color;
        si++;
      }
      const i = Math.min(n - 1, Math.floor((x / width) * n));
      const amp = Math.min(1, rms[i] * RMS_GAIN) * half;
      ctx.fillStyle = sectionColor ?? color;
      ctx.fillRect(x, mid - amp, 1, Math.max(1, amp * 2));
    }
  }, [rms, durationSamples, cues, beginPos, color]);

  return <canvas ref={canvasRef} className="wave-thumb" />;
}
