export interface Line {
  time_ms: number;
  text: string;
}

export const OFFSET_STEP_MS = 100;
export const OFFSET_LIMIT_MS = 30_000;

/**
 * Índice da linha atual: a última cuja hora (já deslocada pelo ajuste) é ≤ posição.
 * Offset positivo atrasa a letra. Retorna -1 antes da primeira linha.
 */
export function currentLineIndex(lines: Line[], positionSeconds: number, offsetMs: number): number {
  const at = positionSeconds * 1000 - offsetMs;
  let lo = 0;
  let hi = lines.length - 1;
  let found = -1;
  while (lo <= hi) {
    const mid = (lo + hi) >> 1;
    if ((lines[mid]?.time_ms ?? Infinity) <= at) {
      found = mid;
      lo = mid + 1;
    } else {
      hi = mid - 1;
    }
  }
  return found;
}

export function clampOffset(ms: number): number {
  return Math.max(-OFFSET_LIMIT_MS, Math.min(OFFSET_LIMIT_MS, Math.round(ms)));
}

export function formatOffset(ms: number): string {
  if (ms === 0) return "0,0 s";
  return `${ms > 0 ? "+" : "−"}${(Math.abs(ms) / 1000).toLocaleString("pt-BR", {
    minimumFractionDigits: 1,
    maximumFractionDigits: 1,
  })} s`;
}

/** Texto para mostrar no lugar de uma linha vazia (pausa instrumental). */
export function lineLabel(text: string): string {
  return text.trim() === "" ? "♪" : text;
}
