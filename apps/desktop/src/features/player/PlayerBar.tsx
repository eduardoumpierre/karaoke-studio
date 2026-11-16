import { useEffect, useRef, useState } from "react";

import type { PlayerService } from "../../services/player";
import { formatDuration, type Song } from "../library/format";
import { formatShift, keyLabel, MAX_SHIFT, transposeKey, type Mode } from "../analysis/logic";
import { AudioSettings } from "./AudioSettings";
import { Slider } from "./controls";
import { SingButton, VoiceDrawer } from "../voice/VoicePanel";
import type { VoiceState } from "../voice/useVoice";
import { activePreset, PRESETS, type MixState } from "./logic";
import { SeekBar } from "./SeekBar";
import { usePeaks, type PlayerState } from "./usePlayer";

function QueueRow({
  song,
  index,
  actions,
}: {
  song: Song;
  index?: number;
  actions: React.ReactNode;
}) {
  return (
    <li className="queue-row">
      {index !== undefined && <span className="queue-index">{index + 1}</span>}
      <div className="queue-text">
        <strong>{song.title}</strong>
        <small>{song.artist || "Artista desconhecido"}</small>
      </div>
      <span className="queue-duration">{formatDuration(song.duration_seconds)}</span>
      <div className="queue-actions">{actions}</div>
    </li>
  );
}

function QueuePanel({ player }: { player: PlayerState }) {
  const { queue } = player;
  const [tab, setTab] = useState<"next" | "history">("next");
  const history = queue.history.slice(0, 10);
  return (
    <div className="queue-panel" aria-label="Fila de músicas">
      <div className="queue-now-card">
        <span className="queue-eyebrow">Tocando agora</span>
        {queue.current ? (
          <div className="queue-text">
            <strong>{queue.current.title}</strong>
            <small>{queue.current.artist || "Artista desconhecido"}</small>
          </div>
        ) : (
          <small>Nada tocando.</small>
        )}
      </div>

      <div className="queue-toolbar">
        <div className="seg" role="tablist" aria-label="Fila ou histórico">
          <button
            type="button"
            role="tab"
            aria-selected={tab === "next"}
            className={tab === "next" ? "seg-opt active" : "seg-opt"}
            onClick={() => setTab("next")}
          >
            Próximas · {queue.queue.length}
          </button>
          <button
            type="button"
            role="tab"
            aria-selected={tab === "history"}
            className={tab === "history" ? "seg-opt active" : "seg-opt"}
            onClick={() => setTab("history")}
          >
            Histórico
          </button>
        </div>
        {tab === "next" && queue.queue.length > 0 && (
          <button type="button" className="link" onClick={player.clearQueue}>
            Limpar fila
          </button>
        )}
      </div>

      {tab === "next" ? (
        queue.queue.length === 0 ? (
          <p className="queue-empty">A fila está vazia. Use “+ Fila” nas músicas da biblioteca.</p>
        ) : (
          <ol className="queue-list">
            {queue.queue.map((s, i) => (
              <QueueRow
                key={s.id}
                song={s}
                index={i}
                actions={
                  <button
                    type="button"
                    className="icon-btn"
                    aria-label={`Tirar ${s.title} da fila`}
                    title="Tirar da fila"
                    onClick={() => player.removeFromQueue(i)}
                  >
                    ✕
                  </button>
                }
              />
            ))}
          </ol>
        )
      ) : history.length === 0 ? (
        <p className="queue-empty">Ainda não tocou nada.</p>
      ) : (
        <ul className="queue-list">
          {history.map((s) => (
            <QueueRow
              key={s.id}
              song={s}
              actions={
                <button
                  type="button"
                  className="icon-btn"
                  aria-label={`Tocar ${s.title}`}
                  title="Tocar de novo"
                  onClick={() => void player.play(s)}
                >
                  ▶
                </button>
              }
            />
          ))}
        </ul>
      )}
    </div>
  );
}

function ShiftControl({
  player,
  original,
}: {
  player: PlayerState;
  original: { tonic: number; mode: Mode } | null;
}) {
  const { shift } = player;
  const playing = original ? transposeKey(original.tonic, original.mode, shift) : null;
  return (
    <div className="shift" role="group" aria-label="Tom da música">
      <span className="shift-label">Tom</span>
      <button
        type="button"
        className="ghost"
        onClick={() => player.setShift(shift - 1)}
        disabled={shift <= -MAX_SHIFT}
        aria-label="Meio tom abaixo"
        title="Meio tom abaixo"
      >
        ♭
      </button>
      <output
        className={player.shifting ? "shift-value busy" : "shift-value"}
        title={
          playing
            ? `Original: ${keyLabel(original!.tonic, original!.mode)} · Tocando em: ${keyLabel(playing.tonic, playing.mode)}`
            : "Semitons em relação ao tom original"
        }
      >
        {player.shifting ? "…" : formatShift(shift)}
      </output>
      <button
        type="button"
        className="ghost"
        onClick={() => player.setShift(shift + 1)}
        disabled={shift >= MAX_SHIFT}
        aria-label="Meio tom acima"
        title="Meio tom acima"
      >
        ♯
      </button>
      {shift !== 0 && (
        <button type="button" className="ghost" onClick={() => player.setShift(0)}>
          Original
        </button>
      )}
      {playing && <span className="shift-key">{keyLabel(playing.tonic, playing.mode)}</span>}
    </div>
  );
}

