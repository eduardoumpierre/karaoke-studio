import { useEffect, useRef, useState } from "react";

import type { Song } from "../library/format";
import type { PlayerState } from "../player/usePlayer";
import {
  BPM_MAX,
  BPM_MIN,
  clampBpm,
  confidenceLabel,
  formatBpm,
  formatAnalyzedAt,
  formatBytes,
  formatElapsed,
  formatShift,
  keyLabel,
  NOTE_NAMES,
  relativeKey,
  tapTempo,
  transposeKey,
  type Mode,
} from "./logic";
import type { AnalysisState } from "./useAnalysis";

export function AnalysisView({
  song,
  state,
  player,
}: {
  song: Song | null;
  state: AnalysisState;
  player: PlayerState;
}) {
  const { analysis, busy, message, progress } = state;
  const [bpmText, setBpmText] = useState("");
  const [taps, setTaps] = useState<number[]>([]);
  const lastTap = useRef(0);
  const tapped = tapTempo(taps);

  const tap = () => {
    const now = performance.now();
    // pausa longa = recomeçou a contar
    setTaps((prev) => (now - lastTap.current > 2500 ? [now] : [...prev, now]));
    lastTap.current = now;
  };

  useEffect(() => {
    setBpmText(analysis?.bpm != null ? String(analysis.bpm) : "");
  }, [analysis?.bpm, song?.id]);

  if (!song) {
    return (
      <section className="card analysis">
        <p className="hint">Toque uma música para ver o andamento e o tom.</p>
      </section>
    );
  }

  const done = analysis?.status === "done";
  const tonic = analysis?.key_tonic ?? null;
  const mode = (analysis?.key_mode ?? null) as Mode | null;
  const rel = tonic !== null && mode ? relativeKey(tonic, mode) : null;

  const commitBpm = (value: number) => {
    if (!Number.isFinite(value)) return;
    void state.edit({ bpm: clampBpm(value) });
  };

  return (
    <section className="card analysis" aria-label="Tom e andamento">
      {(busy || analysis?.running) && (
        <div className="progress-box" role="status" aria-live="polite">
          <div className="progress-head">
            <strong>{progress?.stage || "Iniciando a análise…"}</strong>
            <span>
              {Math.round((progress?.fraction ?? 0) * 100)}%
              {progress ? ` · ${formatElapsed(progress.elapsed)}` : ""}
            </span>
          </div>
          <div
            className="progress"
            role="progressbar"
            aria-valuemin={0}
            aria-valuemax={100}
            aria-valuenow={Math.round((progress?.fraction ?? 0) * 100)}
          >
            <div
              className="progress-fill"
              style={{ width: `${(progress?.fraction ?? 0) * 100}%` }}
            />
          </div>
          <p className="hint small">
            A primeira análise depois de abrir o app leva mais tempo (o analisador é carregado). Ela
            continua em segundo plano se você trocar de música; acompanhe em “Tarefas”.
          </p>
        </div>
      )}
      {state.message && (
        <p className="error" role="alert">
          {message}
        </p>
      )}

      {done && (
        <div className="analysis-grid">
          <div className="metric">
            <h3>Andamento</h3>
            <div className="metric-row">
              <input
                type="number"
                min={BPM_MIN}
                max={BPM_MAX}
                step={0.5}
                value={bpmText}
                aria-label="Andamento em BPM"
                onChange={(e) => setBpmText(e.target.value)}
                onBlur={() => {
                  const v = Number(bpmText);
                  if (bpmText !== "" && v !== analysis?.bpm) commitBpm(v);
                }}
                onKeyDown={(e) => e.key === "Enter" && e.currentTarget.blur()}
              />
              <span>BPM</span>
            </div>
            <div className="metric-row">
              <button
                type="button"
                className="ghost"
                disabled={analysis?.bpm == null}
                onClick={() => analysis?.bpm != null && commitBpm(analysis.bpm / 2)}
              >
                ÷ 2
              </button>
              <button
                type="button"
                className="ghost"
                disabled={analysis?.bpm == null}
                onClick={() => analysis?.bpm != null && commitBpm(analysis.bpm * 2)}
              >
                × 2
              </button>
              <button
                type="button"
                className="ghost"
                onClick={tap}
                title="Toque no ritmo da música (4 vezes ou mais) para conferir o andamento"
              >
                Bater no ritmo
              </button>
            </div>
            {tapped !== null && (
              <p className="hint small">
                Você bateu <strong>{formatBpm(tapped)}</strong> BPM
                {analysis?.bpm != null && ` (análise: ${formatBpm(analysis.bpm)})`}.{" "}
                <button type="button" className="link" onClick={() => commitBpm(tapped)}>
                  Usar este
                </button>
              </p>
            )}
            <p className="hint small">
              {analysis?.bpm_manual
                ? "Definido por você."
                : `Sugestão automática${analysis?.bpm != null ? ` (${formatBpm(analysis.bpm)})` : ""}. Se parecer metade ou o dobro, use ÷ 2 / × 2.`}
            </p>
          </div>

          <div className="metric">
            <h3>Tom</h3>
            <div className="metric-row">
              <select
                aria-label="Nota do tom"
                value={tonic ?? 0}
                onChange={(e) => void state.edit({ key_tonic: Number(e.target.value) })}
              >
                {NOTE_NAMES.map((n, i) => (
                  <option key={n} value={i}>
                    {n}
                  </option>
                ))}
              </select>
              <select
                aria-label="Modo do tom"
                value={mode ?? "major"}
                onChange={(e) => void state.edit({ key_mode: e.target.value as Mode })}
              >
                <option value="major">maior</option>
                <option value="minor">menor</option>
              </select>
            </div>
            <p className="metric-big">{tonic !== null && mode ? keyLabel(tonic, mode) : "—"}</p>
            <p className="hint small">
              {analysis?.key_manual
                ? "Definido por você."
                : `Sugestão automática (${confidenceLabel(analysis?.key_confidence)}).`}
              {rel && ` Tom relativo: ${keyLabel(rel.tonic, rel.mode)}.`}
            </p>
            {tonic !== null && mode && player.shift !== 0 && (
              <p className="hint small">
                Tocando em{" "}
                <strong>
                  {keyLabel(
                    transposeKey(tonic, mode, player.shift).tonic,
                    transposeKey(tonic, mode, player.shift).mode,
                  )}
                </strong>{" "}
                ({formatShift(player.shift)} semitons).
              </p>
            )}
          </div>
        </div>
      )}

      {done && analysis?.analyzed_at && !busy && !analysis.running && (
        <p className="hint small">Analisada em {formatAnalyzedAt(analysis.analyzed_at)}.</p>
      )}

      <div className="lyrics-actions">
        <button
          type="button"
          className="ghost"
          disabled={busy || analysis?.running === true}
          onClick={() => void state.run(false)}
        >
          Analisar de novo
        </button>
        {player.pitchCacheBytes > 0 && (
          <button type="button" className="ghost" onClick={() => void player.clearPitchCache()}>
            Limpar tons gerados ({formatBytes(player.pitchCacheBytes)})
          </button>
        )}
        {done && (analysis?.bpm_manual || analysis?.key_manual) && (
          <button
            type="button"
            className="ghost"
            disabled={busy}
            onClick={() => void state.run(true)}
          >
            Descartar minhas edições
          </button>
        )}
      </div>
    </section>
  );
}
