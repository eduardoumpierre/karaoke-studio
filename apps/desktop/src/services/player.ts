import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

import { insideTauri } from "./files";

export interface DeviceInfo {
  name: string;
  isDefault: boolean;
}

export interface OutputInfo {
  name: string;
  sampleRate: number;
  channels: number;
}

export interface DevicesReply {
  devices: { outputs: DeviceInfo[]; inputs: DeviceInfo[] };
  /** Escolhas salvas; null = padrão do sistema. */
  outputDevice: string | null;
  inputDevice: string | null;
  output: OutputInfo | null;
}

export interface LoadRequest {
  vocals: string | null;
  instrumental: string;
  autoplay: boolean;
  start?: number;
}

export interface LoadedInfo {
  duration: number;
  hasVocals: boolean;
}

export interface MicInfo {
  name: string;
  sampleRate: number;
  channels: number;
}

/** Ajustes da voz ao vivo, nas unidades do motor (ganhos lineares). */
export interface VoiceSettings {
  monitor: boolean;
  volume: number;
  inputGain: number;
  gateOn: boolean;
  gateThreshold: number;
  /** Efeitos: EQ em dB (−12…12); os demais de 0 a 1; o tempo do eco em ms. */
  eqLowDb: number;
  eqMidDb: number;
  eqHighDb: number;
  comp: number;
  reverb: number;
  reverbSize: number;
  echo: number;
  echoMs: number;
  /** Autotune: ligado, intensidade e velocidade (0 a 1) e notas permitidas (12 bits, bit 0 = Dó). */
  tuneOn: boolean;
  tuneAmount: number;
  tuneSpeed: number;
  tuneScale: number;
}

export interface EngineMix {
  master: number;
  vocal: number;
  instrumental: number;
}

export type PlayerEvent =
  | {
      type: "state";
      position: number;
      duration: number;
      playing: boolean;
      peakL: number;
      peakR: number;
    }
  | { type: "ended" }
  | { type: "error"; message: string }
  | { type: "micLevel"; peak: number }
  | { type: "voice"; bridgeMs: number; targetMs: number; underruns: number; tuneMs: number };

/** O que a interface pode pedir ao motor de áudio (Rust, via Tauri). */
export interface PlayerService {
  devices(): Promise<DevicesReply>;
  setOutput(name: string | null): Promise<OutputInfo>;
  setInput(name: string | null): Promise<void>;
  load(request: LoadRequest): Promise<LoadedInfo>;
  play(): Promise<void>;
  pause(): Promise<void>;
  seek(seconds: number): Promise<void>;
  setMix(mix: EngineMix): Promise<void>;
  micTest(enable: boolean): Promise<string | null>;
  /** Liga/desliga o retorno da voz e aplica os ajustes. Devolve o microfone aberto (ou null). */
  voice(settings: VoiceSettings): Promise<MicInfo | null>;
  onEvent(callback: (event: PlayerEvent) => void): Promise<() => void>;
  /** true quando é só uma simulação (navegador, sem som de verdade). */
  readonly simulated: boolean;
}

function tauriPlayer(): PlayerService {
  return {
    simulated: false,
    devices: () => invoke<DevicesReply>("player_devices"),
    setOutput: (name) => invoke<OutputInfo>("player_set_output", { name }),
    setInput: (name) => invoke<void>("player_set_input", { name }),
    load: (request) => invoke<LoadedInfo>("player_load", { request }),
    play: () => invoke<void>("player_play"),
    pause: () => invoke<void>("player_pause"),
    seek: (seconds) => invoke<void>("player_seek", { seconds }),
    setMix: (mix) => invoke<void>("player_set_mix", { mix }),
    micTest: (enable) => invoke<string | null>("player_mic_test", { enable }),
    voice: (settings) => invoke<MicInfo | null>("player_voice", { settings }),
    onEvent: (callback) => listen<PlayerEvent>("player-event", (e) => callback(e.payload)),
  };
}

/**
 * Simulação para desenvolver a interface no navegador (sem Tauri): não toca nada, só avança
 * o relógio. Os dispositivos listados são fictícios.
 */
export function simulatedPlayer(): PlayerService {
  const listeners = new Set<(e: PlayerEvent) => void>();
  const emit = (e: PlayerEvent) => listeners.forEach((l) => l(e));
  let duration = 0;
  let position = 0;
  let playing = false;
  let micTimer: ReturnType<typeof setInterval> | undefined;
  let voiceTimer: ReturnType<typeof setInterval> | undefined;
  let output: string | null = null;
  let input: string | null = null;
  const outputs = [
    { name: "Alto-falantes (EDIFIER R19BT)", isDefault: true },
    { name: "Fones (Realtek Audio)", isDefault: false },
  ];
  const inputs = [
    { name: "Microfone (Realtek Audio)", isDefault: true },
    { name: "Headset Sennheiser", isDefault: false },
  ];
  setInterval(() => {
    if (playing) {
      position = Math.min(duration, position + 0.033);
      if (position >= duration) {
        playing = false;
        emit({ type: "ended" });
      }
    }
    emit({
      type: "state",
      position,
      duration,
      playing,
      peakL: playing ? 0.3 + Math.random() * 0.4 : 0,
      peakR: playing ? 0.3 + Math.random() * 0.4 : 0,
    });
  }, 33);
  const info = (): OutputInfo => ({
    name: output ?? outputs[0]!.name,
    sampleRate: 48000,
    channels: 2,
  });
  return {
    simulated: true,
    devices: async () => ({
      devices: { outputs, inputs },
      outputDevice: output,
      inputDevice: input,
      output: info(),
    }),
    setOutput: async (name) => ((output = name), info()),
    setInput: async (name) => void (input = name),
    load: async (req) => {
      duration = 215;
      position = req.start ?? 0;
      playing = req.autoplay;
      return { duration, hasVocals: req.vocals !== null };
    },
    play: async () => void (playing = duration > 0),
    pause: async () => void (playing = false),
    seek: async (s) => void (position = Math.min(Math.max(0, s), duration)),
    setMix: async () => {},
    micTest: async (enable) => {
      if (micTimer) clearInterval(micTimer);
      micTimer = undefined;
      if (!enable) return null;
      micTimer = setInterval(() => emit({ type: "micLevel", peak: Math.random() * 0.6 }), 33);
      return input ?? inputs[0]!.name;
    },
    voice: async (settings) => {
      if (micTimer) clearInterval(micTimer);
      if (voiceTimer) clearInterval(voiceTimer);
      micTimer = undefined;
      voiceTimer = undefined;
      if (!settings.monitor) return null;
      micTimer = setInterval(() => emit({ type: "micLevel", peak: Math.random() * 0.5 }), 33);
      voiceTimer = setInterval(
        () => emit({
            type: "voice",
            bridgeMs: 25,
            targetMs: 25,
            underruns: 0,
            tuneMs: settings.tuneOn ? 12 : 0,
          }),
        500,
      );
      return { name: input ?? inputs[0]!.name, sampleRate: 48000, channels: 1 };
    },
    onEvent: async (cb) => {
      listeners.add(cb);
      return () => void listeners.delete(cb);
    },
  };
}

export function createPlayerService(): PlayerService {
  return insideTauri() ? tauriPlayer() : simulatedPlayer();
}
