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
  // Link state lives on the engine snapshot — toggles dispatch and trust the
  // next poll for confirmation instead of holding a separate optimistic copy.
  const linkEnabled = snapshot?.link_enabled ?? false;
  const linkAudioEnabled = snapshot?.link_audio_enabled ?? false;
  const linkPeers = snapshot?.link_peers ?? 0;
  const linkSendBypass = snapshot?.link_send_bypass_eq_filter ?? false;

  // Mirror the engine's authoritative BPM into the input while Link is engaged
  // — peers can change the tempo and we want the slider to follow.
  useEffect(() => {
    if (linkEnabled && snapshot?.tempo) {
      setTempo(snapshot.tempo);
    }
  }, [linkEnabled, snapshot?.tempo]);

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

  const onLink = () => {
    api.setLinkEnabled(!linkEnabled);
  };

  const onLinkAudio = () => {
    // LinkAudio only makes sense with the basic Link session running; enable
    // both in one click so the user doesn't have to think about the order.
    if (!linkEnabled) {
      api.setLinkEnabled(true);
    }
    api.setLinkAudioEnabled(!linkAudioEnabled);
  };

  const onLinkSendBypass = () => {
    api.setLinkSendBypassEqFilter(!linkSendBypass);
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

      <button
        className={`link-toggle ${linkEnabled ? 'on' : ''}`}
        onClick={onLink}
        title="Ableton Link — share tempo and beat phase with peers on the LAN"
      >
        LINK{linkEnabled && linkPeers > 0 ? ` · ${linkPeers}` : ''}
      </button>

      <button
        className={`link-audio-toggle ${linkAudioEnabled ? 'on' : ''}`}
        onClick={onLinkAudio}
        title="Link Audio — broadcast each deck as its own channel on the Link network"
      >
        LINK AUDIO
      </button>

      <button
        className={`link-bypass-toggle ${linkSendBypass ? 'on' : ''}`}
        onClick={onLinkSendBypass}
        title={
          linkSendBypass
            ? 'Bypass on — broadcasting raw deck audio (pre EQ/filter)'
            : 'Bypass off — broadcasting booth feed (post EQ/filter)'
        }
        disabled={!linkAudioEnabled}
      >
        EQ/FLT BYPASS
      </button>
    </div>
  );
}
