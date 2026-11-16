import { useState } from "react";

import type { PlayerService } from "../../services/player";
import type { Song } from "../library/format";
import { AudioSettings } from "./AudioSettings";
import { activePreset, formatTime, PRESETS, type MixState } from "./logic";
import type { PlayerState } from "./usePlayer";

function Slider({
  label,
  value,
  onChange,
  disabled,
  hint,
}: {
  label: string;
  value: number;
  onChange: (v: number) => void;
  disabled?: boolean;
  hint?: string;
}) {
  return (
    <label className="vol" title={hint}>
      <span>{label}</span>
      <input
        type="range"
        min={0}
        max={100}
        value={value}
        disabled={disabled}
        onChange={(e) => onChange(Number(e.target.value))}
        aria-label={label}
      />
      <output>{value}%</output>
    </label>
  );
}

function QueuePanel({ player }: { player: PlayerState }) {
  const { queue } = player;
  const name = (s: Song) => `${s.title}${s.artist ? ` — ${s.artist}` : ""}`;
  return (
    <div className="queue-panel">
      <section>
        <h3>Tocando agora</h3>
        <p className="queue-now">{queue.current ? name(queue.current) : "Nada tocando."}</p>
      </section>
      <section>
        <div className="queue-head">
          <h3>Próximas ({queue.queue.length})</h3>
          {queue.queue.length > 0 && (
            <button type="button" className="ghost" onClick={player.clearQueue}>
              Limpar
            </button>
          )}
        </div>
        {queue.queue.length === 0 ? (
          <p className="hint">A fila está vazia. Use “+” nas músicas da biblioteca.</p>
        ) : (
          <ol>
            {queue.queue.map((s, i) => (
              <li key={s.id}>
                <span>{name(s)}</span>
                <button type="button" className="ghost" onClick={() => player.removeFromQueue(i)}>
                  Tirar
                </button>
              </li>
            ))}
          </ol>
        )}
      </section>
      <section>
        <h3>Histórico</h3>
        {queue.history.length === 0 ? (
          <p className="hint">Ainda não tocou nada.</p>
        ) : (
          <ul>
            {queue.history.slice(0, 10).map((s) => (
              <li key={s.id}>
                <span>{name(s)}</span>
                <button type="button" className="ghost" onClick={() => void player.play(s)}>
                  Tocar
                </button>
              </li>
            ))}
          </ul>
        )}
      </section>
    </div>
  );
}

export function PlayerBar({ player, service }: { player: PlayerState; service: PlayerService }) {
  const [dragging, setDragging] = useState<number | null>(null);
  const [showQueue, setShowQueue] = useState(false);
  const [showSettings, setShowSettings] = useState(false);
  const { queue, mix } = player;
  const song = queue.current;
  const ready = song !== null && player.duration > 0;
  const shown = dragging ?? player.position;
  const setMix = (patch: Partial<MixState>) => player.setMix({ ...mix, ...patch });
  const preset = activePreset(mix);

  const commit = () => {
    if (dragging !== null) player.seek(dragging);
    setDragging(null);
  };

  return (
    <>
      {showQueue && <QueuePanel player={player} />}
      <footer className="player" aria-label="Player">
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
            <strong>{song ? song.title : "Nada tocando"}</strong>
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
            <span className="time">{formatTime(shown)}</span>
            <input
              type="range"
              min={0}
              max={Math.max(player.duration, 0.1)}
              step={0.1}
              value={Math.min(shown, player.duration || 0)}
              disabled={!ready}
              onChange={(e) => setDragging(Number(e.target.value))}
              onPointerUp={commit}
              onKeyUp={commit}
              onBlur={commit}
              aria-label="Posição"
            />
            <span className="time">{formatTime(player.duration)}</span>
          </div>

          <div className="player-actions">
            <button type="button" className="ghost" onClick={() => setShowQueue((v) => !v)}>
              Fila ({queue.queue.length})
            </button>
            <button type="button" className="ghost" onClick={() => setShowSettings(true)}>
              Áudio
            </button>
          </div>
        </div>

        <div className="player-mix">
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
            hint={player.hasVocals ? undefined : "Música ainda não separada: não há faixa de voz"}
          />
          <Slider
            label="Instrumental"
            value={mix.instrumentalPercent}
            onChange={(v) => setMix({ instrumentalPercent: v })}
          />
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
          <div className="meters" aria-hidden="true">
            {player.peaks.map((p, i) => (
              <div className="meter small" key={i}>
                <div className="meter-fill" style={{ width: `${Math.min(1, p) * 100}%` }} />
              </div>
            ))}
          </div>
        </div>
        {song && !player.hasVocals && (
          <p className="hint player-note">
            Tocando o áudio original: separe a voz da música para usar os presets e o controle de
            voz.
          </p>
        )}
      </footer>
      {showSettings && <AudioSettings service={service} onClose={() => setShowSettings(false)} />}
    </>
  );
}
