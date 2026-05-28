import { useEffect, useState } from 'react';
import { api, EngineSnapshot } from './api';

/**
 * Polls the audio engine snapshot on an interval. The engine publishes its
 * state from the audio thread; here we just read the latest value.
 */
export function useEngineState(intervalMs = 33): EngineSnapshot | null {
  const [state, setState] = useState<EngineSnapshot | null>(null);

  useEffect(() => {
    let active = true;
    let timer: number;

    const poll = async () => {
      try {
        const snapshot = await api.getEngineState();
        if (active) setState(snapshot);
      } catch (e) {
        // Engine may not be ready during startup; ignore transient errors.
      }
      if (active) timer = window.setTimeout(poll, intervalMs);
    };

    poll();
    return () => {
      active = false;
      window.clearTimeout(timer);
    };
  }, [intervalMs]);

  return state;
}
