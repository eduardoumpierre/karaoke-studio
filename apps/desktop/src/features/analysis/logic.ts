export type Mode = "major" | "minor";

/** Notação brasileira (Dó, Ré, Mi…); sustenidos para as teclas pretas. */
export const NOTE_NAMES = [
  "Dó",
  "Dó♯",
  "Ré",
  "Ré♯",
  "Mi",
  "Fá",
  "Fá♯",
  "Sol",
  "Sol♯",
  "Lá",
  "Lá♯",
  "Si",
] as const;

export const MODE_LABEL: Record<Mode, string> = { major: "maior", minor: "menor" };

export function keyLabel(tonic: number, mode: Mode): string {
  const name = NOTE_NAMES[((tonic % 12) + 12) % 12];
  return `${name} ${MODE_LABEL[mode]}`;
}

/** Tom relativo: Lá menor ↔ Dó maior. O estimador costuma confundir os dois. */
export function relativeKey(tonic: number, mode: Mode): { tonic: number; mode: Mode } {
  return mode === "major"
    ? { tonic: (tonic + 9) % 12, mode: "minor" }
    : { tonic: (tonic + 3) % 12, mode: "major" };
}

export const BPM_MIN = 30;
export const BPM_MAX = 300;

export function clampBpm(bpm: number): number {
  return Math.round(Math.max(BPM_MIN, Math.min(BPM_MAX, bpm)) * 10) / 10;
}

export function formatBpm(bpm: number): string {
  return bpm.toLocaleString("pt-BR", { minimumFractionDigits: 0, maximumFractionDigits: 1 });
}

/** Confiança do tom em palavras (o número cru de correlação diz pouco ao usuário). */
export function confidenceLabel(confidence: number | null | undefined): string {
  if (confidence == null) return "";
  if (confidence >= 0.8) return "confiança alta";
  if (confidence >= 0.6) return "confiança média";
  return "confiança baixa";
}

export const MAX_SHIFT = 6;

export function clampShift(n: number): number {
  return Math.max(-MAX_SHIFT, Math.min(MAX_SHIFT, Math.round(n)));
}

/** Tom depois de transpor (ex.: Lá menor +3 = Dó menor). */
export function transposeKey(
  tonic: number,
  mode: Mode,
  semitones: number,
): { tonic: number; mode: Mode } {
  return { tonic: (((tonic + semitones) % 12) + 12) % 12, mode };
}

export function formatShift(n: number): string {
  if (n === 0) return "Original";
  return `${n > 0 ? "+" : "−"}${Math.abs(n)}`;
}

export function formatBytes(bytes: number): string {
  if (bytes < 1024 * 1024) return `${Math.max(1, Math.round(bytes / 1024))} KB`;
  return `${(bytes / (1024 * 1024)).toLocaleString("pt-BR", { maximumFractionDigits: 0 })} MB`;
}

/** "Bater no ritmo": mediana dos intervalos entre toques recentes. Só vale com ≥ 4 toques. */
export function tapTempo(taps: number[]): number | null {
  const recent = taps.slice(-9); // até 8 intervalos
  if (recent.length < 4) return null;
  const gaps: number[] = [];
  for (let i = 1; i < recent.length; i++) gaps.push(recent[i]! - recent[i - 1]!);
  gaps.sort((a, b) => a - b);
  const mid = gaps[Math.floor(gaps.length / 2)] ?? 0;
  if (mid <= 0) return null;
  return Math.round((60000 / mid) * 10) / 10;
}

/** Texto curto para "Analisada em …" (data local do app). */
export function formatAnalyzedAt(iso: string | null | undefined): string {
  if (!iso) return "";
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return "";
  return d.toLocaleString("pt-BR", {
    day: "2-digit",
    month: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
  });
}

export function formatElapsed(seconds: number): string {
  const s = Math.max(0, Math.round(seconds));
  return s < 60 ? `${s} s` : `${Math.floor(s / 60)} min ${String(s % 60).padStart(2, "0")} s`;
}
