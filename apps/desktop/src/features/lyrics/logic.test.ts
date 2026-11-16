import { describe, expect, it } from "vitest";

import { clampOffset, currentLineIndex, formatOffset, lineLabel } from "./logic";

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
