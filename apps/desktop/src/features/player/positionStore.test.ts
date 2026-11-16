import { describe, expect, it } from "vitest";

import { PositionStore, SEEK_HOLD_MAX_MS } from "./positionStore";

function make() {
  let t = 0;
  const store = new PositionStore(() => t);
  return { store, advance: (ms: number) => (t += ms) };
}

describe("PositionStore", () => {
  it("segue os avisos do motor e só notifica quando muda", () => {
    const { store } = make();
    let calls = 0;
    store.subscribe(() => calls++);
    store.fromEngine(1);
    store.fromEngine(1);
    store.fromEngine(2);
    expect(store.get()).toBe(2);
    expect(calls).toBe(2);
  });

  it("depois de um seek mostra o destino e ignora avisos antigos do motor", () => {
    const { store } = make();
    store.fromEngine(10);
    store.seekTo(120);
    expect(store.get()).toBe(120);
    store.fromEngine(10.03); // aviso emitido antes de o motor processar o pulo
    store.fromEngine(10.06);
    expect(store.get()).toBe(120); // não volta
    store.fromEngine(120.04); // o motor chegou
    expect(store.get()).toBe(120.04);
    store.fromEngine(120.07);
    expect(store.get()).toBe(120.07); // e segue normalmente
  });

  it("solta a trava se o motor nunca confirmar", () => {
    const { store, advance } = make();
    store.seekTo(50);
    store.fromEngine(3);
    expect(store.get()).toBe(50);
    advance(SEEK_HOLD_MAX_MS + 1);
    store.fromEngine(3.1);
    expect(store.get()).toBe(3.1);
  });

  it("um novo seek substitui o anterior", () => {
    const { store } = make();
    store.seekTo(30);
    store.seekTo(90);
    store.fromEngine(30.02); // confirmação do primeiro pulo, já superado
    expect(store.get()).toBe(90);
    store.fromEngine(90.1);
    expect(store.get()).toBe(90.1);
  });

  it("reset (música nova) cancela o seek em voo", () => {
    const { store } = make();
    store.seekTo(100);
    store.reset(0);
    store.fromEngine(0.03);
    expect(store.get()).toBe(0.03);
  });

  it("quem se desinscreve não é mais avisado", () => {
    const { store } = make();
    let calls = 0;
    const off = store.subscribe(() => calls++);
    store.fromEngine(1);
    off();
    store.fromEngine(2);
    expect(calls).toBe(1);
  });
});
