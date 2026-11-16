import { useEffect, useRef, useState } from "react";

import { usePositionSelector, type PlayerState } from "../player/usePlayer";
import {
  clampOffset,
  currentLineIndex,
  formatOffset,
  LINE_LEAD_IN_S,
  lineLabel,
  lineStartSeconds,
  OFFSET_STEP_MS,
} from "./logic";
import type { LyricsState } from "./useLyrics";

export function LyricsView({ player, state }: { player: PlayerState; state: LyricsState }) {
  const song = player.queue.current;
  const { lyrics, busy, message } = state;
  const [open, setOpen] = useState(false);
  const [view, setView] = useState<"menu" | "search" | "manual">("menu");
  const settingsRef = useRef<HTMLDivElement>(null);
  const [title, setTitle] = useState("");
  const [artist, setArtist] = useState("");
  const [text, setText] = useState("");
  const activeRef = useRef<HTMLLIElement>(null);

  const offset = lyrics?.offset_ms ?? 0;
  // A posição do motor é a do que ele está entregando; você ouve isso um instante depois.
  const latency = player.outputLatencyS;
  const lines = lyrics?.lines ?? [];
  const synced = Boolean(lyrics?.synced) && lines.length > 0;
  // Só redesenha quando a linha muda, não a cada aviso de posição.
  const current = usePositionSelector(player, (pos) =>
    synced ? currentLineIndex(lines, pos - latency, offset, LINE_LEAD_IN_S) : -1,
  );

  useEffect(() => {
    activeRef.current?.scrollIntoView({ block: "center", behavior: "smooth" });
  }, [current]);

  useEffect(() => {
    setOpen(false);
    setView("menu");
    setTitle(song?.title ?? "");
    setArtist(song?.artist ?? "");
  }, [song?.id, song?.title, song?.artist]);

  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (settingsRef.current && !settingsRef.current.contains(e.target as Node)) setOpen(false);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setOpen(false);
    };
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [open]);

  const openSettings = (v: "menu" | "search" | "manual" = "menu") => {
    setView(v);
    setOpen(true);
  };

  if (!song) {
    return (
      <section className="card lyrics">
        <p className="hint">Toque uma música para ver a letra.</p>
      </section>
    );
  }

  const source =
    lyrics?.status === "found"
      ? lyrics.source === "manual"
        ? "Letra colada por você."
        : `Fonte: LRCLIB${lyrics.matched_title ? ` · ${lyrics.matched_title}${lyrics.matched_artist ? ` — ${lyrics.matched_artist}` : ""}` : ""}`
      : null;

  const settings = (
    <div className="lyrics-settings" ref={settingsRef}>
      <button
        type="button"
        className="icon-btn lyrics-gear"
        aria-label="Configurações da letra"
        aria-expanded={open}
        title="Configurações da letra"
        onClick={() => (open ? setOpen(false) : openSettings())}
      >
        ⚙
      </button>
      {open && (
        <div className="popover lyrics-popover" role="dialog" aria-label="Configurações da letra">
          {view === "menu" && (
            <>
              {lyrics?.status === "found" && lyrics.synced && (
                <div className="setting">
                  <span className="setting-label">Ajuste de tempo</span>
                  <div className="stepper" role="group" aria-label="Ajuste de tempo da letra">
                    <button
                      type="button"
                      className="icon-btn"
                      aria-label="Adiantar letra"
                      title="Adiantar letra"
                      onClick={() => state.setOffset(clampOffset(offset - OFFSET_STEP_MS))}
                    >
                      −
                    </button>
                    <output title="Quanto a letra está deslocada">{formatOffset(offset)}</output>
                    <button
                      type="button"
                      className="icon-btn"
                      aria-label="Atrasar letra"
                      title="Atrasar letra"
                      onClick={() => state.setOffset(clampOffset(offset + OFFSET_STEP_MS))}
                    >
                      +
                    </button>
                  </div>
                  {offset !== 0 && (
                    <button type="button" className="link" onClick={() => state.setOffset(0)}>
                      Zerar
                    </button>
                  )}
                </div>
              )}
              <div className="setting-list">
                <button type="button" className="menu-item" onClick={() => setView("search")}>
                  Buscar de novo
                </button>
                <button type="button" className="menu-item" onClick={() => setView("manual")}>
                  Colar letra
                </button>
                {lyrics?.status === "found" && (
                  <button
                    type="button"
                    className="menu-item danger"
                    onClick={() => {
                      setOpen(false);
                      void state.remove();
                    }}
                  >
                    Remover letra
                  </button>
                )}
              </div>
              {source && <p className="popover-note">{source}</p>}
            </>
          )}

          {view === "search" && (
            <form
              className="lyrics-form"
              onSubmit={(e) => {
                e.preventDefault();
                setOpen(false);
                void state.fetchLyrics({ title: title.trim(), artist: artist.trim() });
              }}
            >
              <button type="button" className="link back" onClick={() => setView("menu")}>
                ← Voltar
              </button>
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

          {view === "manual" && (
            <form
              className="lyrics-form"
              onSubmit={(e) => {
                e.preventDefault();
                void state.saveManual(text).then((ok) => ok && setOpen(false));
              }}
            >
              <button type="button" className="link back" onClick={() => setView("menu")}>
                ← Voltar
              </button>
              <label>
                Letra (aceita LRC, com [mm:ss.xx] no início das linhas)
                <textarea rows={8} value={text} onChange={(e) => setText(e.target.value)} />
              </label>
              <button type="submit" disabled={text.trim() === ""}>
                Salvar letra
              </button>
            </form>
          )}
        </div>
      )}
    </div>
  );

  return (
    <section className="card lyrics" aria-label="Letra">
      {settings}
      {busy && <p className="hint">Buscando a letra…</p>}
      {message && (
        <p className="error" role="alert">
          {message}
        </p>
      )}

      {lyrics?.status === "found" && lyrics.synced && (
        <ol className="lyric-lines">
          {lines.map((ln, i) => (
            <li
              key={`${ln.time_ms}-${i}`}
              ref={i === current ? activeRef : undefined}
              className={i === current ? "on" : i < current ? "past" : ""}
              aria-current={i === current ? "true" : undefined}
            >
              <button
                type="button"
                className="lyric-seek"
                title="Ir para este trecho"
                onClick={() => player.seek(lineStartSeconds(ln.time_ms, offset, latency))}
              >
                {lineLabel(ln.text)}
              </button>
            </li>
          ))}
        </ol>
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
        <div className="lyrics-empty">
          <p className="hint">Não encontrei a letra desta música.</p>
          <div className="lyrics-empty-actions">
            <button type="button" className="ghost" onClick={() => openSettings("search")}>
              Buscar de novo
            </button>
            <button type="button" className="ghost" onClick={() => openSettings("manual")}>
              Colar letra
            </button>
          </div>
        </div>
      )}
    </section>
  );
}
