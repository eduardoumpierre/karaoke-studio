import { describe, expect, it } from "vitest";

import { clampOffset, currentLineIndex, formatOffset, lineLabel, lineStartSeconds } from "./logic";

const lines = [
  { time_ms: 1000, text: "a" },
  { time_ms: 3000, text: "b" },
  { time_ms: 5000, text: "" },
];

describe("currentLineIndex", () => {
  it("é -1 antes da primeira linha", () => {
    expect(currentLineIndex(lines, 0.5, 0)).toBe(-1);
    expect(currentLineIndex([], 10, 0)).toBe(-1);
  });
  it("troca exatamente no tempo da linha", () => {
    expect(currentLineIndex(lines, 1.0, 0)).toBe(0);
    expect(currentLineIndex(lines, 2.999, 0)).toBe(0);
    expect(currentLineIndex(lines, 3.0, 0)).toBe(1);
    expect(currentLineIndex(lines, 99, 0)).toBe(2);
  });
  it("offset positivo atrasa a letra, negativo adianta", () => {
    expect(currentLineIndex(lines, 3.2, 500)).toBe(0);
    expect(currentLineIndex(lines, 3.5, 500)).toBe(1);
    expect(currentLineIndex(lines, 2.6, -500)).toBe(1);
  });
});

describe("formatação", () => {
  it("offset", () => {
    expect(formatOffset(0)).toBe("0,0 s");
    expect(formatOffset(300)).toBe("+0,3 s");
    expect(formatOffset(-1500)).toBe("−1,5 s");
  });
  it("limita offset", () => {
    expect(clampOffset(99999)).toBe(30000);
    expect(clampOffset(-99999)).toBe(-30000);
    expect(clampOffset(120.4)).toBe(120);
  });
  it("linha vazia vira nota musical", () => {
    expect(lineLabel("  ")).toBe("♪");
    expect(lineLabel("oi")).toBe("oi");
  });
});

describe("clique na letra", () => {
  it("vai um pouco antes do verso e respeita o ajuste de tempo", () => {
    expect(lineStartSeconds(10_000, 0)).toBeCloseTo(9.7);
    expect(lineStartSeconds(10_000, 500)).toBeCloseTo(10.2); // letra atrasada: o verso soa depois
    expect(lineStartSeconds(100, 0)).toBe(0); // não passa do início
  });
});

describe("clique na letra", () => {
  it("o verso clicado já vira a linha atual ao pular para a entrada", async () => {
    const { currentLineIndex, lineStartSeconds, LINE_LEAD_IN_S } = await import("./logic");
    const lines = [
      { time_ms: 1000, text: "a" },
      { time_ms: 5000, text: "b" },
    ];
    const target = lineStartSeconds(5000, 0);
    expect(currentLineIndex(lines, target, 0)).toBe(0); // sem antecipação, destaca a anterior
    expect(currentLineIndex(lines, target, 0, LINE_LEAD_IN_S)).toBe(1);
  });
});

describe("clique na letra com atraso da saída", () => {
  it("o verso clicado já aparece destacado mesmo com o atraso do dispositivo", async () => {
    const { currentLineIndex, lineStartSeconds, LINE_LEAD_IN_S } = await import("./logic");
    const lines = [
      { time_ms: 1000, text: "a" },
      { time_ms: 5000, text: "b" },
    ];
    for (const latency of [0, 0.05, 0.12, 0.3]) {
      const target = lineStartSeconds(5000, 0, latency);
      // a letra usa a posição menos o atraso (o que você ouve)
      expect(currentLineIndex(lines, target - latency, 0, LINE_LEAD_IN_S)).toBe(1);
    }
  });
});
