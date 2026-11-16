import type { Song } from "../library/format";

export interface Preset {
  id: "traditional" | "reference" | "together";
  label: string;
  /** Volume da voz original, em %. */
  vocalPercent: number;
  hint: string;
}

export const PRESETS: Preset[] = [
  { id: "traditional", label: "Tradicional", vocalPercent: 0, hint: "Só o instrumental" },
  { id: "reference", label: "Referência", vocalPercent: 20, hint: "Voz original baixinha" },
  { id: "together", label: "Cantar junto", vocalPercent: 50, hint: "Voz original em meio volume" },
];

export interface MixState {
  masterPercent: number;
  vocalPercent: number;
  instrumentalPercent: number;
}

export const DEFAULT_MIX: MixState = {
  masterPercent: 100,
  vocalPercent: 20,
  instrumentalPercent: 100,
};

/** Curva perceptual: o controle de 0 a 100% vira ganho de 0 a 1 (quadrática, soa mais natural). */
export function percentToGain(percent: number): number {
  const p = Math.min(100, Math.max(0, percent)) / 100;
  return p * p;
}

export function toEngineMix(mix: MixState) {
  return {
    master: percentToGain(mix.masterPercent),
    vocal: percentToGain(mix.vocalPercent),
    instrumental: percentToGain(mix.instrumentalPercent),
  };
}

export function activePreset(mix: MixState): Preset["id"] | null {
  return PRESETS.find((p) => p.vocalPercent === mix.vocalPercent)?.id ?? null;
}

export function formatTime(seconds: number): string {
  const total = Math.max(0, Math.floor(seconds));
  return `${Math.floor(total / 60)}:${String(total % 60).padStart(2, "0")}`;
}

// ---------------------------------------------------------------- fila e histórico

export interface QueueState {
  current: Song | null;
  queue: Song[];
  history: Song[];
}

export const EMPTY_QUEUE: QueueState = { current: null, queue: [], history: [] };
export const HISTORY_LIMIT = 50;

/** Toca agora: a música atual vai para o início do histórico (sem repetir a mesma em seguida). */
export function playNow(state: QueueState, song: Song): QueueState {
  const history =
    state.current && state.current.id !== song.id
      ? [state.current, ...state.history]
      : state.history;
  return {
    current: song,
    // se a música estava na fila, sai de lá
    queue: state.queue.filter((s) => s.id !== song.id),
    history: dedupeHistory(history).slice(0, HISTORY_LIMIT),
  };
}

function dedupeHistory(list: Song[]): Song[] {
  const seen = new Set<number>();
  return list.filter((s) => (seen.has(s.id) ? false : (seen.add(s.id), true)));
}

export function enqueue(state: QueueState, song: Song): QueueState {
  if (state.queue.some((s) => s.id === song.id)) return state;
  return { ...state, queue: [...state.queue, song] };
}

export function removeFromQueue(state: QueueState, index: number): QueueState {
  return { ...state, queue: state.queue.filter((_, i) => i !== index) };
}

/** Próxima da fila vira a atual. Sem próxima, a atual vai para o histórico e fica sem música. */
export function advance(state: QueueState): { state: QueueState; next: Song | null } {
  const [next, ...rest] = state.queue;
  if (!next) return { state, next: null };
  return { state: { ...playNow({ ...state, queue: rest }, next) }, next };
}

export function clearQueue(state: QueueState): QueueState {
  return { ...state, queue: [] };
}
