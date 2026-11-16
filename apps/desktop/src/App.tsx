import { useMemo, useState } from "react";

import type { Api } from "@karaoke/api-client";

import { LibraryView } from "./features/library/LibraryView";
import { LyricsView } from "./features/lyrics/LyricsView";
import { useLyrics } from "./features/lyrics/useLyrics";
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
  const [tab, setTab] = useState<"library" | "lyrics">("library");
  return (
    <>
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
      </nav>
      {tab === "library" ? (
        <LibraryView api={api} player={player} />
      ) : (
        <LyricsView player={player} state={lyrics} />
      )}
      <PlayerBar player={player} service={service} />
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
