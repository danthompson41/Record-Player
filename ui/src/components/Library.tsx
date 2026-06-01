import { useEffect, useState } from 'react';
import { api, formatTime, Track } from '../api';
import { WaveThumb } from './WaveThumb';

interface LibraryProps {
  /** Bumped whenever a new track is indexed, to trigger a refresh. */
  refreshKey: number;
  onLoadToDeck: (deck: number, path: string) => void;
}

export function Library({ refreshKey, onLoadToDeck }: LibraryProps) {
  const [query, setQuery] = useState('');
  const [results, setResults] = useState<Track[]>([]);

  useEffect(() => {
    let active = true;
    api
      .searchLibrary(query)
      .then((tracks) => {
        if (active) setResults(tracks);
      })
      .catch(() => {});
    return () => {
      active = false;
    };
  }, [query, refreshKey]);

  return (
    <div className="library">
      <div className="library-header">
        <h2 className="library-title">Library</h2>
        <input
          className="library-search"
          type="text"
          placeholder="Search title / artist / album…"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
        />
      </div>

      <div className="library-table">
        <div className="library-row library-head">
          <span>Track</span>
          <span>BPM</span>
          <span>Length</span>
          <span>Waveform</span>
          <span>Location</span>
          <span>Load</span>
        </div>

        <div className="library-body">
          {results.length === 0 && (
            <div className="library-empty">
              No tracks yet. Load a file into a deck to add it here.
            </div>
          )}
          {results.map((track) => (
            <div key={track.id} className="library-row">
              <span className="lib-name" title={track.title ?? track.path}>
                {track.title ?? track.path}
              </span>
              <span className="lib-bpm">
                {track.bpm != null ? Number(track.bpm.toFixed(1)) : '—'}
              </span>
              <span className="lib-len">
                {formatTime(track.duration_samples, track.sample_rate)}
              </span>
              <span className="lib-wave">
                <WaveThumb
                  rms={track.waveform}
                  durationSamples={track.duration_samples}
                  cues={track.cues}
                  beginPos={track.first_beat}
                  color="#6f9fd8"
                />
              </span>
              <span className="lib-loc" title={track.path}>
                {track.path}
              </span>
              <span className="lib-actions">
                <button onClick={() => onLoadToDeck(0, track.path)}>A</button>
                <button onClick={() => onLoadToDeck(1, track.path)}>B</button>
                <button onClick={() => onLoadToDeck(2, track.path)}>C</button>
                <button onClick={() => onLoadToDeck(3, track.path)}>D</button>
              </span>
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}
