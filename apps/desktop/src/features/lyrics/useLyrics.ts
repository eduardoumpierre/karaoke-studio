import type { Api, components } from "@karaoke/api-client";
import { useCallback, useEffect, useRef, useState } from "react";

import { apiMessage, type Song } from "../library/format";
import { clampOffset } from "./logic";

export type Lyrics = components["schemas"]["Lyrics"];

export interface LyricsState {
  lyrics: Lyrics | null;
  busy: boolean;
  /** Mensagem amigável (ex.: sem internet). */
  message: string | null;
  fetchLyrics: (override?: { title?: string; artist?: string }) => Promise<void>;
  saveManual: (text: string) => Promise<boolean>;
  setOffset: (ms: number) => void;
  remove: () => Promise<void>;
}

/**
 * Letra da música tocando. Ao trocar de música carrega o que já está salvo e,
 * se nunca foi buscada, tenta o LRCLIB uma vez (sem internet: avisa e não insiste).
 */
export function useLyrics(api: Api, song: Song | null): LyricsState {
  const [lyrics, setLyrics] = useState<Lyrics | null>(null);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<string | null>(null);
  const songId = song?.id ?? null;
  const idRef = useRef<number | null>(null);
  idRef.current = songId;
  const offsetTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);

  const fetchLyrics = useCallback(
    async (override?: { title?: string; artist?: string }) => {
      if (songId === null) return;
      setBusy(true);
      setMessage(null);
      try {
        const { data, error } = await api.POST("/songs/{song_id}/lyrics/fetch", {
          params: { path: { song_id: songId } },
          body: override ?? {},
        });
        if (idRef.current !== songId) return;
        if (data) setLyrics(data);
        else setMessage(apiMessage(error, "Não foi possível buscar a letra."));
      } catch (e) {
        if (idRef.current === songId) setMessage(e instanceof Error ? e.message : String(e));
      } finally {
        if (idRef.current === songId) setBusy(false);
      }
    },
    [api, songId],
  );

  useEffect(() => {
    setLyrics(null);
    setMessage(null);
    if (songId === null) return;
    let cancelled = false;
    void (async () => {
      try {
        const { data } = await api.GET("/songs/{song_id}/lyrics", {
          params: { path: { song_id: songId } },
        });
        if (cancelled || !data) return;
        setLyrics(data);
        if (data.status === "none") await fetchLyrics();
      } catch {
        // backend indisponível: a aba mostra o botão de tentar de novo
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [api, songId, fetchLyrics]);

  const saveManual = useCallback(
    async (text: string) => {
      if (songId === null) return false;
      const { data, error } = await api.PUT("/songs/{song_id}/lyrics", {
        params: { path: { song_id: songId } },
        body: { text },
      });
      if (data) {
        setLyrics(data);
        setMessage(null);
        return true;
      }
      setMessage(apiMessage(error, "Não foi possível salvar a letra."));
      return false;
    },
    [api, songId],
  );

  const setOffset = useCallback(
    (ms: number) => {
      if (songId === null) return;
      const value = clampOffset(ms);
      setLyrics((l) => (l ? { ...l, offset_ms: value } : l)); // resposta imediata na tela
      clearTimeout(offsetTimer.current);
      offsetTimer.current = setTimeout(() => {
        void api.PUT("/songs/{song_id}/lyrics/offset", {
          params: { path: { song_id: songId } },
          body: { offset_ms: value },
        });
      }, 400);
    },
    [api, songId],
  );

  useEffect(() => () => clearTimeout(offsetTimer.current), []);

  const remove = useCallback(async () => {
    if (songId === null) return;
    await api.DELETE("/songs/{song_id}/lyrics", { params: { path: { song_id: songId } } });
    setLyrics({ status: "none", synced: false, lines: [], offset_ms: 0 } as Lyrics);
  }, [api, songId]);

  return { lyrics, busy, message, fetchLyrics, saveManual, setOffset, remove };
}
