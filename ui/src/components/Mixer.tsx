import { useCallback, useEffect, useRef, useState } from 'react';
import { api, EngineSnapshot, SyncMode } from '../api';
import { ChannelState } from '../App';
import { Meter } from './Meter';

interface MixerProps {
  snapshot: EngineSnapshot | null;
  channels: ChannelState[];
  setChannel: (id: number, partial: Partial<ChannelState>) => void;
  deckColors: [string, string, string, string];
}

const SYNC_MODES: SyncMode[] = ['off', 'tempo', 'phase'];
const STRIP_LABELS = ['A', 'B', 'C', 'D'];

export function Mixer({ snapshot, channels, setChannel, deckColors }: MixerProps) {
  const [master, setMaster] = useState(100); // 0..120

  const onMaster = (v: number) => {
    setMaster(v);
    api.setMasterGain(v / 100);
  };

  return (
    <div className="mixer">
      <div className="mixer-strips">
        {STRIP_LABELS.map((label, i) => (
          <ChannelStrip
            key={i}
            id={i}
            label={label}
            color={deckColors[i]}
            state={channels[i]}
            setState={(p) => setChannel(i, p)}
            peakL={snapshot?.decks[i]?.peak_left ?? 0}
            peakR={snapshot?.decks[i]?.peak_right ?? 0}
            enginePitch={snapshot?.decks[i]?.pitch ?? 1}
          />
        ))}
      </div>

      <div className="master-row">
        <span className="master-label">Master</span>
        <input
          type="range"
          min={0}
          max={120}
          value={master}
          onChange={(e) => onMaster(Number(e.target.value))}
        />
        <span className="master-val">{master}</span>
        <div className="master-meters">
          <Meter level={snapshot?.master_left ?? 0} />
          <Meter level={snapshot?.master_right ?? 0} />
        </div>
      </div>

      <XYPad
        x={snapshot?.crossfader_xy?.[0] ?? 0}
        y={snapshot?.crossfader_xy?.[1] ?? 0}
        deckColors={deckColors}
      />
    </div>
  );
}

interface ChannelStripProps {
  id: number;
  label: string;
  color: string;
  state: ChannelState;
  setState: (partial: Partial<ChannelState>) => void;
  peakL: number;
  peakR: number;
  enginePitch: number;
}

function ChannelStrip({
  id,
  label,
  color,
  state,
  setState,
  peakL,
  peakR,
  enginePitch,
}: ChannelStripProps) {
  const synced = state.sync !== 'off';
  const displayedPitch = synced ? (enginePitch - 1) * 100 : state.pitch;

  const onVolume = (v: number) => {
    setState({ volume: v });
    api.setDeckVolume(id, v / 100);
  };

  const onPitch = (p: number) => {
    setState({ pitch: p });
    api.setDeckPitch(id, 1 + p / 100);
  };

  const onSync = (mode: SyncMode) => {
    setState({ sync: mode });
    api.setDeckSync(id, mode);
  };

  const onEq = (band: 'low' | 'mid' | 'high', value: number) => {
    const next = { ...state.eq, [band]: value };
    setState({ eq: next });
    api.setChannelEq(id, next.low, next.mid, next.high);
  };

  const onFilter = (v: number) => {
    setState({ filter: v });
    api.setChannelFilter(id, v / 100);
  };


  return (
    <div className="mixer-strip">
      <div className="strip-header" style={{ borderColor: color }}>
        <span className="strip-header-label" style={{ color }}>{label}</span>
      </div>

      <div className="strip-sync">
        {SYNC_MODES.map((mode) => (
          <button
            key={mode}
            className={`sync-btn ${state.sync === mode ? 'active' : ''}`}
            onClick={() => onSync(mode)}
            title={mode === 'off' ? 'Free pitch' : 'Match global BPM'}
          >
            {mode === 'off' ? 'off' : mode === 'tempo' ? 'sync' : 'phs'}
          </button>
        ))}
      </div>

      <div className="strip-knobs">
        <KnobRow
          label="PITCH"
          value={Math.round(displayedPitch)}
          min={-50}
          max={50}
          step={1}
          disabled={synced}
          onChange={onPitch}
          formatted={`${displayedPitch > 0 ? '+' : ''}${displayedPitch.toFixed(0)}%`}
        />
        {(['high', 'mid', 'low'] as const).map((band) => {
          const lbl = band === 'high' ? 'HI' : band === 'mid' ? 'MID' : 'LOW';
          return (
            <KnobRow
              key={band}
              label={lbl}
              value={state.eq[band]}
              min={0}
              max={2}
              step={0.01}
              onChange={(v) => onEq(band, v)}
            />
          );
        })}
        <KnobRow
          label="FLTR"
          value={state.filter}
          min={-100}
          max={100}
          step={1}
          onChange={onFilter}
          centered
        />
      </div>

      <div className="strip-fader">
        <input
          type="range"
          className="vol-fader"
          min={0}
          max={100}
          value={state.volume}
          onChange={(e) => onVolume(Number(e.target.value))}
        />
        <div className="strip-meters">
          <Meter level={peakL} />
          <Meter level={peakR} />
        </div>
      </div>
    </div>
  );
}

