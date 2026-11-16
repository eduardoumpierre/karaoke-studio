import type { Api } from "@karaoke/api-client";
import { useCallback, useEffect, useRef, useState } from "react";

import { apiMessage, isActive, latestJobBySong, type Job } from "./format";

const POLL_MS = 1000;

export interface JobsState {
  /** O job mais recente de cada música. */
  bySong: Record<number, Job>;
  error: string | null;
  separate: (songId: number, force?: boolean) => Promise<void>;
  cancel: (jobId: number) => Promise<void>;
}

/**
 * Acompanha os jobs do backend. Consulta a cada segundo enquanto houver algum ativo e avisa
 * (`onFinished`) quando um job termina, para a biblioteca recarregar o estado das músicas.
 */
export function useJobs(api: Api, onFinished: () => void): JobsState {
  const [jobs, setJobs] = useState<Job[]>([]);
  const [error, setError] = useState<string | null>(null);
  const activeIds = useRef<Set<number>>(new Set());
  const finished = useRef(onFinished);
  finished.current = onFinished;
  const [polling, setPolling] = useState(true); // uma consulta inicial, depois só se houver ativos

  const refresh = useCallback(async () => {
    try {
      const { data } = await api.GET("/jobs", { params: { query: { active: false } } });
      if (!data) return;
      setJobs(data);
      const nowActive = new Set(data.filter(isActive).map((j) => j.id));
      const someFinished = [...activeIds.current].some((id) => !nowActive.has(id));
      activeIds.current = nowActive;
      if (someFinished) finished.current();
      setPolling(nowActive.size > 0);
    } catch {
      // backend indisponível: a tela principal já mostra o estado da conexão
    }
  }, [api]);

  useEffect(() => {
    if (!polling) return;
    void refresh();
    const timer = setInterval(() => void refresh(), POLL_MS);
    return () => clearInterval(timer);
  }, [polling, refresh]);

  const separate = useCallback(
    async (songId: number, force = false) => {
      const { data, error: apiError } = await api.POST("/songs/{song_id}/separate", {
        params: { path: { song_id: songId }, query: { force } },
      });
      if (!data) {
        setError(
          `Não foi possível iniciar a separação: ${apiMessage(apiError, "erro desconhecido")}`,
        );
        return;
      }
      setError(null);
      setPolling(true);
      await refresh();
    },
    [api, refresh],
  );

  const cancel = useCallback(
    async (jobId: number) => {
      await api.POST("/jobs/{job_id}/cancel", { params: { path: { job_id: jobId } } });
      setPolling(true);
      await refresh();
    },
    [api, refresh],
  );

  return { bySong: latestJobBySong(jobs), error, separate, cancel };
}
