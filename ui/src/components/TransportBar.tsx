import { useEffect, useState } from 'react';
import { api, EngineSnapshot, Quantize } from '../api';

interface TransportBarProps {
  snapshot: EngineSnapshot | null;
}

const QUANTIZE_OPTIONS: Quantize[] = ['off', 'beat', 'bar'];

export function TransportBar({ snapshot }: TransportBarProps) {
  const [tempo, setTempo] = useState(120);
  const [metronome, setMetronome] = useState(false);
  const [quantize, setQuantize] = useState<Quantize>('bar');

  const beatInBar = snapshot?.beat_in_bar ?? 0;
  const beatPhase = snapshot?.beat_phase ?? 0;

  // Sync the UI's initial quantize value down to the engine on mount.
  useEffect(() => {
    api.setQuantize('bar');
  }, []);

  const onTempo = (v: number) => {
    setTempo(v);
    api.setTempo(v);
  };

  const onMetronome = () => {
    const next = !metronome;
    setMetronome(next);
    api.setMetronome(next);
  };

  const onQuantize = (q: Quantize) => {
    setQuantize(q);
    api.setQuantize(q);
  };

  return (
    <div className="transport-bar">
      {/* Four quarter-note dots; the active one pulses on the beat. */}
      <div className="beat-dots">
        {[0, 1, 2, 3].map((i) => {
          const active = i === beatInBar;
          // Pop on the beat, relax across it.
          const scale = active ? 1 + (1 - beatPhase) * 0.6 : 1;
          return (
            <div
              key={i}
              className={`beat-dot ${active ? 'active' : ''} ${i === 0 ? 'downbeat' : ''}`}
              style={{ transform: `scale(${scale})` }}
            />
          );
        })}
      </div>

      <div className="transport-tempo">
        <span className="bpm-value">{tempo.toFixed(0)}</span>
        <span className="bpm-label">BPM</span>
        <input
          type="range"
          min={60}
          max={200}
          value={tempo}
          onChange={(e) => onTempo(Number(e.target.value))}
        />
      </div>

      <div className="transport-quantize">
        <span className="control-label">Quantize</span>
        <div className="segmented">
          {QUANTIZE_OPTIONS.map((q) => (
            <button
              key={q}
              className={quantize === q ? 'active' : ''}
              onClick={() => onQuantize(q)}
            >
              {q}
            </button>
          ))}
        </div>
      </div>

      <button
        className={`metronome-toggle ${metronome ? 'on' : ''}`}
        onClick={onMetronome}
        title="Toggle audible metronome"
      >
        {metronome ? '🔊' : '🔇'} Metronome
      </button>
    </div>
  );
}
