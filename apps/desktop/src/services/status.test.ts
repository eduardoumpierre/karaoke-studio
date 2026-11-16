import { describe, expect, it } from "vitest";

import {
  INITIAL_STATE,
  MAX_CONNECTING_ATTEMPTS,
  formatUptime,
  nextState,
  pollDelayMs,
  statusLabel,
  type BackendState,
  type Health,
} from "./status";

const health: Health = { status: "ok", version: "0.1.0", python: "3.11.9", uptime_seconds: 3 };
const fail = { ok: false as const, message: "falhou" };

describe("nextState", () => {
  it("vai para conectado quando o health responde", () => {
    expect(nextState(INITIAL_STATE, { ok: true, health }).kind).toBe("connected");
  });

  it("continua conectando nas primeiras falhas", () => {
    const s = nextState(INITIAL_STATE, fail);
    expect(s).toEqual({ kind: "connecting", attempt: 1 });
  });

  it("mostra erro depois de muitas falhas", () => {
    let s: BackendState = INITIAL_STATE;
    for (let i = 0; i < MAX_CONNECTING_ATTEMPTS; i++) s = nextState(s, fail);
    expect(s.kind).toBe("error");
  });

  it("uma falha depois de conectado vira erro e se recupera", () => {
    const connected: BackendState = { kind: "connected", health };
    const lost = nextState(connected, fail);
    expect(lost).toEqual({ kind: "error", message: "falhou", attempt: 1 });
    expect(nextState(lost, { ok: true, health }).kind).toBe("connected");
  });
});

describe("apresentação", () => {
  it("rótulos em português", () => {
    expect(statusLabel(INITIAL_STATE)).toBe("Conectando ao backend…");
    expect(statusLabel({ kind: "connected", health })).toBe("Conectado");
    expect(statusLabel({ kind: "error", message: "x", attempt: 1 })).toBe(
      "Sem conexão com o backend",
    );
  });

  it("consulta mais rápido enquanto não conectou", () => {
    expect(pollDelayMs(INITIAL_STATE)).toBeLessThan(pollDelayMs({ kind: "connected", health }));
  });

  it("formata o tempo ligado", () => {
    expect(formatUptime(5.9)).toBe("5 s");
    expect(formatUptime(125)).toBe("2 min 5 s");
    expect(formatUptime(3725)).toBe("1 h 2 min");
  });
});
