import { useState } from "react";

import { keyLabel, NOTE_NAMES, type Mode } from "../analysis/logic";
import { Meter, RangeRow, Slider } from "../player/controls";
import {
  activeVoicePreset,
  ECHO_MS_RANGE,
  EQ_DB_RANGE,
  GAIN_DB_RANGE,
  GATE_DB_RANGE,
  TUNE_STYLES,
  VOICE_PRESETS,
  activeTuneStyle,
  formatDb,
  levelAdvice,
  meterLevel,
} from "./logic";
import type { VoiceState } from "./useVoice";

/** Botão "Cantar" da barra do player, com um medidor pequeno enquanto o microfone está ligado. */
export function SingButton({ voice }: { voice: VoiceState }) {
  const { settings: s } = voice;
  return (
    <div className="sing-wrap">
      <button
        type="button"
        className={s.singing ? "sing on" : "sing"}
        aria-pressed={s.singing}
        onClick={() => voice.update({ singing: !s.singing })}
        title="Liga o microfone e toca a sua voz junto com a música"
      >
        {s.singing ? "● Cantando" : "🎤 Cantar"}
      </button>
      {s.singing && (
        <div className="voice-meter mini" title="Nível do microfone">
          <Meter peak={meterLevel(voice.peak)} />
        </div>
      )}
    </div>
  );
}

/** Gaveta "Voz": duas seções, o microfone (volume, ganho, portão) e os efeitos da voz. */
export function VoiceDrawer({ voice }: { voice: VoiceState }) {
  const [section, setSection] = useState<"mic" | "fx" | "tune">("mic");
  return (
    <div className="drawer-body voice" aria-label="Voz ao vivo">
      {voice.error && (
        <p className="error" role="alert">
          {voice.error}{" "}
          <button type="button" className="link" onClick={voice.dismissError}>
            Fechar
          </button>
        </p>
      )}
      <div className="segmented" role="tablist" aria-label="Seção da voz">
        <button
          type="button"
          role="tab"
          aria-selected={section === "mic"}
          onClick={() => setSection("mic")}
        >
          Microfone
        </button>
        <button
          type="button"
          role="tab"
          aria-selected={section === "fx"}
          onClick={() => setSection("fx")}
        >
          Efeitos
        </button>
        <button
          type="button"
          role="tab"
          aria-selected={section === "tune"}
          onClick={() => setSection("tune")}
        >
          Autotune{voice.settings.tuneOn ? " ●" : ""}
        </button>
      </div>
      {section === "mic" ? (
        <MicrophoneSection voice={voice} />
      ) : section === "fx" ? (
        <EffectsSection voice={voice} />
      ) : (
        <TuneSection voice={voice} />
      )}
    </div>
  );
}

