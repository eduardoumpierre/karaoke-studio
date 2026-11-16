import { useEffect, useRef, useState } from "react";

import type { PlayerState } from "../player/usePlayer";
import { clampOffset, currentLineIndex, formatOffset, lineLabel, OFFSET_STEP_MS } from "./logic";
import type { LyricsState } from "./useLyrics";

export function LyricsView({ player, state }: { player: PlayerState; state: LyricsState }) {
  const song = player.queue.current;
  const { lyrics, busy, message } = state;
  const [editing, setEditing] = useState<"search" | "manual" | null>(null);
  const [title, setTitle] = useState("");
  const [artist, setArtist] = useState("");
  const [text, setText] = useState("");
  const activeRef = useRef<HTMLLIElement>(null);

  const offset = lyrics?.offset_ms ?? 0;
  const lines = lyrics?.lines ?? [];
  const current =
    lyrics?.synced && lines.length > 0 ? currentLineIndex(lines, player.position, offset) : -1;

  useEffect(() => {
    activeRef.current?.scrollIntoView({ block: "center", behavior: "smooth" });
  }, [current]);

  useEffect(() => {
    setEditing(null);
    setTitle(song?.title ?? "");
    setArtist(song?.artist ?? "");
  }, [song?.id, song?.title, song?.artist]);

  if (!song) {
    return (
      <section className="card lyrics">
        <p className="hint">Toque uma música para ver a letra.</p>
      </section>
    );
  }

  const actions = (
    <div className="lyrics-actions">
      <button
        type="button"
        className="ghost"
        onClick={() => setEditing(editing === "search" ? null : "search")}
      >
        Buscar de novo
      </button>
      <button
        type="button"
        className="ghost"
        onClick={() => setEditing(editing === "manual" ? null : "manual")}
      >
        Colar letra
      </button>
      {lyrics && lyrics.status === "found" && (
        <button type="button" className="ghost" onClick={() => void state.remove()}>
          Remover
        </button>
      )}
    </div>
  );

  return (
    <section className="card lyrics" aria-label="Letra">
      {busy && <p className="hint">Buscando a letra…</p>}
      {message && (
        <p className="error" role="alert">
          {message}
        </p>
      )}

      {lyrics?.status === "found" && lyrics.synced && (
        <>
          <ol className="lyric-lines">
            {lines.map((ln, i) => (
              <li
                key={`${ln.time_ms}-${i}`}
                ref={i === current ? activeRef : undefined}
                className={i === current ? "on" : i < current ? "past" : ""}
                aria-current={i === current ? "true" : undefined}
              >
                {lineLabel(ln.text)}
              </li>
            ))}
          </ol>
          <div className="offset-row" role="group" aria-label="Ajuste de tempo da letra">
            <button
              type="button"
              className="ghost"
              onClick={() => state.setOffset(clampOffset(offset - OFFSET_STEP_MS))}
            >
              Adiantar letra
            </button>
            <output title="Quanto a letra está deslocada">{formatOffset(offset)}</output>
            <button
              type="button"
              className="ghost"
              onClick={() => state.setOffset(clampOffset(offset + OFFSET_STEP_MS))}
            >
              Atrasar letra
            </button>
            {offset !== 0 && (
              <button type="button" className="ghost" onClick={() => state.setOffset(0)}>
                Zerar
              </button>
            )}
          </div>
        </>
      )}

      {lyrics?.status === "found" && !lyrics.synced && (
        <>
          <p className="hint">
            Esta letra não tem marcação de tempo, então não acompanha a música.
          </p>
          <pre className="plain-lyrics">{lyrics.plain}</pre>
        </>
      )}

      {lyrics?.status === "not_found" && !busy && (
        <p className="hint">
          Não encontrei a letra desta música. Tente ajustar título e artista, ou cole a letra.
        </p>
      )}

      {lyrics?.status === "found" && (
        <p className="hint small">
          {lyrics.source === "manual"
            ? "Letra colada por você."
            : `Fonte: LRCLIB${lyrics.matched_title ? ` · ${lyrics.matched_title}${lyrics.matched_artist ? ` — ${lyrics.matched_artist}` : ""}` : ""}`}
        </p>
      )}

      {actions}

      {editing === "search" && (
        <form
          className="lyrics-form"
          onSubmit={(e) => {
            e.preventDefault();
            setEditing(null);
            void state.fetchLyrics({ title: title.trim(), artist: artist.trim() });
          }}
        >
          <label>
            Título
            <input value={title} onChange={(e) => setTitle(e.target.value)} />
          </label>
          <label>
            Artista
            <input value={artist} onChange={(e) => setArtist(e.target.value)} />
          </label>
          <button type="submit" disabled={busy || title.trim() === ""}>
            Buscar
          </button>
        </form>
      )}

      {editing === "manual" && (
        <form
          className="lyrics-form"
          onSubmit={(e) => {
            e.preventDefault();
            void state.saveManual(text).then((ok) => ok && setEditing(null));
          }}
        >
          <label>
            Letra (aceita o formato LRC, com [mm:ss.xx] no início das linhas)
            <textarea rows={8} value={text} onChange={(e) => setText(e.target.value)} />
          </label>
          <button type="submit" disabled={text.trim() === ""}>
            Salvar letra
          </button>
        </form>
      )}
    </section>
  );
}
