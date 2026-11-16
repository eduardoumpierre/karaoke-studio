import type { Api } from "@karaoke/api-client";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

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
  position: number;
  duration: number;
  /** false = tocando o áudio original (música ainda não separada): o controle de voz não age. */
  hasVocals: boolean;
  mix: MixState;
  peaks: [number, number];
  error: string | null;
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
  const [position, setPosition] = useState(0);
  const [duration, setDuration] = useState(0);
  const [hasVocals, setHasVocals] = useState(true);
  const [peaks, setPeaks] = useState<[number, number]>([0, 0]);
  const [error, setError] = useState<string | null>(null);
  const queueRef = useRef(queue);
  queueRef.current = queue;

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
      setLoading(true);
      setError(null);
      try {
        const { data, error: apiError } = await api.GET("/songs/{song_id}/audio", {
          params: { path: { song_id: song.id } },
        });
        if (!data) {
          setError(`Não foi possível abrir a música: ${apiMessage(apiError, "erro desconhecido")}`);
          return false;
        }
        const info = await service.load({
          vocals: data.vocals,
          instrumental: data.instrumental ?? data.original,
          autoplay: true,
        });
        setHasVocals(info.hasVocals);
        setDuration(info.duration);
        setPosition(0);
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
    [api, service],
  );

  const play = useCallback(
    async (song: Song) => {
      if (await loadSong(song)) setQueue((q) => playNow(q, song));
    },
    [loadSong],
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
          setPosition(e.position);
          setPlaying(e.playing);
          if (e.duration > 0) setDuration(e.duration);
          setPeaks([e.peakL, e.peakR]);
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
  }, [service, play]);

  return {
    queue,
    playing,
    loading,
    position,
    duration,
    hasVocals,
    mix,
    peaks,
    error,
    play,
    enqueue: (song) => setQueue((q) => enqueue(q, song)),
    removeFromQueue: (i) => setQueue((q) => removeFromQueue(q, i)),
    clearQueue: () => setQueue((q) => clearQueue(q)),
    togglePlay: () => void (playing ? service.pause() : service.play()),
    seek: (s) => void service.seek(s),
    next,
    setMix: setMixState,
    dismissError: () => setError(null),
  };
}
