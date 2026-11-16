import createClient from "openapi-fetch";

import type { paths } from "./schema";

export type { components, paths } from "./schema";

/** Cabeçalho que o music-engine exige em todas as rotas. */
export const TOKEN_HEADER = "X-Karaoke-Token";

export function createApi(baseUrl: string, token: string) {
  return createClient<paths>({ baseUrl, headers: { [TOKEN_HEADER]: token } });
}

export type Api = ReturnType<typeof createApi>;
