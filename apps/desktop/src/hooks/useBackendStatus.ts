import { createApi } from "@karaoke/api-client";
import { useCallback, useEffect, useRef, useState } from "react";

import { getBackendConfig } from "../services/backend";
import {
  INITIAL_STATE,
  nextState,
  pollDelayMs,
  type BackendState,
  type CheckResult,
} from "../services/status";

type Api = ReturnType<typeof createApi>;

async function check(api: Api): Promise<CheckResult> {
  try {
    const { data, error } = await api.GET("/health");
    if (data) return { ok: true, health: data };
    return { ok: false, message: `Resposta inesperada: ${JSON.stringify(error)}` };
  } catch (e) {
    return { ok: false, message: e instanceof Error ? e.message : String(e) };
  }
}

/** Consulta o /health do backend até conectar e depois a cada poucos segundos. */
export function useBackendStatus(): { state: BackendState; retry: () => void } {
  const [state, setState] = useState<BackendState>(INITIAL_STATE);
  const [round, setRound] = useState(0);
  const stateRef = useRef<BackendState>(INITIAL_STATE);

  const apply = useCallback((result: CheckResult) => {
    stateRef.current = nextState(stateRef.current, result);
    setState(stateRef.current);
  }, []);

  useEffect(() => {
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    stateRef.current = INITIAL_STATE;
    setState(INITIAL_STATE);

    const loop = async (api: Api | null) => {
      let current = api;
      if (!current) {
        try {
          const cfg = await getBackendConfig();
          current = createApi(cfg.baseUrl, cfg.token);
        } catch (e) {
          apply({ ok: false, message: e instanceof Error ? e.message : String(e) });
        }
      }
      if (current) apply(await check(current));
      if (!cancelled) timer = setTimeout(() => void loop(current), pollDelayMs(stateRef.current));
    };
    void loop(null);

    return () => {
      cancelled = true;
      if (timer) clearTimeout(timer);
    };
  }, [apply, round]);

  return { state, retry: () => setRound((r) => r + 1) };
}