function TuneSection({ voice }: { voice: VoiceState }) {
  const { settings: s, songKey, stats } = voice;
  const style = activeTuneStyle(s);
  const pct = (v: number) => `${v}%`;
  const scaleHint =
    s.tuneScale === "song"
      ? songKey
        ? `Notas de ${keyLabel(songKey.tonic, songKey.mode)} (o tom que está tocando).`
        : "O tom da música ainda não foi detectado: usando todas as notas. Veja a aba Tom e BPM."
      : s.tuneScale === "manual"
        ? `Notas de ${keyLabel(s.tuneTonic, s.tuneMode)}.`
        : "Corrige para a nota mais próxima, qualquer que seja.";
  return (
    <>
      <label className="check-row">
        <input
          type="checkbox"
          checked={s.tuneOn}
          onChange={(e) => voice.update({ tuneOn: e.target.checked })}
          aria-label="Ligar o autotune"
        />{" "}
        Ligar o autotune
      </label>
      <div className="presets" role="group" aria-label="Estilo do autotune">
        {TUNE_STYLES.map((t) => (
          <button
            key={t.id}
            type="button"
            className={style === t.id ? "chip active" : "chip"}
            title={t.hint}
            onClick={() =>
              voice.update({ tuneAmountPercent: t.amountPercent, tuneSpeedPercent: t.speedPercent })
            }
          >
            {t.label}
          </button>
        ))}
        {style === null && <span className="hint small">Personalizado</span>}
      </div>
      <div className="drawer-grid">
        <RangeRow
          label="Intensidade"
          value={s.tuneAmountPercent}
          min={0}
          max={100}
          step={5}
          format={pct}
          onChange={(v) => voice.update({ tuneAmountPercent: v })}
          hint="Quanto da distância até a nota certa é corrigida (100% = vai direto na nota)"
        />
        <RangeRow
          label="Velocidade"
          value={s.tuneSpeedPercent}
          min={0}
          max={100}
          step={5}
          format={(v) => (v < 35 ? `${v}% · natural` : v > 80 ? `${v}% · robô` : pct(v))}
          onChange={(v) => voice.update({ tuneSpeedPercent: v })}
          hint="Devagar mantém o jeito de cantar; rápido troca de nota na hora"
        />
        <label className="vol">
          <span>Notas</span>
          <select
            value={s.tuneScale}
            onChange={(e) => voice.update({ tuneScale: e.target.value as typeof s.tuneScale })}
            aria-label="Notas permitidas"
          >
            <option value="song">Tom da música</option>
            <option value="chromatic">Todas as notas</option>
            <option value="manual">Escolher o tom</option>
          </select>
          <output />
        </label>
        {s.tuneScale === "manual" && (
          <label className="vol">
            <span>Tom</span>
            <span className="inline-selects">
              <select
                value={s.tuneTonic}
                onChange={(e) => voice.update({ tuneTonic: Number(e.target.value) })}
                aria-label="Tônica"
              >
                {NOTE_NAMES.map((n, i) => (
                  <option key={n} value={i}>
                    {n}
                  </option>
                ))}
              </select>
              <select
                value={s.tuneMode}
                onChange={(e) => voice.update({ tuneMode: e.target.value as Mode })}
                aria-label="Modo"
              >
                <option value="major">maior</option>
                <option value="minor">menor</option>
              </select>
            </span>
            <output />
          </label>
        )}
      </div>
      <p className="hint small">
        {scaleHint}{" "}
        {s.tuneOn
          ? s.singing && stats
            ? `Soma ~${Math.round(stats.tuneMs)} ms de atraso enquanto estiver ligado.`
            : "Soma ~12 ms de atraso enquanto estiver ligado."
          : "Desligado, não soma atraso nenhum."}
      </p>
      <p className="hint small">
        O autotune acompanha o tom da música, mesmo transposta. Ele só corrige o “centro” de cada
        nota e deixa o vibrato passar. Para o efeito “robô”, use Intensidade e Velocidade no máximo.
        Funciona melhor com fones ou caixas longe do microfone.
      </p>
    </>
  );
}

