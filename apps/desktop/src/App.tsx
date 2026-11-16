import { useBackendStatus } from "./hooks/useBackendStatus";
import { formatUptime, statusLabel } from "./services/status";

export default function App() {
  const { state, retry } = useBackendStatus();

  return (
    <main className="shell">
      <header>
        <h1>Karaoke Studio</h1>
        <p className="tagline">Biblioteca, separação de voz e letras sincronizadas.</p>
      </header>

      <section className="card" aria-live="polite">
        <div className="status-row">
          <span className={`dot dot-${state.kind}`} aria-hidden="true" />
          <strong>{statusLabel(state)}</strong>
        </div>

        {state.kind === "connected" && (
          <dl className="details">
            <dt>Versão do backend</dt>
            <dd>{state.health.version}</dd>
            <dt>Python</dt>
            <dd>{state.health.python}</dd>
            <dt>Ligado há</dt>
            <dd>{formatUptime(state.health.uptime_seconds)}</dd>
          </dl>
        )}

        {state.kind === "error" && (
          <>
            <p className="error">{state.message}</p>
            <button type="button" onClick={retry}>
              Tentar de novo
            </button>
          </>
        )}
      </section>
    </main>
  );
}
