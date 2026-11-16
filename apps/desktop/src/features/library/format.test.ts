import { describe, expect, it } from "vitest";

import {
  fileName,
  filterAudioPaths,
  formatDuration,
  apiMessage,
  describeJob,
  formatTechnical,
  isActive,
  latestJobBySong,
  summarizeImport,
  type Job,
} from "./format";

describe("formatação", () => {
  it("duração em m:ss", () => {
    expect(formatDuration(0)).toBe("0:00");
    expect(formatDuration(65.4)).toBe("1:05");
    expect(formatDuration(223.9)).toBe("3:44");
    expect(formatDuration(3725)).toBe("62:05");
  });

  it("descrição técnica em português", () => {
    expect(formatTechnical({ format: "mp3", sample_rate: 44100, channels: 2 })).toBe(
      "MP3 · 44,1 kHz · estéreo",
    );
    expect(formatTechnical({ format: "wav", sample_rate: 48000, channels: 1 })).toBe(
      "WAV · 48 kHz · mono",
    );
  });

  it("nome do arquivo em caminhos do Windows e do Unix", () => {
    expect(fileName("C:\\musicas\\faixa.mp3")).toBe("faixa.mp3");
    expect(fileName("/home/x/faixa.mp3")).toBe("faixa.mp3");
  });
});

describe("filterAudioPaths", () => {
  it("aceita mp3, wav e flac sem diferenciar maiúsculas", () => {
    const r = filterAudioPaths(["a.MP3", "b.wav", "c.Flac", "d.txt", "sem_extensao"]);
    expect(r.accepted).toEqual(["a.MP3", "b.wav", "c.Flac"]);
    expect(r.ignored).toBe(2);
  });
});

describe("summarizeImport", () => {
  it("resume importadas, duplicadas e com erro", () => {
    const s = summarizeImport([
      { name: "a.mp3", status: "imported" },
      { name: "b.mp3", status: "imported" },
      { name: "c.mp3", status: "duplicate" },
      { name: "d.mp3", status: "error", message: "corrompido" },
    ]);
    expect(s.headline).toBe(
      "2 músicas importadas · 1 já estava na biblioteca · 1 arquivo com erro",
    );
    expect(s.problems).toEqual(["d.mp3: corrompido"]);
  });

  it("singular e lista vazia", () => {
    expect(summarizeImport([{ name: "a", status: "imported" }]).headline).toBe(
      "1 música importada",
    );
    expect(summarizeImport([]).headline).toBe("Nenhum arquivo selecionado.");
  });
});

const job = (over: Partial<Job>): Job => ({
  id: 1,
  song_id: 1,
  kind: "separate",
  status: "running",
  stage: "separando",
  progress: 0.456,
  error: null,
  created_at: "",
  updated_at: "",
  ...over,
});

describe("jobs", () => {
  it("descreve o andamento em português", () => {
    expect(describeJob(job({}))).toBe("Separando voz e instrumental · 46%");
    expect(describeJob(job({ stage: "normalizando", progress: 0 }))).toBe(
      "Normalizando o áudio · 0%",
    );
    expect(describeJob(job({ status: "queued", stage: "na fila" }))).toBe("Na fila");
    expect(describeJob(job({ stage: "outra" }))).toBe("Processando · 46%");
  });

  it("extrai a mensagem de erro da API", () => {
    expect(apiMessage({ code: "x", message: "Ruim" }, "padrão")).toBe("Ruim");
    expect(apiMessage({ detail: [] }, "padrão")).toBe("padrão");
    expect(apiMessage(undefined, "padrão")).toBe("padrão");
  });

  it("distingue jobs ativos", () => {
    expect(isActive(job({ status: "queued" }))).toBe(true);
    expect(isActive(job({ status: "running" }))).toBe(true);
    for (const status of ["done", "failed", "cancelled"]) {
      expect(isActive(job({ status }))).toBe(false);
    }
  });

  it("guarda o job mais recente de cada música", () => {
    const result = latestJobBySong([
      job({ id: 5, song_id: 1, status: "failed" }),
      job({ id: 3, song_id: 1, status: "done" }),
      job({ id: 4, song_id: 2 }),
    ]);
    expect(result[1]?.id).toBe(5);
    expect(result[2]?.id).toBe(4);
  });
});
