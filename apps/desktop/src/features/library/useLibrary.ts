import type { Api } from "@karaoke/api-client";
import { useCallback, useEffect, useRef, useState } from "react";

import {
  fileName,
  filterAudioPaths,
  summarizeImport,
  type ImportOutcome,
  type ImportSummary,
  type Song,
} from "./format";

export interface LibraryState {
  songs: Song[];
  query: string;
  setQuery: (q: string) => void;
  loading: boolean;
  error: string | null;
  importing: boolean;
  notice: ImportSummary | null;
  dismissNotice: () => void;
  importPaths: (paths: string[]) => Promise<void>;
  removeSong: (id: number) => Promise<void>;
  /** Recarrega a lista com a busca atual. */
  reload: () => Promise<void>;
}

/** Tempo máximo esperando a lista antes de desistir e mostrar um erro. */
export const LIST_TIMEOUT_MS = 15_000;

export function useLibrary(api: Api): LibraryState {
  const [songs, setSongs] = useState<Song[]>([]);
  const [query, setQuery] = useState("");
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [importing, setImporting] = useState(false);
  const [notice, setNotice] = useState<ImportSummary | null>(null);
  const latest = useRef(0);
  const queryRef = useRef(query);
  queryRef.current = query;

  const refresh = useCallback(
    async (q: string) => {
      const ticket = ++latest.current;
      const controller = new AbortController();
      const timer = setTimeout(() => controller.abort(), LIST_TIMEOUT_MS);
      try {
        const { data, error: apiError } = await api.GET("/songs", {
          params: { query: { q } },
          signal: controller.signal,
        });
        if (ticket !== latest.current) return; // chegou uma resposta mais nova
        if (data) {
          setSongs(data);
          setError(null);
        } else {
          setError(`Não foi possível carregar a biblioteca: ${JSON.stringify(apiError)}`);
        }
      } catch (e) {
        if (ticket === latest.current) {
          setError(
            controller.signal.aborted
              ? "O motor de música demorou demais para responder."
              : `Não foi possível carregar a biblioteca: ${e instanceof Error ? e.message : String(e)}`,
          );
        }
      } finally {
        clearTimeout(timer);
        if (ticket === latest.current) setLoading(false);
      }
    },
    [api],
  );

  // Busca com pequeno atraso para não consultar a cada tecla.
  useEffect(() => {
    const timer = setTimeout(() => void refresh(query), query === "" ? 0 : 200);
    return () => clearTimeout(timer);
  }, [query, refresh]);

  const importPaths = useCallback(
    async (paths: string[]) => {
      const { accepted, ignored } = filterAudioPaths(paths);
      const outcomes: ImportOutcome[] = [];
      for (let i = 0; i < ignored; i++) {
        outcomes.push({
          name: "arquivo",
          status: "error",
          message: "formato não suportado (use MP3, WAV ou FLAC)",
        });
      }
      setImporting(true);
      try {
        for (const path of accepted) {
          const name = fileName(path);
          try {
            const {
              data,
              error: apiError,
              response,
            } = await api.POST("/songs/import", { body: { path } });
            if (data) outcomes.push({ name, status: "imported" });
            else if (response.status === 409) outcomes.push({ name, status: "duplicate" });
            else
              outcomes.push({
                name,
                status: "error",
                message: apiError?.message ?? `erro ${response.status}`,
              });
          } catch (e) {
            outcomes.push({
              name,
              status: "error",
              message: e instanceof Error ? e.message : String(e),
            });
          }
        }
      } finally {
        setImporting(false);
      }
      setNotice(summarizeImport(outcomes));
      await refresh(query);
    },
    [api, query, refresh],
  );

  const removeSong = useCallback(
    async (id: number) => {
      const { error: apiError, response } = await api.DELETE("/songs/{song_id}", {
        params: { path: { song_id: id } },
      });
      if (!response.ok && response.status !== 404) {
        setError(`Não foi possível remover: ${apiError?.message ?? response.status}`);
        return;
      }
      await refresh(query);
    },
    [api, query, refresh],
  );

  return {
    songs,
    query,
    setQuery,
    loading,
    error,
    importing,
    notice,
    dismissNotice: () => setNotice(null),
    importPaths,
    removeSong,
    reload: () => refresh(queryRef.current),
  };
}
