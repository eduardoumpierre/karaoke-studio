import { describe, expect, it } from "vitest";

import type { Song } from "../library/format";
import {
  activePreset,
  advance,
  clearQueue,
  DEFAULT_MIX,
  EMPTY_QUEUE,
  enqueue,
  formatTime,
  fractionFromPointer,
  HISTORY_LIMIT,
  keySeekTarget,
  percentToGain,
  playNow,
  PRESETS,
  removeFromQueue,
  toEngineMix,
} from "./logic";

const song = (id: number): Song =>
  ({ id, title: `Música ${id}`, artist: "", album: "", status: "separated" }) as Song;

describe("volume", () => {
  it("curva quadrática limitada a 0–1", () => {
    expect(percentToGain(0)).toBe(0);
    expect(percentToGain(100)).toBe(1);
    expect(percentToGain(50)).toBeCloseTo(0.25);
    expect(percentToGain(-10)).toBe(0);
    expect(percentToGain(250)).toBe(1);
  });

  it("converte a mixagem para o motor", () => {
    expect(toEngineMix(DEFAULT_MIX)).toEqual({
      master: 1,
      vocal: 0.04000000000000001,
      instrumental: 1,
    });
  });

  it("presets: tradicional 0%, referência 20%, cantar junto 50%", () => {
    expect(PRESETS.map((p) => p.vocalPercent)).toEqual([0, 20, 50]);
    expect(activePreset({ ...DEFAULT_MIX, vocalPercent: 0 })).toBe("traditional");
    expect(activePreset({ ...DEFAULT_MIX, vocalPercent: 20 })).toBe("reference");
    expect(activePreset({ ...DEFAULT_MIX, vocalPercent: 50 })).toBe("together");
    expect(activePreset({ ...DEFAULT_MIX, vocalPercent: 33 })).toBeNull();
  });

  it("formata tempo", () => {
    expect(formatTime(0)).toBe("0:00");
    expect(formatTime(65.9)).toBe("1:05");
    expect(formatTime(-3)).toBe("0:00");
    expect(formatTime(3725)).toBe("62:05");
  });
});

describe("fila e histórico", () => {
  it("tocar agora move a atual para o histórico e tira da fila", () => {
    let s = playNow(EMPTY_QUEUE, song(1));
    s = enqueue(enqueue(s, song(2)), song(3));
    s = playNow(s, song(2));
    expect(s.current?.id).toBe(2);
    expect(s.queue.map((x) => x.id)).toEqual([3]);
    expect(s.history.map((x) => x.id)).toEqual([1]);
  });

  it("não enfileira repetida nem repete no histórico", () => {
    let s = enqueue(enqueue(EMPTY_QUEUE, song(1)), song(1));
    expect(s.queue).toHaveLength(1);
    s = playNow(playNow(playNow(EMPTY_QUEUE, song(1)), song(2)), song(1));
    s = playNow(s, song(2));
    expect(s.history.map((x) => x.id)).toEqual([1, 2]);
    expect(s.current?.id).toBe(2);
  });

  it("avançar toca a próxima; sem fila, não muda", () => {
    let s = playNow(EMPTY_QUEUE, song(1));
    s = enqueue(enqueue(s, song(2)), song(3));
    const a = advance(s);
    expect(a.next?.id).toBe(2);
    expect(a.state.current?.id).toBe(2);
    expect(a.state.queue.map((x) => x.id)).toEqual([3]);
    const b = advance(advance(a.state).state);
    expect(b.next).toBeNull();
    expect(b.state.current?.id).toBe(3);
  });

  it("remove da fila e limpa", () => {
    const s = enqueue(enqueue(enqueue(EMPTY_QUEUE, song(1)), song(2)), song(3));
    expect(removeFromQueue(s, 1).queue.map((x) => x.id)).toEqual([1, 3]);
    expect(clearQueue(s).queue).toEqual([]);
  });

  it("limita o histórico", () => {
    let s = EMPTY_QUEUE;
    for (let i = 1; i <= HISTORY_LIMIT + 20; i++) s = playNow(s, song(i));
    expect(s.history).toHaveLength(HISTORY_LIMIT);
    expect(s.history[0]?.id).toBe(HISTORY_LIMIT + 19);
  });
});

describe("barra de tempo", () => {
  it("converte a posição do ponteiro em fração limitada a 0–1", () => {
    expect(fractionFromPointer(150, 100, 200)).toBe(0.25);
    expect(fractionFromPointer(50, 100, 200)).toBe(0); // antes do início
    expect(fractionFromPointer(999, 100, 200)).toBe(1); // depois do fim
    expect(fractionFromPointer(10, 0, 0)).toBe(0); // largura zero
  });

  it("teclas: setas ±5 s, PageUp/Down ±15 s, Home e End, sempre dentro da música", () => {
    expect(keySeekTarget("ArrowRight", 60, 200)).toBe(65);
    expect(keySeekTarget("ArrowLeft", 60, 200)).toBe(55);
    expect(keySeekTarget("PageUp", 60, 200)).toBe(75);
    expect(keySeekTarget("PageDown", 10, 200)).toBe(0);
    expect(keySeekTarget("ArrowRight", 198, 200)).toBe(200);
    expect(keySeekTarget("Home", 90, 200)).toBe(0);
    expect(keySeekTarget("End", 90, 200)).toBe(199.5);
    expect(keySeekTarget("a", 90, 200)).toBeNull();
  });
});
