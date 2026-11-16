import type { Api } from "@karaoke/api-client";
import { useCallback, useEffect, useMemo, useRef, useState, useSyncExternalStore } from "react";

import { clampShift } from "../analysis/logic";
import { apiMessage, type Song } from "../library/format";
import type { PlayerService } from "../../services/player";
import {
  advance,
  clearQueue,
  DEFAULT_MIX,
  EMPTY_QUEUE,
  enqueue,
  playNow,
  removeFromQueue,
  toEngineMix,
  type MixState,
  type QueueState,
} from "./logic";
import { PositionStore, ValueStore, type Store } from "./positionStore";

const STORAGE_KEY = "karaoke-studio.player.v1";

interface Stored {
  mix: MixState;
  history: Song[];
}

function readStored(): Stored {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (raw) {
      const parsed = JSON.parse(raw) as Partial<Stored>;
      return {
        mix: { ...DEFAULT_MIX, ...parsed.mix },
        history: Array.isArray(parsed.history) ? parsed.history : [],
      };
    }
  } catch {
    // sem armazenamento: segue com os padrões
  }
  return { mix: DEFAULT_MIX, history: [] };
}

export interface PlayerState {
  queue: QueueState;
  playing: boolean;
  loading: boolean;
  /** Posição e níveis ficam fora do estado (mudam ~30×/s): use `usePosition` e `usePeaks`. */
  positionStore: Store<number>;
  peaksStore: Store<[number, number]>;
  duration: number;
  /** false = tocando o áudio original (música ainda não separada): o controle de voz não age. */
  hasVocals: boolean;
  mix: MixState;
  error: string | null;
  /** Transposição mostrada (semitons); muda na hora, o áudio acompanha após um instante. */
  shift: number;
  /** true enquanto o novo tom está sendo preparado. */
  shifting: boolean;
  /** Atraso entre o motor tocar e você ouvir (s), informado pelo dispositivo de saída. */
  outputLatencyS: number;
  /** Espaço ocupado pelos tons já gerados da música atual. */
  pitchCacheBytes: number;
  setShift: (semitones: number) => void;
  clearPitchCache: () => Promise<void>;
  play: (song: Song) => Promise<void>;
  enqueue: (song: Song) => void;
  removeFromQueue: (index: number) => void;
  clearQueue: () => void;
  togglePlay: () => void;
  seek: (seconds: number) => void;
  next: () => void;
  setMix: (mix: MixState) => void;
  dismissError: () => void;
}

