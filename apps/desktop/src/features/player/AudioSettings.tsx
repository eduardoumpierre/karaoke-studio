import { useCallback, useEffect, useState } from "react";

import type { DevicesReply, PlayerService } from "../../services/player";

const DEFAULT_VALUE = "";

function Meter({ peak }: { peak: number }) {
  return (
    <div className="meter" role="meter" aria-valuemin={0} aria-valuemax={1} aria-valuenow={peak}>
      <div className="meter-fill" style={{ width: `${Math.min(1, peak) * 100}%` }} />
    </div>
  );
}

export function AudioSettings({
  service,
  onClose,
}: {
  service: PlayerService;
  onClose: () => void;
}) {
  const [info, setInfo] = useState<DevicesReply | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [micOn, setMicOn] = useState(false);
  const [micName, setMicName] = useState<string | null>(null);
  const [micPeak, setMicPeak] = useState(0);

  const refresh = useCallback(async () => {
    try {
      setInfo(await service.devices());
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }, [service]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    void service
      .onEvent((e) => {
        if (e.type === "micLevel") setMicPeak((p) => Math.max(e.peak, p * 0.85));
      })
      .then((fn) => {
        if (cancelled) fn();
        else unlisten = fn;
      });
    return () => {
      cancelled = true;
      unlisten?.();
      void service.micTest(false).catch(() => {});
    };
  }, [service]);

  const changeOutput = async (value: string) => {
    setBusy(true);
    try {
      await service.setOutput(value === DEFAULT_VALUE ? null : value);
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
      await refresh();
    }
  };

  const changeInput = async (value: string) => {
    await service.setInput(value === DEFAULT_VALUE ? null : value);
    if (micOn) await toggleMic(true); // reabre o teste no novo microfone
    await refresh();
  };

  const toggleMic = async (forceOn?: boolean) => {
    const enable = forceOn ?? !micOn;
    try {
      const name = await service.micTest(enable);
      setMicOn(enable);
      setMicName(name);
      setMicPeak(0);
      setError(null);
    } catch (e) {
      setMicOn(false);
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  const close = () => {
    void service.micTest(false).catch(() => {});
    onClose();
  };

  return (
    <div className="modal-backdrop" onClick={close}>
      <div
        className="modal"
        role="dialog"
        aria-modal="true"
        aria-labelledby="audio-title"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="modal-head">
          <h2 id="audio-title">Configurações de áudio</h2>
          <button type="button" className="ghost" onClick={close}>
            Fechar
          </button>
        </div>

        {service.simulated && (
          <p className="hint">
            Modo de simulação (navegador): os dispositivos abaixo são fictícios.
          </p>
        )}
        {error && <p className="error">{error}</p>}

        {!info ? (
          <p className="hint">Procurando dispositivos…</p>
        ) : (
          <>
            <label className="field">
              <span>Saída da música (caixa de som ou fone)</span>
              <select
                disabled={busy}
                value={info.outputDevice ?? DEFAULT_VALUE}
                onChange={(e) => void changeOutput(e.target.value)}
              >
                <option value={DEFAULT_VALUE}>Padrão do sistema</option>
                {info.devices.outputs.map((d) => (
                  <option key={d.name} value={d.name}>
                    {d.name}
                    {d.isDefault ? " (padrão)" : ""}
                  </option>
                ))}
              </select>
              <small>
                {busy
                  ? "Trocando…"
                  : info.output
                    ? `Em uso: ${info.output.name} · ${info.output.sampleRate} Hz · ${info.output.channels} canais`
                    : "Nenhuma saída ativa."}
              </small>
            </label>

            <label className="field">
              <span>Entrada do microfone</span>
              <select
                value={info.inputDevice ?? DEFAULT_VALUE}
                onChange={(e) => void changeInput(e.target.value)}
              >
                <option value={DEFAULT_VALUE}>Padrão do sistema</option>
                {info.devices.inputs.map((d) => (
                  <option key={d.name} value={d.name}>
                    {d.name}
                    {d.isDefault ? " (padrão)" : ""}
                  </option>
                ))}
              </select>
              <small>
                A voz será captada por este dispositivo (usado a partir da próxima fase).
              </small>
            </label>

            <div className="field">
              <span>Testar o microfone</span>
              <div className="mic-test">
                <button type="button" onClick={() => void toggleMic()}>
                  {micOn ? "Parar teste" : "Testar"}
                </button>
                <Meter peak={micOn ? micPeak : 0} />
              </div>
              <small>
                {micOn
                  ? `Fale ou cante: a barra deve mexer. Microfone: ${micName ?? "?"}. (Nada é reproduzido.)`
                  : "Mede só o nível de entrada, sem tocar o som."}
              </small>
            </div>

            <button type="button" className="ghost" onClick={() => void refresh()}>
              Atualizar lista
            </button>
          </>
        )}
      </div>
    </div>
  );
}
