import type { VoiceSettings } from "../../services/player";
import type { Mode } from "../analysis/logic";
import { percentToGain } from "../player/logic";

export interface VoiceState {
  /** Cantar: abre o microfone e ouve a voz na saída. */
  singing: boolean;
  /** Volume do retorno da voz, em %. */
  volumePercent: number;
  /** Ganho de entrada, em dB. */
  gainDb: number;
  gateOn: boolean;
  /** Limiar do portão, em dB (negativo). */
  gateDb: number;
  // Efeitos
  eqLowDb: number;
  eqMidDb: number;
  eqHighDb: number;
  compPercent: number;
  reverbPercent: number;
  /** Tamanho da sala do reverb, em %. */
  reverbSizePercent: number;
  echoPercent: number;
  /** Tempo entre as repetições do eco, em ms. */
  echoMs: number;
  // Autotune
  tuneOn: boolean;
  /** Intensidade da correção, em %. */
  tuneAmountPercent: number;
  /** Velocidade da correção, em % (baixa = natural, alta = efeito "robô"). */
  tuneSpeedPercent: number;
  /** De onde vêm as notas permitidas. */
  tuneScale: "song" | "chromatic" | "manual";
  tuneTonic: number;
  tuneMode: Mode;
}

export type FxFields = Pick<
  VoiceState,
  | "eqLowDb"
  | "eqMidDb"
  | "eqHighDb"
  | "compPercent"
  | "reverbPercent"
  | "reverbSizePercent"
  | "echoPercent"
  | "echoMs"
>;

export interface VoicePreset {
  id: string;
  label: string;
  hint: string;
  fx: FxFields;
}

const NO_FX: FxFields = {
  eqLowDb: 0,
  eqMidDb: 0,
  eqHighDb: 0,
  compPercent: 0,
  reverbPercent: 0,
  reverbSizePercent: 50,
  echoPercent: 0,
  echoMs: 300,
};

export const VOICE_PRESETS: VoicePreset[] = [
  { id: "dry", label: "Seca", hint: "Sem efeitos: só a sua voz", fx: NO_FX },
  {
    id: "natural",
    label: "Natural",
    hint: "Um pouco de compressão e uma sala pequena: soa como você, mais firme",
    fx: { ...NO_FX, compPercent: 20, reverbPercent: 12, reverbSizePercent: 40 },
  },
  {
    id: "studio",
    label: "Estúdio",
    hint: "Voz mais presente e controlada, reverb curto",
    fx: {
      ...NO_FX,
      eqLowDb: -2,
      eqMidDb: 2,
      eqHighDb: 2,
      compPercent: 45,
      reverbPercent: 10,
      reverbSizePercent: 30,
    },
  },
  {
    id: "stage",
    label: "Palco",
    hint: "Reverb de salão com um toque de eco",
    fx: {
      ...NO_FX,
      eqMidDb: 2,
      eqHighDb: 1,
      compPercent: 35,
      reverbPercent: 32,
      reverbSizePercent: 65,
      echoPercent: 8,
      echoMs: 220,
    },
  },
  {
    id: "echo",
    label: "Eco",
    hint: "Repetições bem audíveis, como numa caverna de fita",
    fx: {
      ...NO_FX,
      compPercent: 30,
      reverbPercent: 20,
      reverbSizePercent: 55,
      echoPercent: 35,
      echoMs: 340,
    },
  },
  {
    id: "cathedral",
    label: "Catedral",
    hint: "Reverb grande e longo",
    fx: { ...NO_FX, compPercent: 25, reverbPercent: 55, reverbSizePercent: 95 },
  },
];

export function presetFx(id: string): FxFields | null {
  return VOICE_PRESETS.find((p) => p.id === id)?.fx ?? null;
}

/** O preset que bate exatamente com os efeitos atuais (ou null se foi mexido à mão). */
export function activeVoicePreset(v: FxFields): string | null {
  const keys = Object.keys(NO_FX) as (keyof FxFields)[];
  return VOICE_PRESETS.find((p) => keys.every((k) => p.fx[k] === v[k]))?.id ?? null;
}

export const EQ_DB_RANGE = { min: -12, max: 12 } as const;
export const ECHO_MS_RANGE = { min: 100, max: 800 } as const;

export const DEFAULT_VOICE: VoiceState = {
  singing: false,
  volumePercent: 70,
  gainDb: 0,
  gateOn: true,
  gateDb: -40,
  tuneOn: false,
  tuneAmountPercent: 50,
  tuneSpeedPercent: 35,
  tuneScale: "song",
  tuneTonic: 0,
  tuneMode: "major",
  ...(VOICE_PRESETS[1] as VoicePreset).fx,
};

export const GAIN_DB_RANGE = { min: -12, max: 24 } as const;
export const GATE_DB_RANGE = { min: -60, max: -20 } as const;

export function dbToLinear(db: number): number {
  return 10 ** (db / 20);
}

function clamp(n: number, min: number, max: number): number {
  return Math.max(min, Math.min(max, n));
}

