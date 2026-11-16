import { getCurrentWebview } from "@tauri-apps/api/webview";
import { open } from "@tauri-apps/plugin-dialog";

import { AUDIO_EXTENSIONS } from "../features/library/format";

export function insideTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

/** Abre o seletor nativo de arquivos. Devolve os caminhos escolhidos (vazio se cancelou). */
export async function pickAudioFiles(): Promise<string[]> {
  const result = await open({
    multiple: true,
    directory: false,
    filters: [{ name: "Áudio", extensions: AUDIO_EXTENSIONS }],
  });
  if (result === null) return [];
  return Array.isArray(result) ? result : [result];
}

/**
 * Escuta arquivos arrastados para a janela. `onHover` indica se há algo sobre a janela.
 * Devolve a função que cancela a escuta.
 */
export async function listenForDrops(
  onDrop: (paths: string[]) => void,
  onHover: (hovering: boolean) => void,
): Promise<() => void> {
  return getCurrentWebview().onDragDropEvent((event) => {
    const p = event.payload;
    if (p.type === "enter" || p.type === "over") onHover(true);
    else if (p.type === "leave") onHover(false);
    else if (p.type === "drop") {
      onHover(false);
      onDrop(p.paths);
    }
  });
}