interface KnobRowProps {
  label: string;
  value: number;
  min: number;
  max: number;
  step: number;
  disabled?: boolean;
  onChange: (v: number) => void;
  /** Optional formatted readout to the right (else nothing is shown). */
  formatted?: string;
  /** When true, double-clicking the slider snaps it back to 0 (bipolar). */
  centered?: boolean;
}

/** Compact label + horizontal-slider row, used for pitch / EQ / filter. */
function KnobRow({
  label,
  value,
  min,
  max,
  step,
  disabled,
  onChange,
  formatted,
  centered,
}: KnobRowProps) {
  return (
    <div className={`knob-row${centered ? ' knob-row-centered' : ''}`}>
      <span className="knob-label">{label}</span>
      <input
        type="range"
        min={min}
        max={max}
        step={step}
        value={value}
        disabled={disabled}
        onChange={(e) => onChange(Number(e.target.value))}
        onDoubleClick={centered ? () => onChange(0) : undefined}
      />
      {formatted != null && <span className="knob-val">{formatted}</span>}
    </div>
  );
}

interface XYPadProps {
  /** Authoritative position from the engine snapshot (≈30 Hz). */
  x: number; // [-1, 1]
  y: number; // [-1, 1]
  deckColors: [string, string, string, string];
}

/**
 * 2-D crossfader pad. Decks A/B/C/D are pinned to the four corners (A=top-left,
 * B=top-right, C=bottom-left, D=bottom-right) per the engine's `xy_corner_gains`.
 * Local state is used during drag for instant feedback; the engine snapshot is
 * the source of truth between drags.
 */
function XYPad({ x: snapX, y: snapY, deckColors }: XYPadProps) {
  const padRef = useRef<HTMLDivElement>(null);
  const draggingRef = useRef(false);
  const [drag, setDrag] = useState<{ x: number; y: number } | null>(null);

  const positionFromEvent = useCallback((clientX: number, clientY: number) => {
    const el = padRef.current;
    if (!el) return null;
    const rect = el.getBoundingClientRect();
    const nx = ((clientX - rect.left) / rect.width) * 2 - 1;
    const ny = ((clientY - rect.top) / rect.height) * 2 - 1;
    return { x: clamp(nx, -1, 1), y: clamp(ny, -1, 1) };
  }, []);

  const apply = useCallback((p: { x: number; y: number }) => {
    setDrag(p);
    api.setCrossfader(p.x, p.y);
  }, []);

  useEffect(() => {
    const onMove = (e: PointerEvent) => {
      if (!draggingRef.current) return;
      const p = positionFromEvent(e.clientX, e.clientY);
      if (p) apply(p);
    };
    const onUp = () => {
      if (!draggingRef.current) return;
      draggingRef.current = false;
      setDrag(null); // release: defer to snapshot value again
    };
    window.addEventListener('pointermove', onMove);
    window.addEventListener('pointerup', onUp);
    return () => {
      window.removeEventListener('pointermove', onMove);
      window.removeEventListener('pointerup', onUp);
    };
  }, [apply, positionFromEvent]);

  const onPointerDown = (e: React.PointerEvent) => {
    draggingRef.current = true;
    const p = positionFromEvent(e.clientX, e.clientY);
    if (p) apply(p);
  };

  const x = drag?.x ?? snapX;
  const y = drag?.y ?? snapY;
  const knobLeft = `${((x + 1) / 2) * 100}%`;
  const knobTop = `${((y + 1) / 2) * 100}%`;

  return (
    <div className="xy-pad-wrap">
      <div className="xy-pad" ref={padRef} onPointerDown={onPointerDown}>
        <div className="xy-corner xy-tl" style={{ color: deckColors[0] }}>A</div>
        <div className="xy-corner xy-tr" style={{ color: deckColors[1] }}>B</div>
        <div className="xy-corner xy-bl" style={{ color: deckColors[2] }}>C</div>
        <div className="xy-corner xy-br" style={{ color: deckColors[3] }}>D</div>
        <div className="xy-axis xy-axis-h" />
        <div className="xy-axis xy-axis-v" />
        <div className="xy-knob" style={{ left: knobLeft, top: knobTop }} />
      </div>
    </div>
  );
}

function clamp(v: number, lo: number, hi: number) {
  return Math.max(lo, Math.min(hi, v));
}
