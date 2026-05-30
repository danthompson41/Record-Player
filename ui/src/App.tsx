import { useCallback, useState } from 'react';
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
}

const EMPTY_DECK: DeckUi = { track: null };

const DEFAULT_CHANNEL: ChannelState = {
  volume: 100,
  pitch: 0,
  sync: 'off',
  eq: { low: 1, mid: 1, high: 1 },
};

const DECKS = [
  { id: 0, label: 'Deck A', color: '#e94560' },
  { id: 1, label: 'Deck B', color: '#4ecdc4' },
];

export function App() {
  const snapshot = useEngineState();
  const [decks, setDecks] = useState<DeckUi[]>([EMPTY_DECK, EMPTY_DECK]);
  const [channels, setChannels] = useState<ChannelState[]>([
    DEFAULT_CHANNEL,
    DEFAULT_CHANNEL,
  ]);
  const [refreshKey, setRefreshKey] = useState(0);
  const [error, setError] = useState<string | null>(null);

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
          <Deck
            key={DECKS[0].id}
            deckId={DECKS[0].id}
            label={DECKS[0].label}
            color={DECKS[0].color}
            track={decks[0].track}
            snapshot={snapshot?.decks[0]}
            globalBpm={snapshot?.tempo ?? 120}
            synced={channels[0].sync !== 'off'}
            onLoad={() => pickAndLoad(0)}
          />

          <Mixer
            snapshot={snapshot}
            channels={channels}
            setChannel={setChannel}
            deckColors={[DECKS[0].color, DECKS[1].color]}
          />

          <Deck
            key={DECKS[1].id}
            deckId={DECKS[1].id}
            label={DECKS[1].label}
            color={DECKS[1].color}
            track={decks[1].track}
            snapshot={snapshot?.decks[1]}
            globalBpm={snapshot?.tempo ?? 120}
            synced={channels[1].sync !== 'off'}
            onLoad={() => pickAndLoad(1)}
          />
        </div>
      </div>

      <Library refreshKey={refreshKey} onLoadToDeck={loadToDeck} />
    </div>
  );
}
