import { useCallback, useState } from 'react';
import { api, Track } from './api';
import { useEngineState } from './useEngineState';
import { Deck } from './components/Deck';
import { Mixer } from './components/Mixer';
import { Library } from './components/Library';
import { TransportBar } from './components/TransportBar';

interface DeckUi {
  track: Track | null;
}

const EMPTY_DECK: DeckUi = { track: null };

const DECKS = [
  { id: 0, label: 'Deck A', color: '#e94560' },
  { id: 1, label: 'Deck B', color: '#4ecdc4' },
];

export function App() {
  const snapshot = useEngineState();
  const [decks, setDecks] = useState<DeckUi[]>([EMPTY_DECK, EMPTY_DECK]);
  const [refreshKey, setRefreshKey] = useState(0);
  const [error, setError] = useState<string | null>(null);

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

        <div className="decks-container">
          {DECKS.map((d) => (
            <Deck
              key={d.id}
              deckId={d.id}
              label={d.label}
              color={d.color}
              track={decks[d.id].track}
              snapshot={snapshot?.decks[d.id]}
              globalBpm={snapshot?.tempo ?? 120}
              onLoad={() => pickAndLoad(d.id)}
            />
          ))}
        </div>

        <Mixer snapshot={snapshot} />
      </div>

      <Library refreshKey={refreshKey} onLoadToDeck={loadToDeck} />
    </div>
  );
}
