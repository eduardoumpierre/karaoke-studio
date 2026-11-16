import { useCallback, useEffect, useRef, useState } from "react";

import type { MicInfo, PlayerService } from "../../services/player";
import { DEFAULT_VOICE, toEngineVoice, type SongKey, type VoiceState as Settings } from "./logic";

const STORAGE_KEY = "karaoke-studio.voice.v1";

function readStored(): Settings {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (raw) {
      // "Cantar" nunca volta ligado sozinho: o microfone só abre por um clique.
      return { ...DEFAULT_VOICE, ...(JSON.parse(raw) as Partial<Settings>), singing: false };
    }
  } catch {
    // sem armazenamento: usa os padrões
  }
  return DEFAULT_VOICE;
}

export interface VoiceStats {
  bridgeMs: number;
  underruns: number;
  /** Atraso extra do autotune (0 com ele desligado). */
  tuneMs: number;
}

export interface VoiceState {
  settings: Settings;
  /** Tom da música tocando (já transposto), usado pelo autotune. */
  songKey: SongKey | null;
  mic: MicInfo | null;
  peak: number;
  stats: VoiceStats | null;
  error: string | null;
  update: (patch: Partial<Settings>) => void;
  dismissError: () => void;
}

/** Microfone ao vivo: liga/desliga o retorno, guarda os ajustes e acompanha nível e atraso. */
export function useVoice(service: PlayerService, songKey: SongKey | null = null): VoiceState {
  const [settings, setSettings] = useState<Settings>(readStored);
  const [mic, setMic] = useState<MicInfo | null>(null);
  const [peak, setPeak] = useState(0);
  const [stats, setStats] = useState<VoiceStats | null>(null);
  const [error, setError] = useState<string | null>(null);
  const ticket = useRef(0);
  const keyTonic = songKey?.tonic ?? null;
  const keyMode = songKey?.mode ?? null;

  useEffect(() => {
    try {
      localStorage.setItem(STORAGE_KEY, JSON.stringify({ ...settings, singing: false }));
    } catch {
      // ignora
    }
  }, [settings]);

  // Qualquer mudança vai para o motor; mudanças rápidas (sliders) só aplicam a última.
  useEffect(() => {
    const mine = ++ticket.current;
    const timer = setTimeout(() => {
      void service
        .voice(
          toEngineVoice(
            settings,
            keyTonic !== null && keyMode !== null ? { tonic: keyTonic, mode: keyMode } : null,
          ),
        )
        .then((info) => {
          if (mine !== ticket.current) return;
          setMic(info);
          if (!info) {
            setPeak(0);
            setStats(null);
          }
        })
        .catch((e: unknown) => {
          if (mine !== ticket.current) return;
          setError(e instanceof Error ? e.message : String(e));
          setSettings((s) => ({ ...s, singing: false }));
        });
    }, 0);
    return () => clearTimeout(timer);
  }, [service, settings, keyTonic, keyMode]);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    void service
      .onEvent((e) => {
        if (e.type === "micLevel") setPeak((p) => Math.max(e.peak, p * 0.9));
        else if (e.type === "voice") setStats({ bridgeMs: e.bridgeMs, underruns: e.underruns, tuneMs: e.tuneMs });
        else if (e.type === "error") {
          setSettings((s) => (s.singing ? { ...s, singing: false } : s));
        }
      })
      .then((fn) => {
        if (cancelled) fn();
        else unlisten = fn;
      });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [service]);

  const update = useCallback((patch: Partial<Settings>) => {
    setError(null);
    setSettings((s) => ({ ...s, ...patch }));
  }, []);

  return {
    settings,
    songKey: keyTonic !== null && keyMode !== null ? { tonic: keyTonic, mode: keyMode } : null,
    mic,
    peak,
    stats,
    error,
    update,
    dismissError: () => setError(null),
  };
}
