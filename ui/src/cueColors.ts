// Global cue colors (index 0 = cue 1), shared across all decks.
//
// A palette inspired by Van Gogh's "The Starry Night" — swirling cobalt and
// ultramarine blues, chrome star-yellow and amber village lights, teal and a
// twilight violet — arranged in a deliberately shuffled (non-gradient) order so
// neighbouring cues contrast. Kept mid-bright so each reads on the dark panel.
export const CUE_COLORS: string[] = [
  '#ffd23f', // chrome star yellow
  '#4a78c4', // cobalt
  '#3fb0a0', // teal swirl
  '#f0a500', // amber village light
  '#2e5eaa', // deep ultramarine
  '#e8c84a', // pale gold / moonlight
  '#8e7cc3', // twilight violet
  '#6fb7e0', // sky swirl
];

// Color for the "first bar" / beginning cue (the ⇤ button). A pale moonlight
// blue-white, distinct from the eight numbered cue colors.
export const BEGIN_COLOR = '#cdd9f0';

/** Pick readable text (dark/light) for a hex background by luminance. */
export function textOn(hex: string): string {
  const r = parseInt(hex.slice(1, 3), 16);
  const g = parseInt(hex.slice(3, 5), 16);
  const b = parseInt(hex.slice(5, 7), 16);
  const lum = (0.299 * r + 0.587 * g + 0.114 * b) / 255;
  return lum > 0.6 ? '#1a1400' : '#ffffff';
}