export function usePlayer(api: Api, service: PlayerService): PlayerState {
  const initial = useMemo(readStored, []);
  const [queue, setQueue] = useState<QueueState>({ ...EMPTY_QUEUE, history: initial.history });
  const [mix, setMixState] = useState<MixState>(initial.mix);
  const [playing, setPlaying] = useState(false);
  const [loading, setLoading] = useState(false);
  const positionStore = useMemo(() => new PositionStore(), []);
  const peaksStore = useMemo(() => new ValueStore<[number, number]>([0, 0]), []);
  const [duration, setDuration] = useState(0);
  const [hasVocals, setHasVocals] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [shift, setShiftState] = useState(0);
  const [shifting, setShifting] = useState(false);
  const [outputLatencyS, setOutputLatencyS] = useState(0);
  const [pitchCacheBytes, setPitchCacheBytes] = useState(0);
  const queueRef = useRef(queue);
  queueRef.current = queue;
  const playingRef = useRef(false);
  playingRef.current = playing;
  const shiftTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const shiftTicket = useRef(0);
  const shiftAbort = useRef<AbortController | null>(null);

  useEffect(() => {
    try {
      localStorage.setItem(STORAGE_KEY, JSON.stringify({ mix, history: queue.history }));
    } catch {
      // ignora
    }
  }, [mix, queue.history]);

  useEffect(() => {
    void service.setMix(toEngineMix(mix)).catch(() => {});
  }, [service, mix]);

  const loadSong = useCallback(
    async (song: Song): Promise<boolean> => {
      clearTimeout(shiftTimer.current); // um tom pendente era da música anterior
      shiftTicket.current++;
      shiftAbort.current?.abort();
      setShifting(false);
      setLoading(true);
      setError(null);
      try {
        // O tom salvo da música; 0 devolve os arquivos originais.
        const saved = await api.GET("/songs/{song_id}/pitch", {
          params: { path: { song_id: song.id } },
        });
        const semitones = saved.data?.semitones ?? 0;
        const { data: result, error: apiError } = await api.PUT("/songs/{song_id}/pitch", {
          params: { path: { song_id: song.id } },
          body: { semitones },
        });
        if (!result) {
          setError(`Não foi possível abrir a música: ${apiMessage(apiError, "erro desconhecido")}`);
          return false;
        }
        const data = result.audio;
        setShiftState(semitones);
        setPitchCacheBytes(result.pitch.cache_bytes);
        const info = await service.load({
          vocals: data.vocals,
          instrumental: data.instrumental ?? data.original,
          autoplay: true,
        });
        setHasVocals(info.hasVocals);
        setDuration(info.duration);
        positionStore.reset(0);
        return true;
      } catch (e) {
        const message = e instanceof Error ? e.message : String(e);
        // um carregamento mais novo substituiu este: não é erro para o usuário
        if (!message.startsWith("cancelado")) setError(message);
        return false;
      } finally {
        setLoading(false);
      }
    },
    [api, service, positionStore],
  );

  const applyShift = useCallback(
    async (semitones: number) => {
      const song = queueRef.current.current;
      if (!song) return;
      const ticket = ++shiftTicket.current;
      // Um pedido de tom que ficou para trás não pode continuar ocupando uma conexão com o motor.
      shiftAbort.current?.abort();
      const controller = new AbortController();
      shiftAbort.current = controller;
      setShifting(true);
      setError(null);
      try {
        const { data: result, error: apiError } = await api.PUT("/songs/{song_id}/pitch", {
          params: { path: { song_id: song.id } },
          body: { semitones },
          signal: controller.signal,
        });
        if (ticket !== shiftTicket.current) return; // o usuário já pediu outro tom
        if (!result) {
          setError(`Não foi possível trocar o tom: ${apiMessage(apiError, "erro desconhecido")}`);
          return;
        }
        setPitchCacheBytes(result.pitch.cache_bytes);
        const info = await service.load({
          vocals: result.audio.vocals,
          instrumental: result.audio.instrumental ?? result.audio.original,
          autoplay: playingRef.current,
          start: positionStore.get(),
        });
        if (ticket !== shiftTicket.current) return;
        setHasVocals(info.hasVocals);
        setDuration(info.duration);
      } catch (e) {
        if (ticket !== shiftTicket.current) return; // substituído por um pedido mais novo
        const message = e instanceof Error ? e.message : String(e);
        if (!message.startsWith("cancelado")) setError(message);
      } finally {
        if (ticket === shiftTicket.current) setShifting(false);
      }
    },
    [api, service, positionStore],
  );

  const setShift = useCallback(
    (semitones: number) => {
      const value = clampShift(semitones);
      setShiftState(value);
      clearTimeout(shiftTimer.current);
      // Espera o usuário parar de clicar: cada tom novo leva alguns segundos para ser gerado.
      shiftTimer.current = setTimeout(() => void applyShift(value), 600);
    },
    [applyShift],
  );

  useEffect(() => () => clearTimeout(shiftTimer.current), []);

  const clearPitchCache = useCallback(async () => {
    const song = queueRef.current.current;
    if (!song) return;
    const { data } = await api.DELETE("/songs/{song_id}/pitch/cache", {
      params: { path: { song_id: song.id } },
    });
    if (data) setPitchCacheBytes(data.cache_bytes);
  }, [api]);

  const play = useCallback(
    async (song: Song) => {
      if (await loadSong(song)) setQueue((q) => playNow(q, song));
    },
    [loadSong],
  );

  const durationRef = useRef(0);
  durationRef.current = duration;
  const seek = useCallback(
    (seconds: number) => {
      const max = durationRef.current;
      const target = Math.max(0, max > 0 ? Math.min(seconds, max) : seconds);
      positionStore.seekTo(target); // a barra já mostra o destino; o motor confirma em seguida
      void service.seek(target);
    },
    [positionStore, service],
  );

  const next = useCallback(() => {
    const { next: song } = advance(queueRef.current);
    if (song) void play(song);
  }, [play]);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    void service
      .onEvent((e) => {
        if (e.type === "state") {
          positionStore.fromEngine(e.position);
          setPlaying(e.playing);
          if (e.duration > 0) setDuration(e.duration);
          peaksStore.set([e.peakL, e.peakR]);
        } else if (e.type === "latency") {
          // só atualiza quando muda de verdade (evita redesenhar a cada medição)
          const next = Math.round(e.outputMs) / 1000;
          setOutputLatencyS((old) => (Math.abs(old - next) >= 0.003 ? next : old));
        } else if (e.type === "ended") {
          const { next: song } = advance(queueRef.current);
          if (song) void play(song);
        } else if (e.type === "error") {
          setError(e.message);
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
  }, [service, play, positionStore, peaksStore]);

  return {
    queue,
    playing,
    loading,
    positionStore,
    peaksStore,
    duration,
    hasVocals,
    mix,
    error,
    shift,
    shifting,
    outputLatencyS,
    pitchCacheBytes,
    setShift,
    clearPitchCache,
    play,
    enqueue: (song) => setQueue((q) => enqueue(q, song)),
    removeFromQueue: (i) => setQueue((q) => removeFromQueue(q, i)),
    clearQueue: () => setQueue((q) => clearQueue(q)),
    togglePlay: () => void (playing ? service.pause() : service.play()),
    seek,
    next,
    setMix: setMixState,
    dismissError: () => setError(null),
  };
}

/** Posição de reprodução (segundos). Só o componente que chama é redesenhado a cada aviso. */
export function usePosition(player: Pick<PlayerState, "positionStore">): number {
  return useSyncExternalStore(player.positionStore.subscribe, player.positionStore.get);
}

/** Um valor derivado da posição (ex.: a linha da letra); redesenha só quando ele muda. */
export function usePositionSelector<T extends string | number | boolean>(
  player: Pick<PlayerState, "positionStore">,
  select: (position: number) => T,
): T {
  return useSyncExternalStore(player.positionStore.subscribe, () =>
    select(player.positionStore.get()),
  );
}

export function usePeaks(player: Pick<PlayerState, "peaksStore">): [number, number] {
  return useSyncExternalStore(player.peaksStore.subscribe, player.peaksStore.get);
}
