import type { components } from "@karaoke/api-client";

export type Song = components["schemas"]["Song"];

export function formatDuration(seconds: number): string {
  const total = Math.round(seconds);
  const m = Math.floor(total / 60);
  const s = total % 60;
  return `${m}:${String(s).padStart(2, "0")}`;
}

export function formatTechnical(song: Pick<Song, "format" | "sample_rate" | "channels">): string {
  const khz = (song.sample_rate / 1000).toLocaleString("pt-BR", { maximumFractionDigits: 1 });
  const channels =
    song.channels === 1 ? "mono" : song.channels === 2 ? "estéreo" : `${song.channels} canais`;
  return `${song.format.toUpperCase()} · ${khz} kHz · ${channels}`;
}

export interface ImportOutcome {
  name: string;
  status: "imported" | "duplicate" | "error";
  message?: string;
}

export interface ImportSummary {
  headline: string;
  problems: string[];
}

function plural(n: number, one: string, many: string): string {
  return `${n} ${n === 1 ? one : many}`;
}

export function summarizeImport(outcomes: ImportOutcome[]): ImportSummary {
  const imported = outcomes.filter((o) => o.status === "imported").length;
  const duplicates = outcomes.filter((o) => o.status === "duplicate").length;
  const errors = outcomes.filter((o) => o.status === "error");
  const parts: string[] = [];
  if (imported > 0) parts.push(`${plural(imported, "música importada", "músicas importadas")}`);
  if (duplicates > 0)
    parts.push(`${plural(duplicates, "já estava na biblioteca", "já estavam na biblioteca")}`);
  if (errors.length > 0)
    parts.push(`${plural(errors.length, "arquivo com erro", "arquivos com erro")}`);
  return {
    headline: parts.length > 0 ? parts.join(" · ") : "Nenhum arquivo selecionado.",
    problems: errors.map((o) => `${o.name}: ${o.message ?? "erro desconhecido"}`),
  };
}

export const AUDIO_EXTENSIONS = ["mp3", "wav", "flac"];

/** Mantém só os caminhos com extensão de áudio aceita; devolve também quantos foram ignorados. */
export function filterAudioPaths(paths: string[]): { accepted: string[]; ignored: number } {
  const accepted = paths.filter((p) =>
    AUDIO_EXTENSIONS.includes(p.split(".").pop()?.toLowerCase() ?? ""),
  );
  return { accepted, ignored: paths.length - accepted.length };
}

export function fileName(path: string): string {
  return path.split(/[\\/]/).pop() ?? path;
}

export type Job = components["schemas"]["Job"];

export function isActive(job: Pick<Job, "status">): boolean {
  return job.status === "queued" || job.status === "running";
}

/** O job de separação mais recente de cada música (a lista vem com os mais novos primeiro). */
export function latestJobBySong(jobs: Job[]): Record<number, Job> {
  const result: Record<number, Job> = {};
  for (const job of jobs) {
    if (job.kind !== "separate") continue; // análises aparecem só na lista de tarefas
    const current = result[job.song_id];
    if (!current || job.id > current.id) result[job.song_id] = job;
  }
  return result;
}

const STAGE_LABELS: Record<string, string> = {
  "na fila": "Na fila",
  iniciando: "Iniciando",
  normalizando: "Normalizando o áudio",
  "carregando modelo": "Carregando o modelo (na primeira vez baixa ~80 MB)",
  separando: "Separando voz e instrumental",
  convertendo: "Salvando em FLAC",
};

export function describeJob(job: Pick<Job, "status" | "stage" | "progress">): string {
  if (job.status === "queued") return "Na fila";
  const label = (job.stage && STAGE_LABELS[job.stage]) || "Processando";
  return `${label} · ${Math.round(job.progress * 100)}%`;
}

/** Mensagem de um corpo de erro da API (`{ code, message }`), ou o texto padrão. */
export function apiMessage(error: unknown, fallback: string): string {
  if (error && typeof error === "object" && "message" in error) {
    const { message } = error as { message: unknown };
    if (typeof message === "string") return message;
  }
  return fallback;
}

export const KIND_LABELS: Record<string, string> = {
  separate: "Separação de voz",
  analyze: "Análise de tom e BPM",
};

/** Linha de estado de uma tarefa para a lista de tarefas. */
export function describeTask(job: Job): string {
  switch (job.status) {
    case "queued":
      return "Na fila";
    case "running": {
      const stage = job.stage ? (STAGE_LABELS[job.stage] ?? job.stage) : "Processando";
      return `${stage} · ${Math.round(job.progress * 100)}%`;
    }
    case "done":
      return "Concluída";
    case "cancelled":
      return "Cancelada";
    default:
      return `Falhou: ${job.error ?? "erro desconhecido"}`;
  }
}
