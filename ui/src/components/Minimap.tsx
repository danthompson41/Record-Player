import { useEffect, useRef } from 'react';
import { CUE_COLORS } from '../cueColors';

interface MinimapProps {
  /** RMS level (0..1) per bucket across the whole track. */
  rms: number[];
  durationSamples: number;
  /** Playback position in source samples. */
  position: number;
  color: string;
  cues: (number | null)[];
  /** "First bar" / beginning cue — colors the leading section. */
  beginPos: number;
  beginColor: string;
  /** The main waveform's visible window, drawn as a viewport box. */
  viewStart: number;
  viewEnd: number;
  /** Called with the clicked sample position. */
  onClickSample: (sample: number) => void;
}

// RMS runs lower than peak; boost so the minimap fills nicely.
const RMS_GAIN = 3.5;

export function Minimap({
  rms,
  durationSamples,
  position,
  color,
  cues,
  beginPos,
  beginColor,
  viewStart,
  viewEnd,
  onClickSample,
}: MinimapProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null);

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
    const half = height / 2 - 1;
    const n = rms.length;
    if (n === 0 || durationSamples <= 0) return;

    const sampleToX = (s: number) => (s / durationSamples) * width;

    // Cue sections coloring. The "first bar" cue is always present and colors
    // the leading section up to the first numbered cue.
    const sections = [
      { pos: beginPos, color: beginColor },
      ...cues
        .map((c, i) => (c == null ? null : { pos: c, color: CUE_COLORS[i] ?? '#888' }))
        .filter((s): s is { pos: number; color: string } => s != null),
    ].sort((a, b) => a.pos - b.pos);

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

    // RMS waveform, mirrored across the zero line (filled), colored by section.
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
      ctx.fillStyle = sectionColor ?? (sample <= position ? color : '#3a3a52');
      ctx.fillRect(x, mid - amp, 1, Math.max(1, amp * 2));
    }

    // Cue markers (line + small number) in each cue's color.
    ctx.font = '8px sans-serif';
    ctx.textAlign = 'left';
    ctx.textBaseline = 'top';
    cues.forEach((c, i) => {
      if (c == null) return;
      const cueColor = CUE_COLORS[i] ?? '#fff';
      const x = Math.round(sampleToX(c)) + 0.5;
      ctx.strokeStyle = cueColor;
      ctx.lineWidth = 1;
      ctx.beginPath();
      ctx.moveTo(x, 0);
      ctx.lineTo(x, height);
      ctx.stroke();
      ctx.fillStyle = cueColor;
      ctx.fillText(String(i + 1), x + 2, 1);
    });

    // Beginning ("first bar") marker.
    {
      const x = Math.round(sampleToX(beginPos)) + 0.5;
      ctx.strokeStyle = beginColor;
      ctx.lineWidth = 1;
      ctx.beginPath();
      ctx.moveTo(x, 0);
      ctx.lineTo(x, height);
      ctx.stroke();
    }

    // Viewport box: where the main (zoomed) waveform is looking.
    if (viewEnd > viewStart && (viewStart > 0 || viewEnd < durationSamples)) {
      const vx0 = sampleToX(viewStart);
      const vx1 = sampleToX(viewEnd);
      ctx.fillStyle = 'rgba(255,255,255,0.08)';
      ctx.fillRect(vx0, 0, vx1 - vx0, height);
      ctx.strokeStyle = 'rgba(255,255,255,0.6)';
      ctx.lineWidth = 1;
      ctx.strokeRect(vx0 + 0.5, 0.5, Math.max(2, vx1 - vx0), height - 1);
    }

    // Playhead.
    if (position >= 0 && position <= durationSamples) {
      const px = sampleToX(position);
      ctx.strokeStyle = '#fff';
      ctx.lineWidth = 1;
      ctx.beginPath();
      ctx.moveTo(px, 0);
      ctx.lineTo(px, height);
      ctx.stroke();
    }
  }, [rms, durationSamples, position, color, cues, beginPos, beginColor, viewStart, viewEnd]);

  const onClick = (e: React.MouseEvent<HTMLCanvasElement>) => {
    if (durationSamples <= 0) return;
    const rect = e.currentTarget.getBoundingClientRect();
    onClickSample(((e.clientX - rect.left) / rect.width) * durationSamples);
  };

  return <canvas ref={canvasRef} className="minimap" onClick={onClick} />;
}
