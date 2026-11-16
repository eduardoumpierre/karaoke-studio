import type { components } from "@karaoke/api-client";

export type Health = components["schemas"]["Health"];

export type BackendState =
  | { kind: "connecting"; attempt: number }
  | { kind: "connected"; health: Health }
  | { kind: "error"; message: string; attempt: number };

export const INITIAL_STATE: BackendState = { kind: "connecting", attempt: 0 };

/** Depois de tantas falhas seguidas, deixa de mostrar "conectando" e mostra o erro. */
export const MAX_CONNECTING_ATTEMPTS = 10;

export type CheckResult = { ok: true; health: Health } | { ok: false; message: string };

export function nextState(prev: BackendState, result: CheckResult): BackendState {
  if (result.ok) return { kind: "connected", health: result.health };
  const attempt = prev.kind === "connected" ? 1 : prev.attempt + 1;
  if (prev.kind === "connected" || attempt >= MAX_CONNECTING_ATTEMPTS) {
    return { kind: "error", message: result.message, attempt };
  }
  return prev.kind === "error"
    ? { kind: "error", message: result.message, attempt }
    : { kind: "connecting", attempt };
}

export function pollDelayMs(state: BackendState): number {
  return state.kind === "connected" ? 5000 : 1500;
}

export function statusLabel(state: BackendState): string {
  switch (state.kind) {
    case "connecting":
      return "Conectando ao backend…";
    case "connected":
      return "Conectado";
    case "error":
      return "Sem conexão com o backend";
  }
}

export function formatUptime(seconds: number): string {
  const s = Math.floor(seconds);
  if (s < 60) return `${s} s`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m} min ${s % 60} s`;
  return `${Math.floor(m / 60)} h ${m % 60} min`;
}
