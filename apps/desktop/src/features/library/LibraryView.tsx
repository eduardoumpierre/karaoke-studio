import type { Api } from "@karaoke/api-client";
import { useEffect, useRef, useState } from "react";

import { insideTauri, listenForDrops, pickAudioFiles } from "../../services/files";
import {
  describeJob,
  formatDuration,
  formatTechnical,
  isActive,
  type Job,
  type Song,
} from "./format";
import type { JobsState } from "./useJobs";
import { useLibrary } from "./useLibrary";
import type { PlayerState } from "../player/usePlayer";

interface RowProps {
  song: Song;
  playing: boolean;
  onPlay: () => void;
  onEnqueue: () => void;
  job: Job | undefined;
  onRemove: () => void;
  onSeparate: (force: boolean) => void;
  onCancel: (jobId: number) => void;
}

function SongRow({
  song,
  playing,
  onPlay,
  onEnqueue,
  job,
  onRemove,
  onSeparate,
  onCancel,
}: RowProps) {
  const [confirming, setConfirming] = useState(false);
  const active = job && isActive(job) ? job : undefined;
  const failed = job?.status === "failed" && song.status !== "separated" ? job : undefined;
  return (
    <li className={`song${playing ? " now-playing" : ""}`}>
      <button
        type="button"
        className="round small"
        onClick={onPlay}
        aria-label={`Tocar ${song.title}`}
        title={song.status === "separated" ? "Tocar" : "Tocar o original (ainda não separada)"}
      >
        ▶
      </button>
      <div className="song-main">
        <strong className="song-title">{song.title}</strong>
        <span className="song-artist">{song.artist || "Artista desconhecido"}</span>
      </div>
      <span className="song-tech">{formatTechnical(song)}</span>
      <span className="song-duration">{formatDuration(song.duration_seconds)}</span>
      <div className="song-actions">
        {!confirming && (
          <button
            type="button"
            className="ghost"
            onClick={onEnqueue}
            title="Adicionar à fila"
            aria-label="Adicionar à fila"
          >
            + Fila
          </button>
        )}
        {!active && !confirming && song.status === "separated" && (
          <>
            <span className="badge ok">Separada</span>
            <button type="button" className="ghost" onClick={() => onSeparate(true)}>
              Refazer
            </button>
          </>
        )}
        {!active && !confirming && song.status !== "separated" && (
          <button type="button" onClick={() => onSeparate(false)}>
            {failed ? "Tentar de novo" : "Separar voz"}
          </button>
        )}
        {active && (
          <button type="button" className="ghost" onClick={() => onCancel(active.id)}>
            Cancelar
          </button>
        )}
        {active ? null : confirming ? (
          <>
            <span>Remover?</span>
            <button type="button" className="danger" onClick={onRemove}>
              Sim
            </button>
            <button type="button" className="ghost" onClick={() => setConfirming(false)}>
              Não
            </button>
          </>
        ) : (
          <button type="button" className="ghost" onClick={() => setConfirming(true)}>
            Remover
          </button>
        )}
      </div>
      {active && (
        <div className="song-job">
          <progress max={1} value={active.status === "queued" ? undefined : active.progress} />
          <span>{describeJob(active)}</span>
        </div>
      )}
      {failed && (
        <p className="song-job error" role="alert">
          A separação falhou: {failed.error ?? "erro desconhecido"}
        </p>
      )}
    </li>
  );
}

export function LibraryView({
  api,
  player,
  jobs,
  refreshKey,
}: {
  api: Api;
  player: PlayerState;
  jobs: JobsState;
  /** Muda quando algum job termina: a lista de músicas precisa ser recarregada. */
  refreshKey: number;
}) {
  const lib = useLibrary(api);
  const reload = useRef(lib.reload);
  reload.current = lib.reload;
  useEffect(() => {
    if (refreshKey > 0) void reload.current();
  }, [refreshKey]);
  const [hovering, setHovering] = useState(false);
  const native = insideTauri();
  const { importPaths } = lib;

  useEffect(() => {
    if (!native) return;
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    void listenForDrops((paths) => void importPaths(paths), setHovering).then((fn) => {
      if (cancelled) fn();
      else unlisten = fn;
    });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [native, importPaths]);

  const choose = async () => {
    const paths = await pickAudioFiles();
    if (paths.length > 0) await importPaths(paths);
  };

  return (
    <section className={`library${hovering ? " dropping" : ""}`}>
      <div className="toolbar">
        <input
          type="search"
          placeholder="Buscar por título, artista ou álbum"
          value={lib.query}
          onChange={(e) => lib.setQuery(e.target.value)}
          aria-label="Buscar na biblioteca"
        />
        <button type="button" onClick={() => void choose()} disabled={!native || lib.importing}>
          {lib.importing ? "Importando…" : "Importar músicas"}
        </button>
      </div>

      {!native && (
        <p className="hint">
          A importação funciona na janela do aplicativo (Tauri), não no navegador.
        </p>
      )}

      {lib.notice && (
        <div className="notice" role="status">
          <div className="notice-head">
            <strong>{lib.notice.headline}</strong>
            <button type="button" className="ghost" onClick={lib.dismissNotice}>
              Fechar
            </button>
          </div>
          {lib.notice.problems.length > 0 && (
            <ul>
              {lib.notice.problems.map((p) => (
                <li key={p}>{p}</li>
              ))}
            </ul>
          )}
        </div>
      )}

      {lib.error && (
        <p className="error">
          {lib.error}{" "}
          <button type="button" className="link" onClick={() => void lib.reload()}>
            Tentar de novo
          </button>
        </p>
      )}
      {jobs.error && <p className="error">{jobs.error}</p>}

      {lib.loading ? (
        <p className="hint">Carregando…</p>
      ) : lib.songs.length === 0 ? (
        <div className="empty">
          {lib.query ? (
            <p>Nenhuma música encontrada para “{lib.query}”.</p>
          ) : (
            <p>
              A biblioteca está vazia.
              <br />
              Clique em “Importar músicas” ou arraste arquivos MP3, WAV ou FLAC para esta janela.
            </p>
          )}
        </div>
      ) : (
        <ul className="songs">
          {lib.songs.map((s) => (
            <SongRow
              key={s.id}
              song={s}
              playing={player.queue.current?.id === s.id}
              onPlay={() => void player.play(s)}
              onEnqueue={() => player.enqueue(s)}
              job={jobs.bySong[s.id]}
              onRemove={() => void lib.removeSong(s.id)}
              onSeparate={(force) => void jobs.separate(s.id, force)}
              onCancel={(id) => void jobs.cancel(id)}
            />
          ))}
        </ul>
      )}

      {hovering && <div className="drop-overlay">Solte os arquivos para importar</div>}
    </section>
  );
}
