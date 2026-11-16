import { useCallback, useMemo, useState } from "react";

import type { Api } from "@karaoke/api-client";

import { LibraryView } from "./features/library/LibraryView";
import { useJobs } from "./features/library/useJobs";
import { TasksPanel } from "./features/tasks/TasksPanel";
import { transposeKey } from "./features/analysis/logic";
import { AnalysisView } from "./features/analysis/AnalysisView";
import { useAnalysis } from "./features/analysis/useAnalysis";
import { LyricsView } from "./features/lyrics/LyricsView";
import { useLyrics } from "./features/lyrics/useLyrics";
import { useVoice } from "./features/voice/useVoice";
import { PlayerBar } from "./features/player/PlayerBar";
import { usePlayer } from "./features/player/usePlayer";
import { useBackendStatus } from "./hooks/useBackendStatus";
import { createPlayerService } from "./services/player";
import { formatUptime, statusLabel } from "./services/status";

/** Biblioteca + player: só existe com o backend conectado. */
function Workspace({ api }: { api: Api }) {
  const service = useMemo(createPlayerService, []);
  const player = usePlayer(api, service);
  const lyrics = useLyrics(api, player.queue.current);
  // As tarefas em segundo plano vivem aqui (e não na biblioteca) para continuarem em qualquer aba.
  const [libraryVersion, setLibraryVersion] = useState(0);
  const onJobFinished = useCallback(() => setLibraryVersion((v) => v + 1), []);
  const jobs = useJobs(api, onJobFinished);
  const analysis = useAnalysis(api, player.queue.current, jobs.poke);
  const original = analysis.analysis;
  // O tom que está tocando agora (com a transposição), para o autotune cantar na escala certa.
  const songKey =
    original?.key_tonic != null && original.key_mode
      ? transposeKey(original.key_tonic, original.key_mode, player.shift)
      : null;
  const voice = useVoice(service, songKey);
  const [tab, setTab] = useState<"library" | "lyrics" | "analysis">("library");
  return (
    <>
      <div className="tabs-row">
        <nav className="tabs" role="tablist">
          <button
            type="button"
            role="tab"
            aria-selected={tab === "library"}
            onClick={() => setTab("library")}
          >
            Biblioteca
          </button>
          <button
            type="button"
            role="tab"
            aria-selected={tab === "lyrics"}
            onClick={() => setTab("lyrics")}
          >
            Letra
          </button>
          <button
            type="button"
            role="tab"
            aria-selected={tab === "analysis"}
            onClick={() => setTab("analysis")}
          >
            Tom e BPM
          </button>
        </nav>
        <TasksPanel jobs={jobs} />
      </div>
      {tab === "library" ? (
        <LibraryView api={api} player={player} jobs={jobs} refreshKey={libraryVersion} />
      ) : tab === "lyrics" ? (
        <LyricsView player={player} state={lyrics} />
      ) : (
        <AnalysisView song={player.queue.current} state={analysis} player={player} />
      )}
      <PlayerBar
        player={player}
        service={service}
        voice={voice}
        originalKey={
          analysis.analysis?.key_tonic != null && analysis.analysis.key_mode
            ? { tonic: analysis.analysis.key_tonic, mode: analysis.analysis.key_mode }
            : null
        }
      />
    </>
  );
}

export default function App() {
  const { state, retry, api } = useBackendStatus();

  return (
    <main className="shell">
      <header className="top">
        <div>
          <h1>Karaoke Studio</h1>
          <p className="tagline">Biblioteca, separação de voz e letras sincronizadas.</p>
        </div>
        <div
          className="pill"
          title={
            state.kind === "connected"
              ? `Backend ${state.health.version} · Python ${state.health.python} · ligado há ${formatUptime(state.health.uptime_seconds)}`
              : undefined
          }
        >
          <span className={`dot dot-${state.kind}`} aria-hidden="true" />
          {statusLabel(state)}
        </div>
      </header>

      {state.kind === "connected" && api ? (
        <Workspace api={api} />
      ) : (
        <section className="card" aria-live="polite">
          {state.kind === "error" ? (
            <>
              <p className="error">{state.message}</p>
              <button type="button" onClick={retry}>
                Tentar de novo
              </button>
            </>
          ) : (
            <p>Aguardando o backend iniciar…</p>
          )}
        </section>
      )}
    </main>
  );
}
