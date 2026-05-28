interface MeterProps {
  /** Peak level 0..1 (may exceed 1 on clipping). */
  level: number;
}

/** A vertical peak meter. */
export function Meter({ level }: MeterProps) {
  const pct = Math.min(100, Math.max(0, level * 100));
  const clipping = level >= 0.99;
  return (
    <div className="meter">
      <div
        className="meter-fill"
        style={{
          height: `${pct}%`,
          background: clipping ? '#ff3b3b' : undefined,
        }}
      />
    </div>
  );
}
