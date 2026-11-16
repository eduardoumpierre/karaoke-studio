import { invoke } from "@tauri-apps/api/core";

export interface BackendConfig {
  baseUrl: string;
  token: string;
}

function insideTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

/**
 * Descobre onde o music-engine está.
 * - Dentro do Tauri: o comando `get_backend` espera o sidecar ficar pronto e devolve porta e token.
 * - No navegador (desenvolvimento): VITE_BACKEND_URL e VITE_BACKEND_TOKEN, ver docs/development.md.
 */
export async function getBackendConfig(): Promise<BackendConfig> {
  if (insideTauri()) {
    return invoke<BackendConfig>("get_backend");
  }
  const baseUrl = import.meta.env.VITE_BACKEND_URL as string | undefined;
  const token = import.meta.env.VITE_BACKEND_TOKEN as string | undefined;
  if (!baseUrl || !token) {
    throw new Error(
      "Backend não configurado: defina VITE_BACKEND_URL e VITE_BACKEND_TOKEN (veja docs/development.md).",
    );
  }
  return { baseUrl, token };
}
