import { useCallback, useEffect, useState } from 'react';
import { api, SyncMode, Track } from './api';
import { useEngineState } from './useEngineState';
import { Deck } from './components/Deck';
import { Mixer } from './components/Mixer';
import { Library } from './components/Library';
import { TransportBar } from './components/TransportBar';

interface DeckUi {
  track: Track | null;
}

export interface ChannelState {
  volume: number; // 0..100
  pitch: number; // -50..50
  sync: SyncMode;
  eq: { low: number; mid: number; high: number };
  /** DJM-style colour filter slider: -100 = full LP, 0 = bypass, +100 = full HP. */
  filter: number;
}

const EMPTY_DECK: DeckUi = { track: null };

const DEFAULT_CHANNEL: ChannelState = {
  volume: 100,
  pitch: 0,
  sync: 'tempo',
  eq: { low: 1, mid: 1, high: 1 },
  filter: 0,
};

const DECKS = [
  { id: 0, label: 'Deck A', color: '#e94560' }, // top-left  (XY −x, −y)
  { id: 1, label: 'Deck B', color: '#4ecdc4' }, // top-right (XY +x, −y)
  { id: 2, label: 'Deck C', color: '#f9c74f' }, // bottom-left
  { id: 3, label: 'Deck D', color: '#9d4edd' }, // bottom-right
];

export function App() {
  const snapshot = useEngineState();
  const [decks, setDecks] = useState<DeckUi[]>(() => DECKS.map(() => EMPTY_DECK));
  const [channels, setChannels] = useState<ChannelState[]>(() =>
    DECKS.map(() => DEFAULT_CHANNEL),
  );
  const [refreshKey, setRefreshKey] = useState(0);
  const [error, setError] = useState<string | null>(null);

  // Push UI defaults to the engine on mount (the engine starts with sync off,
  // so we have to send the initial sync mode for it to match the UI).
  useEffect(() => {
    DECKS.forEach((d) => api.setDeckSync(d.id, DEFAULT_CHANNEL.sync));
  }, []);

  const setChannel = useCallback((id: number, partial: Partial<ChannelState>) => {
    setChannels((prev) => {
      const next = [...prev];
      next[id] = { ...next[id], ...partial };
      return next;
    });
  }, []);

  const loadToDeck = useCallback(async (deck: number, path: string) => {
    try {
      setError(null);
      const track = await api.loadTrack(deck, path);
      setDecks((prev) => {
        const next = [...prev];
        next[deck] = { track };
        return next;
      });
      setRefreshKey((k) => k + 1);
    } catch (e) {
      setError(`Failed to load track: ${String(e)}`);
    }
  }, []);

  const pickAndLoad = useCallback(
    async (deck: number) => {
      const path = await api.openTrackDialog();
      if (path) await loadToDeck(deck, path);
    },
    [loadToDeck],
  );

  return (
    <div className="app">
      <div className="main">
        {error && <div className="error-banner">{error}</div>}

        <TransportBar snapshot={snapshot} />

        <div className="deck-row">
          {DECKS.map((d, i) => (
            <Deck
              key={d.id}
              deckId={d.id}
              label={d.label}
              color={d.color}
              track={decks[i].track}
              snapshot={snapshot?.decks[i]}
              globalBpm={snapshot?.tempo ?? 120}
              synced={channels[i].sync !== 'off'}
              onLoad={() => pickAndLoad(i)}
            />
          ))}

          <Mixer
            snapshot={snapshot}
            channels={channels}
            setChannel={setChannel}
            deckColors={DECKS.map((d) => d.color) as [string, string, string, string]}
          />
        </div>
      </div>

      <Library refreshKey={refreshKey} onLoadToDeck={loadToDeck} />
    </div>
  );
}
