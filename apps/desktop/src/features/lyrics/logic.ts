export interface Line {
  time_ms: number;
  text: string;
}

export const OFFSET_STEP_MS = 100;
export const OFFSET_LIMIT_MS = 30_000;

/** Quanto antes do verso o clique na letra começa, para dar tempo de respirar e entrar cantando. */
export const LINE_LEAD_IN_S = 0.3;

/**
 * Índice da linha atual: a última cuja hora (já deslocada pelo ajuste) é ≤ posição.
 * Offset positivo atrasa a letra. Retorna -1 antes da primeira linha.
 * `lookaheadS` antecipa a troca de linha (usado para casar com a entrada do clique na letra).
 */
export function currentLineIndex(
  lines: Line[],
  positionSeconds: number,
  offsetMs: number,
  lookaheadS = 0,
): number {
  const at = (positionSeconds + lookaheadS) * 1000 - offsetMs;
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

/**
 * Posição (s) para onde o clique no verso pula: um pouco antes do verso (a entrada), já com o
 * ajuste de tempo da letra. `latencyS` é o atraso da saída: a letra é marcada pelo que você
 * ouve (posição − atraso), então o pulo soma o atraso para o verso já aparecer destacado.
 */
export function lineStartSeconds(timeMs: number, offsetMs: number, latencyS = 0): number {
  return Math.max(0, (timeMs + offsetMs) / 1000 - LINE_LEAD_IN_S + latencyS);
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
