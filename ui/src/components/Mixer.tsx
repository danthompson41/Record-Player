import { useState } from 'react';
import { api, EngineSnapshot } from '../api';
import { Meter } from './Meter';

interface MixerProps {
  snapshot: EngineSnapshot | null;
}

export function Mixer({ snapshot }: MixerProps) {
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
      <div className="mixer-section crossfader-section">
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

      <div className="mixer-section">
        <label className="slider-row">
          <span>Master</span>
          <input
            type="range"
            min={0}
            max={120}
            value={master}
            onChange={(e) => onMaster(Number(e.target.value))}
          />
          <span className="slider-val">{master}</span>
        </label>
      </div>

      <div className="mixer-section master-meters">
        <Meter level={snapshot?.master_left ?? 0} />
        <Meter level={snapshot?.master_right ?? 0} />
      </div>
    </div>
  );
}