export interface TuneStyle {
  id: string;
  label: string;
  hint: string;
  amountPercent: number;
  speedPercent: number;
}

export const TUNE_STYLES: TuneStyle[] = [
  {
    id: "subtle",
    label: "Sutil",
    hint: "Só dá um empurrãozinho: ninguém percebe, mas fica mais afinado",
    amountPercent: 35,
    speedPercent: 25,
  },
  {
    id: "natural",
    label: "Natural",
    hint: "Corrige de verdade e ainda soa como você",
    amountPercent: 60,
    speedPercent: 40,
  },
  {
    id: "firm",
    label: "Firme",
    hint: "Trava bem nas notas, com transições ainda suaves",
    amountPercent: 85,
    speedPercent: 70,
  },
  {
    id: "effect",
    label: "Efeito",
    hint: "Correção total e instantânea: o som “robô” dos hits",
    amountPercent: 100,
    speedPercent: 100,
  },
];

export function activeTuneStyle(v: Pick<VoiceState, "tuneAmountPercent" | "tuneSpeedPercent">) {
  return (
    TUNE_STYLES.find(
      (t) => t.amountPercent === v.tuneAmountPercent && t.speedPercent === v.tuneSpeedPercent,
    )?.id ?? null
  );
}

export const CHROMATIC_MASK = 0xfff;

/** Notas (12 bits, bit 0 = Dó) da escala maior ou menor natural com a tônica dada. */
export function scaleMask(tonic: number, mode: Mode): number {
  const steps = mode === "minor" ? [0, 2, 3, 5, 7, 8, 10] : [0, 2, 4, 5, 7, 9, 11];
  const base = ((Math.round(tonic) % 12) + 12) % 12;
  return steps.reduce((mask, step) => mask | (1 << ((base + step) % 12)), 0);
}

export interface SongKey {
  tonic: number;
  mode: Mode;
}

/** Escala que o autotune usa agora; `song` sem tom conhecido cai em todas as notas. */
export function tuneMask(v: VoiceState, songKey: SongKey | null): number {
  if (v.tuneScale === "manual") return scaleMask(v.tuneTonic, v.tuneMode);
  if (v.tuneScale === "song" && songKey) return scaleMask(songKey.tonic, songKey.mode);
  return CHROMATIC_MASK;
}

export function toEngineVoice(v: VoiceState, songKey: SongKey | null = null): VoiceSettings {
  return {
    monitor: v.singing,
    volume: percentToGain(v.volumePercent),
    inputGain: dbToLinear(Math.max(GAIN_DB_RANGE.min, Math.min(GAIN_DB_RANGE.max, v.gainDb))),
    gateOn: v.gateOn,
    gateThreshold: dbToLinear(Math.max(GATE_DB_RANGE.min, Math.min(GATE_DB_RANGE.max, v.gateDb))),
    eqLowDb: clamp(v.eqLowDb, EQ_DB_RANGE.min, EQ_DB_RANGE.max),
    eqMidDb: clamp(v.eqMidDb, EQ_DB_RANGE.min, EQ_DB_RANGE.max),
    eqHighDb: clamp(v.eqHighDb, EQ_DB_RANGE.min, EQ_DB_RANGE.max),
    comp: clamp(v.compPercent, 0, 100) / 100,
    reverb: clamp(v.reverbPercent, 0, 100) / 100,
    reverbSize: clamp(v.reverbSizePercent, 0, 100) / 100,
    echo: clamp(v.echoPercent, 0, 100) / 100,
    echoMs: clamp(v.echoMs, ECHO_MS_RANGE.min, ECHO_MS_RANGE.max),
    tuneOn: v.tuneOn,
    tuneAmount: clamp(v.tuneAmountPercent, 0, 100) / 100,
    tuneSpeed: clamp(v.tuneSpeedPercent, 0, 100) / 100,
    tuneScale: tuneMask(v, songKey),
  };
}

export function formatDb(db: number): string {
  return `${db > 0 ? "+" : db < 0 ? "−" : ""}${Math.abs(Math.round(db))} dB`;
}

/** Nível do medidor: o pico é linear, mas o ouvido (e o medidor) trabalham em dB. */
export function meterLevel(peak: number): number {
  if (peak <= 0.0005) return 0;
  const db = 20 * Math.log10(Math.min(1, peak));
  return Math.max(0, Math.min(1, (db + 60) / 60));
}

/** Nível de pico em dBFS para exibir, ou null se não houver sinal. */
export function peakDb(peak: number): number | null {
  return peak <= 0.0005 ? null : 20 * Math.log10(Math.min(1, peak));
}

/** Aviso de nível: pico muito baixo ou estourando. */
export function levelAdvice(peak: number): string | null {
  const db = peakDb(peak);
  if (db === null) return null;
  if (db > -1) return "Está estourando: diminua o ganho.";
  if (db < -30) return "Sinal baixo: chegue mais perto do microfone ou aumente o ganho.";
  return null;
}
