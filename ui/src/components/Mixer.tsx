import { useState } from 'react';
import { api, EngineSnapshot, SyncMode } from '../api';
import { ChannelState } from '../App';
import { Meter } from './Meter';

interface MixerProps {
  snapshot: EngineSnapshot | null;
  channels: ChannelState[];
  setChannel: (id: number, partial: Partial<ChannelState>) => void;
  deckColors: [string, string];
}

const SYNC_MODES: SyncMode[] = ['off', 'tempo', 'phase'];

export function Mixer({ snapshot, channels, setChannel, deckColors }: MixerProps) {
  const [crossfader, setCrossfader] = useState(0); // -100..100
  const [master, setMaster] = useState(100); // 0..120

  const onCrossfader = (v: number) => {
    setCrossfader(v);
    api.setCrossfader(v / 100);
  };

  const onMaster = (v: number) => {
    setMaster(v);
    api.setMasterGain(v / 100);
  };

  return (
    <div className="mixer">
      <div className="mixer-strips">
        <ChannelStrip
          id={0}
          label="A"
          color={deckColors[0]}
          state={channels[0]}
          setState={(p) => setChannel(0, p)}
          peakL={snapshot?.decks[0]?.peak_left ?? 0}
          peakR={snapshot?.decks[0]?.peak_right ?? 0}
          enginePitch={snapshot?.decks[0]?.pitch ?? 1}
        />

        <MasterStrip
          master={master}
          onMaster={onMaster}
          peakL={snapshot?.master_left ?? 0}
          peakR={snapshot?.master_right ?? 0}
        />

        <ChannelStrip
          id={1}
          label="B"
          color={deckColors[1]}
          state={channels[1]}
          setState={(p) => setChannel(1, p)}
          peakL={snapshot?.decks[1]?.peak_left ?? 0}
          peakR={snapshot?.decks[1]?.peak_right ?? 0}
          enginePitch={snapshot?.decks[1]?.pitch ?? 1}
        />
      </div>

      <div className="crossfader-row">
        <span className="mixer-label">A</span>
        <input
          type="range"
          className="crossfader"
          min={-100}
          max={100}
          value={crossfader}
          onChange={(e) => onCrossfader(Number(e.target.value))}
        />
        <span className="mixer-label">B</span>
      </div>
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

  return (
    <div className="mixer-strip">
      <div className="strip-label" style={{ color }}>
        {label}
      </div>

      <div className="strip-sync">
        {SYNC_MODES.map((mode) => (
          <button
            key={mode}
            className={`sync-btn ${state.sync === mode ? 'active' : ''}`}
            onClick={() => onSync(mode)}
            title={mode === 'off' ? 'Free pitch' : 'Match global BPM'}
          >
            {mode === 'off' ? 'off' : mode === 'tempo' ? 'sync' : 'phase'}
          </button>
        ))}
      </div>

      <div className="strip-pitch">
        <input
          type="range"
          min={-50}
          max={50}
          value={Math.round(displayedPitch)}
          disabled={synced}
          onChange={(e) => onPitch(Number(e.target.value))}
        />
        <span className="strip-pitch-val">
          {displayedPitch > 0 ? `+${displayedPitch.toFixed(0)}` : displayedPitch.toFixed(0)}%
        </span>
      </div>

      <div className="strip-eq">
        {(['high', 'mid', 'low'] as const).map((band) => (
          <label key={band} className="eq-vertical">
            <input
              type="range"
              min={0}
              max={2}
              step={0.01}
              value={state.eq[band]}
              onChange={(e) => onEq(band, Number(e.target.value))}
            />
            <span>{band.toUpperCase()}</span>
          </label>
        ))}
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

interface MasterStripProps {
  master: number;
  onMaster: (v: number) => void;
  peakL: number;
  peakR: number;
}

function MasterStrip({ master, onMaster, peakL, peakR }: MasterStripProps) {
  return (
    <div className="mixer-strip master-strip">
      <div className="strip-label">M</div>
      {/* Empty placeholders keep the master fader aligned with the channel
          strips' faders (which sit below sync/pitch/eq rows). */}
      <div className="strip-sync strip-placeholder" />
      <div className="strip-pitch strip-placeholder" />
      <div className="strip-eq strip-placeholder" />
      <div className="strip-fader">
        <input
          type="range"
          className="vol-fader"
          min={0}
          max={120}
          value={master}
          onChange={(e) => onMaster(Number(e.target.value))}
        />
        <div className="strip-meters">
          <Meter level={peakL} />
          <Meter level={peakR} />
        </div>
      </div>
    </div>
  );
}
