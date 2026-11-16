import { describe, expect, it } from "vitest";

import {
  clampBpm,
  clampShift,
  confidenceLabel,
  formatBpm,
  formatBytes,
  formatShift,
  keyLabel,
  relativeKey,
  tapTempo,
  transposeKey,
  formatElapsed,
} from "./logic";

describe("tons", () => {
  it("nomes em português", () => {
    expect(keyLabel(0, "major")).toBe("Dó maior");
    expect(keyLabel(9, "minor")).toBe("Lá menor");
    expect(keyLabel(6, "major")).toBe("Fá♯ maior");
    expect(keyLabel(12, "major")).toBe("Dó maior");
  });
  it("relativo", () => {
    expect(relativeKey(0, "major")).toEqual({ tonic: 9, mode: "minor" });
    expect(relativeKey(9, "minor")).toEqual({ tonic: 0, mode: "major" });
    expect(relativeKey(7, "major")).toEqual({ tonic: 4, mode: "minor" });
  });
});

describe("bpm", () => {
  it("limita e arredonda", () => {
    expect(clampBpm(5)).toBe(30);
    expect(clampBpm(999)).toBe(300);
    expect(clampBpm(120.04)).toBe(120);
    expect(clampBpm(240 / 2)).toBe(120);
  });
  it("formata", () => {
    expect(formatBpm(120)).toBe("120");
    expect(formatBpm(87.5)).toBe("87,5");
  });
  it("confiança", () => {
    expect(confidenceLabel(0.9)).toBe("confiança alta");
    expect(confidenceLabel(0.65)).toBe("confiança média");
    expect(confidenceLabel(0.3)).toBe("confiança baixa");
    expect(confidenceLabel(null)).toBe("");
  });
});

describe("transposição", () => {
  it("transpõe o tom", () => {
    expect(transposeKey(9, "minor", 3)).toEqual({ tonic: 0, mode: "minor" });
    expect(transposeKey(0, "major", -2)).toEqual({ tonic: 10, mode: "major" });
    expect(transposeKey(11, "major", 1)).toEqual({ tonic: 0, mode: "major" });
  });
  it("limita e formata", () => {
    expect(clampShift(9)).toBe(6);
    expect(clampShift(-9)).toBe(-6);
    expect(formatShift(0)).toBe("Original");
    expect(formatShift(2)).toBe("+2");
    expect(formatShift(-3)).toBe("−3");
    expect(formatBytes(500)).toBe("1 KB");
    expect(formatBytes(35 * 1024 * 1024)).toBe("35 MB");
  });
});

describe("tap tempo", () => {
  it("precisa de pelo menos 4 toques", () => {
    expect(tapTempo([0, 500, 1000])).toBeNull();
  });
  it("calcula pela mediana e ignora um toque fora do tempo", () => {
    expect(tapTempo([0, 500, 1000, 1500, 2000])).toBe(120);
    expect(tapTempo([0, 500, 1000, 1900, 2400, 2900])).toBe(120);
  });
});

describe("tempo decorrido", () => {
  it("formata segundos e minutos", () => {
    expect(formatElapsed(7.4)).toBe("7 s");
    expect(formatElapsed(75)).toBe("1 min 15 s");
  });
});