export function PlayerBar({
  player,
  service,
  voice,
  originalKey = null,
}: {
  player: PlayerState;
  service: PlayerService;
  voice: VoiceState;
  originalKey?: { tonic: number; mode: Mode } | null;
}) {
  const [showQueue, setShowQueue] = useState(false);
  const [showSettings, setShowSettings] = useState(false);
  // Uma gaveta por vez: o que não está em uso fica recolhido para a barra ficar enxuta.
  const [drawer, setDrawer] = useState<"mixer" | "voice" | null>(null);
  const toggleDrawer = (name: "mixer" | "voice") => setDrawer((d) => (d === name ? null : name));
  const { queue, mix } = player;
  const song = queue.current;
  const ready = song !== null && player.duration > 0;
  const setMix = (patch: Partial<MixState>) => player.setMix({ ...mix, ...patch });
  const preset = activePreset(mix);
  const peaks = usePeaks(player);
  const barRef = useRef<HTMLElement>(null);

  // A barra é fixa e muda de altura (voz, ajustes, avisos): reserva esse espaço no fim da página
  // para ela nunca cobrir a biblioteca.
  useEffect(() => {
    const el = barRef.current;
    if (!el) return;
    const apply = () =>
      document.documentElement.style.setProperty("--player-h", `${el.offsetHeight}px`);
    apply();
    if (typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(apply);
    observer.observe(el);
    return () => observer.disconnect();
  }, []);

  return (
    <>
      {showQueue && <QueuePanel player={player} />}
      <footer className="player" aria-label="Player" ref={barRef}>
        {player.error && (
          <div className="player-error" role="alert">
            <span>{player.error}</span>
            <button type="button" className="ghost" onClick={player.dismissError}>
              Fechar
            </button>
          </div>
        )}
        <div className="player-main">
          <div className="player-song">
            <strong title={song?.title}>{song ? song.title : "Nada tocando"}</strong>
            <span>
              {player.loading
                ? "Carregando a música…"
                : song
                  ? song.artist || "Artista desconhecido"
                  : "Escolha uma música na biblioteca (▶)"}
            </span>
          </div>

          <div className="player-transport">
            <button
              type="button"
              className="round"
              onClick={player.togglePlay}
              disabled={!ready || player.loading}
              aria-label={player.playing ? "Pausar" : "Tocar"}
            >
              {player.playing ? "❚❚" : "▶"}
            </button>
            <button
              type="button"
              className="ghost"
              onClick={player.next}
              disabled={queue.queue.length === 0}
              aria-label="Próxima da fila"
              title="Próxima da fila"
            >
              ⏭
            </button>
          </div>

          <div className="player-seek">
            <SeekBar player={player} disabled={!ready} onSeek={player.seek} />
          </div>
        </div>

        <div className="player-toolbar">
          <div className="toolbar-group">
            {song && <ShiftControl player={player} original={originalKey} />}
          </div>
          <div className="toolbar-group">
            <SingButton voice={voice} />
          </div>
          <div className="toolbar-group toolbar-right" role="group" aria-label="Painéis">
            <button
              type="button"
              className={drawer === "mixer" ? "tab-btn active" : "tab-btn"}
              aria-expanded={drawer === "mixer"}
              onClick={() => toggleDrawer("mixer")}
            >
              Mixer
            </button>
            <button
              type="button"
              className={drawer === "voice" ? "tab-btn active" : "tab-btn"}
              aria-expanded={drawer === "voice"}
              onClick={() => toggleDrawer("voice")}
            >
              Voz
            </button>
            <button
              type="button"
              className={showQueue ? "tab-btn active" : "tab-btn"}
              aria-expanded={showQueue}
              onClick={() => setShowQueue((v) => !v)}
            >
              Fila ({queue.queue.length})
            </button>
            <button type="button" className="tab-btn" onClick={() => setShowSettings(true)}>
              Áudio
            </button>
          </div>
        </div>

        {drawer === "mixer" && (
          <div className="drawer" role="region" aria-label="Mixer">
            <div className="drawer-body">
              <div className="drawer-grid">
                <Slider
                  label="Geral"
                  value={mix.masterPercent}
                  onChange={(v) => setMix({ masterPercent: v })}
                />
                <Slider
                  label="Voz original"
                  value={mix.vocalPercent}
                  onChange={(v) => setMix({ vocalPercent: v })}
                  disabled={!player.hasVocals}
                  hint={
                    player.hasVocals ? undefined : "Música ainda não separada: não há faixa de voz"
                  }
                />
                <Slider
                  label="Instrumental"
                  value={mix.instrumentalPercent}
                  onChange={(v) => setMix({ instrumentalPercent: v })}
                />
                <div className="vol" aria-hidden="true">
                  <span>Saída</span>
                  <div className="meters">
                    {peaks.map((p, i) => (
                      <div className="meter small" key={i}>
                        <div className="meter-fill" style={{ width: `${Math.min(1, p) * 100}%` }} />
                      </div>
                    ))}
                  </div>
                </div>
              </div>
              <div className="presets" role="group" aria-label="Presets de voz">
                {PRESETS.map((p) => (
                  <button
                    key={p.id}
                    type="button"
                    className={preset === p.id ? "chip active" : "chip"}
                    disabled={!player.hasVocals}
                    title={p.hint}
                    onClick={() => setMix({ vocalPercent: p.vocalPercent })}
                  >
                    {p.label} · {p.vocalPercent}%
                  </button>
                ))}
              </div>
              {song && !player.hasVocals && (
                <p className="hint small">
                  Tocando o áudio original: separe a voz da música para usar os presets e o controle
                  de voz.
                </p>
              )}
            </div>
          </div>
        )}
        {drawer === "voice" && (
          <div className="drawer" role="region" aria-label="Voz">
            <VoiceDrawer voice={voice} />
          </div>
        )}
      </footer>
      {showSettings && <AudioSettings service={service} onClose={() => setShowSettings(false)} />}
    </>
  );
}