function EffectsSection({ voice }: { voice: VoiceState }) {
  const { settings: s } = voice;
  const preset = activeVoicePreset(s);
  const pct = (v: number) => `${v}%`;
  const db = (v: number) => formatDb(v);
  return (
    <>
      <div className="presets" role="group" aria-label="Presets de voz">
        {VOICE_PRESETS.map((p) => (
          <button
            key={p.id}
            type="button"
            className={preset === p.id ? "chip active" : "chip"}
            title={p.hint}
            onClick={() => voice.update(p.fx)}
          >
            {p.label}
          </button>
        ))}
        {preset === null && <span className="hint small">Personalizado</span>}
      </div>

      <div className="drawer-grid">
        <RangeRow
          label="Compressão"
          value={s.compPercent}
          min={0}
          max={100}
          format={pct}
          onChange={(v) => voice.update({ compPercent: v })}
          hint="Deixa a voz mais uniforme: segura os gritos e levanta os sussurros"
        />
        <RangeRow
          label="Reverb"
          value={s.reverbPercent}
          min={0}
          max={100}
          format={pct}
          onChange={(v) => voice.update({ reverbPercent: v })}
          hint="Quanto de “sala” soa junto com a voz"
        />
        <RangeRow
          label="Tamanho da sala"
          value={s.reverbSizePercent}
          min={0}
          max={100}
          format={pct}
          disabled={s.reverbPercent === 0}
          onChange={(v) => voice.update({ reverbSizePercent: v })}
          hint="De um quarto pequeno a uma catedral"
        />
        <RangeRow
          label="Eco"
          value={s.echoPercent}
          min={0}
          max={100}
          format={pct}
          onChange={(v) => voice.update({ echoPercent: v })}
          hint="Repetições da voz, cada uma mais baixa e escura"
        />
        <RangeRow
          label="Tempo do eco"
          value={s.echoMs}
          min={ECHO_MS_RANGE.min}
          max={ECHO_MS_RANGE.max}
          step={10}
          format={(v) => `${v} ms`}
          disabled={s.echoPercent === 0}
          onChange={(v) => voice.update({ echoMs: v })}
        />
      </div>

      <details className="voice-adjust">
        <summary>Equalizador</summary>
        <div className="drawer-grid">
          <RangeRow
            label="Graves"
            value={s.eqLowDb}
            min={EQ_DB_RANGE.min}
            max={EQ_DB_RANGE.max}
            format={db}
            onChange={(v) => voice.update({ eqLowDb: v })}
            hint="Corpo da voz (abaixo de ~120 Hz)"
          />
          <RangeRow
            label="Presença"
            value={s.eqMidDb}
            min={EQ_DB_RANGE.min}
            max={EQ_DB_RANGE.max}
            format={db}
            onChange={(v) => voice.update({ eqMidDb: v })}
            hint="Clareza e inteligibilidade (~3 kHz)"
          />
          <RangeRow
            label="Brilho"
            value={s.eqHighDb}
            min={EQ_DB_RANGE.min}
            max={EQ_DB_RANGE.max}
            format={db}
            onChange={(v) => voice.update({ eqHighDb: v })}
            hint="Ar e brilho (acima de ~9 kHz)"
          />
        </div>
      </details>
      <p className="hint small">
        Os efeitos valem para a sua voz ao vivo, não para a música. Ajuste ouvindo você mesmo
        cantar.
      </p>
    </>
  );
}

function MicrophoneSection({ voice }: { voice: VoiceState }) {
  const { settings: s, mic, stats } = voice;
  const advice = s.singing ? levelAdvice(voice.peak) : null;
  return (
    <>
      <div className="drawer-grid">
        <Slider
          label="Minha voz"
          value={s.volumePercent}
          onChange={(v) => voice.update({ volumePercent: v })}
          hint="Volume da sua voz nas caixas"
        />
        <label className="vol">
          <span>Ganho do mic</span>
          <input
            type="range"
            min={GAIN_DB_RANGE.min}
            max={GAIN_DB_RANGE.max}
            step={1}
            value={s.gainDb}
            onChange={(e) => voice.update({ gainDb: Number(e.target.value) })}
            aria-label="Ganho do microfone"
          />
          <output>{formatDb(s.gainDb)}</output>
        </label>
        <label className="vol">
          <span>
            <input
              type="checkbox"
              checked={s.gateOn}
              onChange={(e) => voice.update({ gateOn: e.target.checked })}
              aria-label="Ligar o portão de ruído"
            />{" "}
            Portão
          </span>
          <input
            type="range"
            min={GATE_DB_RANGE.min}
            max={GATE_DB_RANGE.max}
            step={1}
            value={s.gateDb}
            disabled={!s.gateOn}
            onChange={(e) => voice.update({ gateDb: Number(e.target.value) })}
            aria-label="Limiar do portão de ruído"
          />
          <output>{formatDb(s.gateDb)}</output>
        </label>
        <div className="vol">
          <span>Nível do mic</span>
          <div className="voice-meter">
            <Meter peak={s.singing ? meterLevel(voice.peak) : 0} />
          </div>
        </div>
      </div>

      {s.singing ? (
        <p className="hint small">
          {mic ? `Microfone: ${mic.name}. ` : "Abrindo o microfone… "}
          {stats
            ? `Atraso do app: ~${Math.round(stats.bridgeMs)} ms, mais o do dispositivo de saída. `
            : ""}
          {stats && stats.underruns > 0 ? `(${stats.underruns} falha(s) de áudio) ` : ""}
          {advice}
        </p>
      ) : (
        <p className="hint small">Clique em “Cantar” para ligar o microfone.</p>
      )}
      <p className="hint small">
        O portão abafa o microfone quando você não está cantando. Se cortar o começo das palavras,
        deixe o limiar mais baixo. Se ouvir um apito (microfonia), diminua “Minha voz” ou afaste o
        microfone das caixas.
      </p>
    </>
  );
}
