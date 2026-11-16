import { describe, expect, it } from "vitest";

import {
  activeVoicePreset,
  DEFAULT_VOICE,
  presetFx,
  VOICE_PRESETS,
  dbToLinear,
  formatDb,
  levelAdvice,
  meterLevel,
  toEngineVoice,
} from "./logic";

describe("voz", () => {
  it("dB para linear", () => {
    expect(dbToLinear(0)).toBeCloseTo(1);
    expect(dbToLinear(20)).toBeCloseTo(10);
    expect(dbToLinear(-40)).toBeCloseTo(0.01);
    expect(dbToLinear(6)).toBeCloseTo(1.995, 2);
  });
  it("converte os ajustes para o motor", () => {
    const e = toEngineVoice({ ...DEFAULT_VOICE, singing: true });
    expect(e.monitor).toBe(true);
    expect(e.volume).toBeCloseTo(0.49);
    expect(e.inputGain).toBeCloseTo(1);
    expect(e.gateOn).toBe(true);
    expect(e.gateThreshold).toBeCloseTo(0.01);
    expect(toEngineVoice(DEFAULT_VOICE).monitor).toBe(false);
  });
  it("limita ganho e portão a faixas seguras", () => {
    const e = toEngineVoice({ ...DEFAULT_VOICE, gainDb: 99, gateDb: 0 });
    expect(e.inputGain).toBeCloseTo(dbToLinear(24));
    expect(e.gateThreshold).toBeCloseTo(dbToLinear(-20));
  });
  it("formata dB", () => {
    expect(formatDb(0)).toBe("0 dB");
    expect(formatDb(6)).toBe("+6 dB");
    expect(formatDb(-40)).toBe("−40 dB");
  });
  it("medidor em escala de dB", () => {
    expect(meterLevel(0)).toBe(0);
    expect(meterLevel(1)).toBe(1);
    expect(meterLevel(0.1)).toBeCloseTo(40 / 60);
    expect(meterLevel(0.001)).toBeCloseTo(0.0, 0);
  });
  it("avisos de nível", () => {
    expect(levelAdvice(0)).toBeNull();
    expect(levelAdvice(0.99)).toMatch(/estourando/);
    expect(levelAdvice(0.01)).toMatch(/baixo/);
    expect(levelAdvice(0.3)).toBeNull();
  });
});

describe("presets de voz", () => {
  it("o padrão é o preset Natural", () => {
    expect(activeVoicePreset(DEFAULT_VOICE)).toBe("natural");
  });
  it("cada preset é reconhecido e mexer à mão deixa de bater", () => {
    for (const p of VOICE_PRESETS) expect(activeVoicePreset(p.fx)).toBe(p.id);
    expect(
      activeVoicePreset({ ...(presetFx("studio") as object), compPercent: 46 } as never),
    ).toBeNull();
  });
  it("converte para as unidades do motor", () => {
    const v = toEngineVoice({ ...DEFAULT_VOICE, ...presetFx("echo")! });
    expect(v.comp).toBeCloseTo(0.3);
    expect(v.reverb).toBeCloseTo(0.2);
    expect(v.echo).toBeCloseTo(0.35);
    expect(v.echoMs).toBe(340);
  });
  it("limita valores fora da faixa", () => {
    const v = toEngineVoice({
      ...DEFAULT_VOICE,
      eqLowDb: 40,
      eqHighDb: -40,
      compPercent: 300,
      echoMs: 5,
    });
    expect(v.eqLowDb).toBe(12);
    expect(v.eqHighDb).toBe(-12);
    expect(v.comp).toBe(1);
    expect(v.echoMs).toBe(100);
  });
});

describe("autotune", () => {
  it("máscara de escala maior e menor", async () => {
    const { scaleMask, CHROMATIC_MASK } = await import("./logic");
    // Dó maior: Dó Ré Mi Fá Sol Lá Si = bits 0,2,4,5,7,9,11
    expect(scaleMask(0, "major")).toBe(0b101010110101);
    // Lá menor tem as mesmas notas que Dó maior
    expect(scaleMask(9, "minor")).toBe(scaleMask(0, "major"));
    // Tônica fora da faixa é normalizada
    expect(scaleMask(12, "major")).toBe(scaleMask(0, "major"));
    expect(scaleMask(-3, "major")).toBe(scaleMask(9, "major"));
    expect(CHROMATIC_MASK).toBe(0xfff);
  });

  it("escolhe a escala conforme o modo", async () => {
    const { tuneMask, scaleMask, DEFAULT_VOICE } = await import("./logic");
    const key = { tonic: 2, mode: "minor" as const };
    expect(tuneMask({ ...DEFAULT_VOICE, tuneScale: "song" }, key)).toBe(scaleMask(2, "minor"));
    // sem tom detectado, cai em todas as notas (nunca trava a voz)
    expect(tuneMask({ ...DEFAULT_VOICE, tuneScale: "song" }, null)).toBe(0xfff);
    expect(tuneMask({ ...DEFAULT_VOICE, tuneScale: "chromatic" }, key)).toBe(0xfff);
    expect(
      tuneMask({ ...DEFAULT_VOICE, tuneScale: "manual", tuneTonic: 7, tuneMode: "major" }, key),
    ).toBe(scaleMask(7, "major"));
  });

  it("converte para o motor e começa desligado", async () => {
    const { toEngineVoice, DEFAULT_VOICE } = await import("./logic");
    expect(DEFAULT_VOICE.tuneOn).toBe(false);
    const e = toEngineVoice(
      { ...DEFAULT_VOICE, tuneOn: true, tuneAmountPercent: 150, tuneSpeedPercent: 40 },
      { tonic: 0, mode: "major" },
    );
    expect(e).toMatchObject({ tuneOn: true, tuneAmount: 1, tuneSpeed: 0.4 });
    expect(e.tuneScale).toBe(0b101010110101);
  });

  it("reconhece o estilo ativo", async () => {
    const { activeTuneStyle, TUNE_STYLES } = await import("./logic");
    const natural = TUNE_STYLES.find((t) => t.id === "natural")!;
    expect(
      activeTuneStyle({
        tuneAmountPercent: natural.amountPercent,
        tuneSpeedPercent: natural.speedPercent,
      }),
    ).toBe("natural");
    expect(activeTuneStyle({ tuneAmountPercent: 51, tuneSpeedPercent: 40 })).toBeNull();
  });
});

describe("latência", () => {
  it("soma as partes e o ar", async () => {
    const { totalVoiceLatencyMs, AIR_MS } = await import("./logic");
    expect(totalVoiceLatencyMs({ inputMs: 10, bridgeMs: 25, outputMs: 20, tuneMs: 0 })).toBe(
      55 + AIR_MS,
    );
    expect(totalVoiceLatencyMs({ inputMs: 3, bridgeMs: 8, outputMs: 5, tuneMs: 12 })).toBe(
      28 + AIR_MS,
    );
  });

  it("converte o modo para o motor e começa estável", async () => {
    const { toEngineVoice, DEFAULT_VOICE } = await import("./logic");
    expect(DEFAULT_VOICE.latencyMode).toBe("stable");
    expect(toEngineVoice(DEFAULT_VOICE).latencyMode).toBe(0);
    expect(toEngineVoice({ ...DEFAULT_VOICE, latencyMode: "balanced" }).latencyMode).toBe(1);
    expect(toEngineVoice({ ...DEFAULT_VOICE, latencyMode: "minimal" }).latencyMode).toBe(2);
  });
});
