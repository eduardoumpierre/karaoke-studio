import type { Api, components } from "@karaoke/api-client";
import { useCallback, useEffect, useRef, useState } from "react";

import { apiMessage, type Song } from "../library/format";

export type Analysis = components["schemas"]["Analysis"];
export type AnalysisEdit = components["schemas"]["AnalysisEdit"];

export interface AnalysisProgress {
  /** 0–1 */
  fraction: number;
  stage: string;
  elapsed: number;
}

export interface AnalysisState {
  analysis: Analysis | null;
  busy: boolean;
  /** Preenchido enquanto a análise roda no motor. */
  progress: AnalysisProgress | null;
  message: string | null;
  run: (force?: boolean) => Promise<void>;
  edit: (edit: AnalysisEdit) => Promise<void>;
}

/** Tom e andamento da música tocando: carrega o salvo e analisa uma vez se nunca foi feito. */
export function useAnalysis(api: Api, song: Song | null, onQueued?: () => void): AnalysisState {
  const [analysis, setAnalysis] = useState<Analysis | null>(null);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<string | null>(null);
  const [progress, setProgress] = useState<AnalysisProgress | null>(null);
  const songId = song?.id ?? null;
  const idRef = useRef<number | null>(null);
  idRef.current = songId;
  const busyRef = useRef(false);
  busyRef.current = busy;
  const queued = useRef(onQueued);
  queued.current = onQueued;

  const run = useCallback(
    async (force = false) => {
      if (songId === null) return;
      setBusy(true);
      setMessage(null);
      try {
        const { data, error } = await api.POST("/songs/{song_id}/analysis/run", {
          params: { path: { song_id: songId }, query: { force } },
        });
        if (idRef.current !== songId) return;
        if (data) {
          // A análise vai para a fila de tarefas e continua mesmo se você trocar de música.
          setAnalysis(data);
          queued.current?.();
        } else setMessage(apiMessage(error, "Não foi possível analisar a música."));
      } catch (e) {
        if (idRef.current === songId) setMessage(e instanceof Error ? e.message : String(e));
      } finally {
        if (idRef.current === songId) setBusy(false);
      }
    },
    [api, songId],
  );

  useEffect(() => {
    setAnalysis(null);
    setMessage(null);
    setBusy(false);
    if (songId === null) return;
    let cancelled = false;
    void (async () => {
      try {
        const { data } = await api.GET("/songs/{song_id}/analysis", {
          params: { path: { song_id: songId } },
        });
        if (cancelled || !data) return;
        setAnalysis(data);
        if (data.error) setMessage(data.error);
        // Já em andamento (ou falhou antes): não enfileira de novo sem o usuário pedir.
        else if (data.status === "none" && !data.running) await run();
      } catch {
        // backend indisponível: o botão "Analisar de novo" fica disponível
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [api, songId, run]);

  // Enquanto a análise roda (iniciada aqui ou já em andamento no motor), pergunta o andamento dela.
  const watching = busy || analysis?.running === true;
  useEffect(() => {
    if (songId === null || !watching) {
      setProgress(null);
      return;
    }
    let cancelled = false;
    const tick = async () => {
      try {
        const { data } = await api.GET("/songs/{song_id}/analysis", {
          params: { path: { song_id: songId } },
        });
        if (cancelled || !data || idRef.current !== songId) return;
        if (data.running) {
          setProgress({
            fraction: data.progress ?? 0,
            stage: data.stage ?? "",
            elapsed: data.elapsed_seconds ?? 0,
          });
        } else {
          setProgress(null);
          if (!busyRef.current) setAnalysis(data); // terminou (ou foi cancelada) em segundo plano
          if (data.error) setMessage(data.error);
        }
      } catch {
        // ignora: o próximo tick tenta de novo
      }
    };
    void tick();
    const timer = setInterval(() => void tick(), 500);
    return () => {
      cancelled = true;
      clearInterval(timer);
    };
  }, [api, songId, watching]);

  const edit = useCallback(
    async (change: AnalysisEdit) => {
      if (songId === null) return;
      const { data, error } = await api.PUT("/songs/{song_id}/analysis", {
        params: { path: { song_id: songId } },
        body: change,
      });
      if (idRef.current !== songId) return;
      if (data) {
        setAnalysis(data);
        setMessage(null);
      } else {
        setMessage(apiMessage(error, "Não foi possível salvar a alteração."));
      }
    },
    [api, songId],
  );

  return { analysis, busy, progress, message, run, edit };
}
