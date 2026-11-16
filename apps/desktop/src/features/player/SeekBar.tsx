import { useRef, useState } from "react";

import { fractionFromPointer, formatTime, keySeekTarget } from "./logic";
import { usePosition, type PlayerState } from "./usePlayer";

/**
 * Barra de tempo: clique em qualquer ponto para ir até lá, arraste para procurar e passe o mouse
 * para ver o tempo do ponto. Também funciona pelo teclado (setas, PageUp/PageDown, Home/End).
 */
export function SeekBar({
  player,
  disabled,
  onSeek,
}: {
  player: Pick<PlayerState, "positionStore" | "duration">;
  disabled: boolean;
  onSeek: (seconds: number) => void;
}) {
  const position = usePosition(player);
  const { duration } = player;
  const trackRef = useRef<HTMLDivElement>(null);
  const [drag, setDrag] = useState<number | null>(null);
  const [hover, setHover] = useState<number | null>(null);

  const shown = drag ?? position;
  const pct = duration > 0 ? Math.max(0, Math.min(100, (shown / duration) * 100)) : 0;

  const timeAt = (clientX: number): number => {
    const rect = trackRef.current?.getBoundingClientRect();
    if (!rect) return 0;
    return fractionFromPointer(clientX, rect.left, rect.width) * duration;
  };

  const onPointerDown = (e: React.PointerEvent<HTMLDivElement>) => {
    if (disabled || e.button !== 0) return;
    e.currentTarget.setPointerCapture(e.pointerId);
    setDrag(timeAt(e.clientX));
  };

  const onPointerMove = (e: React.PointerEvent<HTMLDivElement>) => {
    if (disabled) return;
    const t = timeAt(e.clientX);
    setHover(t);
    if (drag !== null) setDrag(t);
  };

  const onPointerUp = (e: React.PointerEvent<HTMLDivElement>) => {
    if (drag === null) return;
    const t = timeAt(e.clientX);
    setDrag(null);
    onSeek(t);
  };

  const onKeyDown = (e: React.KeyboardEvent<HTMLDivElement>) => {
    if (disabled) return;
    const target = keySeekTarget(e.key, position, duration);
    if (target === null) return;
    e.preventDefault();
    onSeek(target);
  };

  const tipPct = hover !== null && duration > 0 ? (hover / duration) * 100 : 0;
  const tip = drag ?? hover;

  return (
    <div className="seek">
      <span className="time">{formatTime(shown)}</span>
      <div
        ref={trackRef}
        className={drag !== null ? "seek-track dragging" : "seek-track"}
        role="slider"
        tabIndex={disabled ? -1 : 0}
        aria-label="Posição"
        aria-valuemin={0}
        aria-valuemax={Math.round(duration)}
        aria-valuenow={Math.round(shown)}
        aria-valuetext={`${formatTime(shown)} de ${formatTime(duration)}`}
        aria-disabled={disabled}
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
        onPointerCancel={() => setDrag(null)}
        onPointerLeave={() => setHover(null)}
        onKeyDown={onKeyDown}
      >
        <div className="seek-rail">
          <div className="seek-fill" style={{ width: `${pct}%` }} />
        </div>
        <div className="seek-thumb" style={{ left: `${pct}%` }} />
        {!disabled && tip !== null && (
          <div
            className="seek-tip"
            style={{ left: `clamp(22px, ${drag !== null ? pct : tipPct}%, calc(100% - 22px))` }}
          >
            {formatTime(tip)}
          </div>
        )}
      </div>
      <span className="time">{formatTime(duration)}</span>
    </div>
  );
}
