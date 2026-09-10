// Typen und Status-/Stage-Logik, gespiegelt an den Backend-Keys aus
// src-tauri/src/state.rs (status_name/stage_name) und den Event-Typen von
// usenet-dl. Diese Strings sind ein Protokoll — beide Seiten anpassen,
// wenn sich etwas ändert.

export type AppSettings = {
  host: string;
  port: number;
  tls: boolean;
  username: string;
  password: string;
  connections: number;
  maxConcurrentDownloads: number;
  downloadDir: string;
};

export type BackendStatus = {
  configured: boolean;
  connected: boolean;
  error: string | null;
  version: string;
};

export type JobStatus = "queued" | "downloading" | "paused" | "processing" | "complete" | "failed";
export type JobStage =
  | "queue"
  | "download"
  | "paused"
  | "processing"
  | "complete"
  | "failed"
  | "verify"
  | "repair"
  | "extract";

export type JobView = {
  id: number;
  name: string;
  status: JobStatus;
  stage: string;
  progress: number;
  speedBps: number;
  sizeBytes: number;
  downloadedBytes: number;
  error: string | null;
  destination: string;
  createdAt: number;
  completedAt: number | null;
};

export type DownloadEvent = {
  type: string;
  id?: number;
  name?: string;
  percent?: number;
  speed_bps?: number;
  error?: string;
  stage?: string;
  destination?: string;
  path?: string;
};

export const DEBOUNCE_MS = 400;
export const CLEAR_ARM_MS = 4000;

export const defaultSettings: AppSettings = {
  host: "",
  port: 563,
  tls: true,
  username: "",
  password: "",
  connections: 8,
  maxConcurrentDownloads: 2,
  downloadDir: ""
};

export const pipeline = [
  { key: "download", label: "Download" },
  { key: "verify", label: "Prüfen" },
  { key: "repair", label: "Reparieren" },
  { key: "extract", label: "Entpacken" },
  { key: "complete", label: "Fertig" }
] as const;

export type StageKey = (typeof pipeline)[number]["key"];

export function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

export function clamp(value: number): number {
  return Math.min(100, Math.max(0, Number.isFinite(value) ? value : 0));
}

export function formatBytes(bytes: number): string {
  if (!bytes) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB"];
  const unit = Math.min(Math.floor(Math.log(bytes) / Math.log(1024)), units.length - 1);
  const value = bytes / 1024 ** unit;
  return `${value.toLocaleString("de-DE", { maximumFractionDigits: unit === 0 ? 0 : 1 })} ${units[unit]}`;
}

export function stageKeyOf(job: JobView): StageKey {
  if (job.status === "complete") return "complete";
  if (["download", "verify", "repair", "extract"].includes(job.stage)) return job.stage as StageKey;
  if (job.stage === "verifying") return "verify";
  if (job.stage === "repairing") return "repair";
  if (["move", "cleanup", "directunpack", "extracting"].includes(job.stage)) return "extract";
  if (job.stage === "processing") return "verify";
  return "download";
}

export function stageIndex(job: JobView): number {
  return pipeline.findIndex((step) => step.key === stageKeyOf(job));
}

export function overallProgress(job: JobView): number {
  const progress = clamp(job.progress);
  switch (stageKeyOf(job)) {
    case "verify": return 70 + progress * 0.1;
    case "repair": return 80 + progress * 0.1;
    case "extract": return 90 + progress * 0.09;
    case "complete": return 100;
    default: return progress * 0.7;
  }
}

export function statusLabel(job: JobView): string {
  if (job.status === "failed") return "Fehlgeschlagen";
  if (job.status === "paused") return "Pausiert";
  if (job.status === "queued") return "Wartet";
  switch (stageKeyOf(job)) {
    case "verify": return "Dateien werden geprüft";
    case "repair": return "Dateien werden repariert";
    case "extract": return "Archiv wird entpackt";
    case "complete": return "Abgeschlossen";
    default: return "Wird geladen";
  }
}

export function stepState(job: JobView, index: number): "done" | "current" | "upcoming" | "failed" {
  const current = stageIndex(job);
  if (job.status === "failed" && index === current) return "failed";
  if (index < current || job.status === "complete") return "done";
  if (index === current) return "current";
  return "upcoming";
}

export function isRemovable(job: JobView): boolean {
  return job.status === "complete" || job.status === "failed";
}

/**
 * Backend meldet verarbeitungsübergreifend nur "processing"; behalte den
 * genauer angezeigten Teilschritt, bis ein spezifischeres Event eintrifft.
 */
export function mergeJobs(current: JobView[], next: JobView[]): JobView[] {
  return next.map((job) => {
    const previous = current.find((candidate) => candidate.id === job.id);
    if (previous && job.stage === "processing" && ["verify", "repair", "extract"].includes(previous.stage)) {
      return { ...job, stage: previous.stage, progress: previous.progress };
    }
    return job;
  });
}
