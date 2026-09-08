<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { open } from "@tauri-apps/plugin-dialog";
  import { onMount, tick } from "svelte";

  type AppSettings = {
    host: string;
    port: number;
    tls: boolean;
    username: string;
    password: string;
    connections: number;
    maxConcurrentDownloads: number;
    downloadDir: string;
  };

  type BackendStatus = {
    configured: boolean;
    connected: boolean;
    error: string | null;
  };

  type JobView = {
    id: number;
    name: string;
    status: string;
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

  type DownloadEvent = {
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

  const defaultSettings: AppSettings = {
    host: "",
    port: 563,
    tls: true,
    username: "",
    password: "",
    connections: 8,
    maxConcurrentDownloads: 2,
    downloadDir: ""
  };

  const pipeline = [
    { key: "download", label: "Download" },
    { key: "verify", label: "Prüfen" },
    { key: "repair", label: "Reparieren" },
    { key: "extract", label: "Entpacken" },
    { key: "complete", label: "Fertig" }
  ] as const;

  let jobs = $state<JobView[]>([]);
  let settings = $state<AppSettings>({ ...defaultSettings });
  let backend = $state<BackendStatus>({ configured: false, connected: false, error: null });
  let loading = $state(true);
  let loadError = $state("");
  let globalError = $state("");
  let adding = $state(false);
  let sheetOpen = $state(false);
  let settingsLoading = $state(false);
  let settingsBusy = $state(false);
  let testing = $state(false);
  let testMessage = $state("");
  let settingsError = $state("");
  let busyJobs = $state<Set<number>>(new Set());
  let deleting = $state(false);
  let clearArmed = $state(false);
  let clearArmTimer: ReturnType<typeof setTimeout> | undefined;
  let removableCount = $derived(
    jobs.filter((job) => job.status === "complete" || job.status === "failed").length,
  );
  let reloadTimer: ReturnType<typeof setTimeout> | undefined;
  let settingsSheet = $state<HTMLElement | undefined>();
  let loadSequence = 0;

  function errorText(error: unknown): string {
    return error instanceof Error ? error.message : String(error);
  }

  function clamp(value: number): number {
    return Math.min(100, Math.max(0, Number.isFinite(value) ? value : 0));
  }

  function formatBytes(bytes: number): string {
    if (!bytes) return "0 B";
    const units = ["B", "KB", "MB", "GB", "TB"];
    const unit = Math.min(Math.floor(Math.log(bytes) / Math.log(1024)), units.length - 1);
    const value = bytes / 1024 ** unit;
    return `${value.toLocaleString("de-DE", { maximumFractionDigits: unit === 0 ? 0 : 1 })} ${units[unit]}`;
  }

  function stageKey(job: JobView): string {
    if (job.status === "complete") return "complete";
    if (["download", "verify", "repair", "extract"].includes(job.stage)) return job.stage;
    if (job.stage === "verifying") return "verify";
    if (job.stage === "repairing") return "repair";
    if (["move", "cleanup", "directunpack"].includes(job.stage)) return "extract";
    if (job.stage === "extracting") return "extract";
    if (job.stage === "processing") return "verify";
    return "download";
  }

  function stageIndex(job: JobView): number {
    return pipeline.findIndex((step) => step.key === stageKey(job));
  }

  function overallProgress(job: JobView): number {
    const progress = clamp(job.progress);
    switch (stageKey(job)) {
      case "verify": return 70 + progress * 0.1;
      case "repair": return 80 + progress * 0.1;
      case "extract": return 90 + progress * 0.09;
      case "complete": return 100;
      default: return progress * 0.7;
    }
  }

  function statusLabel(job: JobView): string {
    if (job.status === "failed") return "Fehlgeschlagen";
    if (job.status === "paused") return "Pausiert";
    if (job.status === "queued") return "Wartet";
    switch (stageKey(job)) {
      case "verify": return "Dateien werden geprüft";
      case "repair": return "Dateien werden repariert";
      case "extract": return "Archiv wird entpackt";
      case "complete": return "Abgeschlossen";
      default: return "Wird geladen";
    }
  }

  function stepState(job: JobView, index: number): "done" | "current" | "upcoming" | "failed" {
    const current = stageIndex(job);
    if (job.status === "failed" && index === current) return "failed";
    if (index < current || job.status === "complete") return "done";
    if (index === current) return "current";
    return "upcoming";
  }

  function mergeJobs(next: JobView[]): JobView[] {
    return next.map((job) => {
      const current = jobs.find((candidate) => candidate.id === job.id);
      if (current && job.stage === "processing" && ["verify", "repair", "extract"].includes(current.stage)) {
        return { ...job, stage: current.stage, progress: current.progress };
      }
      return job;
    });
  }

  async function loadJobs(): Promise<void> {
    const sequence = ++loadSequence;
    try {
      const next = await invoke<JobView[]>("list_jobs");
      if (sequence === loadSequence) {
        jobs = mergeJobs(next);
        loadError = "";
      }
    } catch (error) {
      if (sequence === loadSequence) loadError = `Aufträge konnten nicht geladen werden: ${errorText(error)}`;
    } finally {
      if (sequence === loadSequence) loading = false;
    }
  }

  async function loadBackend(): Promise<void> {
    try {
      backend = await invoke<BackendStatus>("backend_status");
    } catch (error) {
      backend = { configured: false, connected: false, error: errorText(error) };
    }
  }

  function scheduleReload(immediate = false): void {
    if (reloadTimer) clearTimeout(reloadTimer);
    reloadTimer = setTimeout(() => void loadJobs(), immediate ? 0 : 400);
  }

  function updateFromEvent(event: DownloadEvent): void {
    if (typeof event.id !== "number") return;
    const index = jobs.findIndex((job) => job.id === event.id);
    if (index < 0) {
      scheduleReload(true);
      return;
    }

    const job = jobs[index];
    let update: Partial<JobView> = {};
    switch (event.type) {
      case "downloading": update = { status: "downloading", stage: "download", progress: clamp(event.percent ?? job.progress), speedBps: event.speed_bps ?? job.speedBps, error: null }; break;
      case "download_complete": update = { status: "processing", stage: "verify", progress: 0, speedBps: 0 }; break;
      case "verifying": update = { status: "processing", stage: "verify", progress: 0 }; break;
      case "verify_complete": update = { status: "processing", stage: "verify", progress: 100 }; break;
      case "repairing": update = { status: "processing", stage: "repair", progress: 0 }; break;
      case "repair_complete":
      case "repair_skipped": update = { status: "processing", stage: "repair", progress: 100 }; break;
      case "extracting": update = { status: "processing", stage: "extract", progress: clamp(event.percent ?? 0) }; break;
      case "extract_complete":
      case "moving":
      case "cleaning": update = { status: "processing", stage: "extract", progress: 100, destination: event.destination ?? job.destination }; break;
      case "complete": update = { status: "complete", stage: "complete", progress: 100, speedBps: 0, destination: event.path ?? job.destination, error: null }; break;
      case "download_failed": update = { status: "failed", stage: "download", speedBps: 0, error: event.error ?? "Download fehlgeschlagen" }; break;
      case "failed": update = { status: "failed", stage: event.stage ?? job.stage, speedBps: 0, error: event.error ?? "Verarbeitung fehlgeschlagen" }; break;
      default: scheduleReload(); return;
    }
    jobs = jobs.map((candidate, candidateIndex) => candidateIndex === index ? { ...candidate, ...update } : candidate);
    scheduleReload(["complete", "failed", "download_failed"].includes(event.type));
  }

  async function addNzbs(): Promise<void> {
    globalError = "";
    adding = true;
    try {
      const selection = await open({ multiple: true, directory: false, filters: [{ name: "NZB-Dateien", extensions: ["nzb"] }] });
      const paths = Array.isArray(selection) ? selection : selection ? [selection] : [];
      if (paths.length) {
        await invoke("import_nzbs", { paths });
        await loadJobs();
      }
    } catch (error) {
      globalError = `NZB konnte nicht hinzugefügt werden: ${errorText(error)}`;
    } finally {
      adding = false;
    }
  }

  async function runJobAction(id: number, command: "pause_job" | "resume_job" | "rerun_job" | "reprocess_job"): Promise<void> {
    globalError = "";
    busyJobs = new Set(busyJobs).add(id);
    try {
      await invoke(command, { id });
      await loadJobs();
    } catch (error) {
      globalError = `Aktion fehlgeschlagen: ${errorText(error)}`;
    } finally {
      const next = new Set(busyJobs);
      next.delete(id);
      busyJobs = next;
    }
  }

  async function deleteJob(id: number): Promise<void> {
    globalError = "";
    busyJobs = new Set(busyJobs).add(id);
    try {
      await invoke("delete_job", { id });
      jobs = jobs.filter((job) => job.id !== id);
    } catch (error) {
      globalError = `Löschen fehlgeschlagen: ${errorText(error)}`;
    } finally {
      const next = new Set(busyJobs);
      next.delete(id);
      busyJobs = next;
    }
  }

  function armClearAll(): void {
    if (clearArmed) {
      void clearAllJobs();
      return;
    }
    clearArmed = true;
    if (clearArmTimer) clearTimeout(clearArmTimer);
    clearArmTimer = setTimeout(() => { clearArmed = false; }, 4000);
  }

  async function clearAllJobs(): Promise<void> {
    clearArmed = false;
    if (clearArmTimer) clearTimeout(clearArmTimer);
    globalError = "";
    deleting = true;
    try {
      const removed = await invoke<number>("clear_jobs");
      if (removed > 0) {
        jobs = jobs.filter((job) => job.status !== "complete" && job.status !== "failed");
      }
    } catch (error) {
      globalError = `Alle löschen fehlgeschlagen: ${errorText(error)}`;
    } finally {
      deleting = false;
    }
  }

  function handleKeydown(event: KeyboardEvent): void {
    if (sheetOpen && event.key === "Escape" && !settingsBusy) sheetOpen = false;
  }
  async function showSettings(): Promise<void> {
    if (settingsLoading) return;
    settingsError = "";
    testMessage = "";
    settingsLoading = true;
    sheetOpen = true;
    await tick();
    settingsSheet?.focus();
    try {
      settings = await invoke<AppSettings>("get_settings");
    } catch (error) {
      settingsError = `Einstellungen konnten nicht geladen werden: ${errorText(error)}`;
    } finally {
      settingsLoading = false;
    }
  }

  async function chooseDownloadDir(): Promise<void> {
    settingsError = "";
    try {
      const directory = await open({ directory: true, multiple: false, defaultPath: settings.downloadDir || undefined });
      if (typeof directory === "string") settings.downloadDir = directory;
    } catch (error) {
      settingsError = `Ordner konnte nicht ausgewählt werden: ${errorText(error)}`;
    }
  }

  async function testConnection(): Promise<void> {
    settingsError = "";
    testMessage = "";
    testing = true;
    try {
      await invoke("test_server", { settings });
      testMessage = "Verbindung erfolgreich hergestellt.";
    } catch (error) {
      settingsError = `Verbindung fehlgeschlagen: ${errorText(error)}`;
    } finally {
      testing = false;
    }
  }

  async function saveSettings(event: SubmitEvent): Promise<void> {
    event.preventDefault();
    settingsError = "";
    testMessage = "";
    settingsBusy = true;
    try {
      await invoke("save_settings", { settings });
      await loadBackend();
      sheetOpen = false;
    } catch (error) {
      settingsError = `Einstellungen konnten nicht gespeichert werden: ${errorText(error)}`;
      await loadBackend();
    } finally {
      settingsBusy = false;
    }
  }

  onMount(() => {
    let disposed = false;
    const unlisteners: UnlistenFn[] = [];
    void Promise.all([loadJobs(), loadBackend()]);
    void Promise.all([
      listen<DownloadEvent>("download-event", ({ payload }) => updateFromEvent(payload)),
      listen<number[]>("nzb-imported", () => scheduleReload(true)),
      listen<string>("nzb-import-error", ({ payload }) => { globalError = `Import fehlgeschlagen: ${payload}`; })
    ]).then((listeners) => {
      if (disposed) listeners.forEach((unlisten) => unlisten());
      else unlisteners.push(...listeners);
    }).catch((error) => { globalError = `Live-Aktualisierung nicht verfügbar: ${errorText(error)}`; });

    return () => {
      disposed = true;
      unlisteners.forEach((unlisten) => unlisten());
      if (reloadTimer) clearTimeout(reloadTimer);
    };
  });
</script>

<svelte:head>
  <title>NZB Deck</title>
  <meta name="description" content="NZB-Aufträge laden und verarbeiten" />
</svelte:head>
<svelte:window onkeydown={handleKeydown} />

<div class="app-shell">
  <header class="toolbar">
    <div class="brand-block">
      <h1>NZB Deck</h1>
      <button class:connected={backend.connected} class="connection-chip" type="button" onclick={showSettings} disabled={settingsLoading} aria-label="Verbindungseinstellungen öffnen">
        <span class="status-dot" aria-hidden="true"></span>
        {backend.connected ? "Verbunden" : backend.configured ? "Getrennt" : "Nicht eingerichtet"}
      </button>
    </div>
    <nav aria-label="Hauptaktionen">
      <button class="button primary" type="button" onclick={addNzbs} disabled={adding}>
        <span aria-hidden="true">＋</span>{adding ? "Wird geöffnet …" : "NZB hinzufügen"}
      </button>
      {#if removableCount > 0}
        <button class="button quiet danger" type="button" onclick={armClearAll} disabled={deleting}>
          {clearArmed ? "Wirklich alle löschen?" : `Alle löschen (${removableCount})`}
        </button>
      {/if}
      <button class="button icon-button" type="button" onclick={showSettings} disabled={settingsLoading} aria-label="Einstellungen öffnen" title="Einstellungen">
        <span aria-hidden="true">⚙︎</span>
      </button>
    </nav>
  </header>

  <main>
    {#if globalError || loadError || backend.error}
      <div class="alert" role="alert">
        <strong>Es ist ein Problem aufgetreten.</strong>
        <div class="alert-messages">
          {#if globalError}<span>{globalError}</span>{/if}
          {#if loadError}<span>{loadError}</span>{/if}
          {#if backend.error}<span>Newsserver: {backend.error}</span>{/if}
        </div>
        {#if loadError}<button type="button" onclick={() => void loadJobs()}>Erneut laden</button>{/if}
      </div>
    {/if}

    {#if loading}
      <section class="empty-state" aria-live="polite">
        <div class="spinner" aria-hidden="true"></div>
        <h2>Aufträge werden geladen</h2>
        <p>NZB Deck liest die aktuelle Warteschlange ein.</p>
      </section>
    {:else if jobs.length === 0}
      <section class="empty-state">
        <div class="empty-icon" aria-hidden="true">↓</div>
        <h2>Bereit für den ersten Download</h2>
        <p>Füge eine oder mehrere NZB-Dateien hinzu. Download, Prüfung, Reparatur und Entpacken laufen anschließend automatisch.</p>
        <button class="button primary" type="button" onclick={addNzbs} disabled={adding}>NZB-Dateien auswählen</button>
      </section>
    {:else}
      <section class="jobs" aria-label="NZB-Aufträge" aria-live="polite">
        {#each jobs as job (job.id)}
          <article class:failed={job.status === "failed"} class:complete={job.status === "complete"} class="job-card">
            <header class="job-header">
              <div class="job-title">
                <h2 title={job.name}>{job.name}</h2>
                <span class="job-status">{statusLabel(job)}</span>
              </div>
              <strong class="percentage">{Math.round(overallProgress(job))}<span>%</span></strong>
            </header>

            <div class="overall-track" role="progressbar" aria-label={`Gesamtfortschritt von ${job.name}`} aria-valuemin="0" aria-valuemax="100" aria-valuenow={Math.round(overallProgress(job))}>
              <span style={`width: ${overallProgress(job)}%`}></span>
            </div>

            <ol class="pipeline" aria-label="Verarbeitungsablauf">
              {#each pipeline as step, index}
                <li class={stepState(job, index)} aria-current={stepState(job, index) === "current" ? "step" : undefined}>
                  <span class="step-marker" aria-hidden="true">{stepState(job, index) === "done" ? "✓" : index + 1}</span>
                  <span>{step.label}</span>
                </li>
              {/each}
            </ol>

            <footer class="job-footer">
              <dl class="metrics">
                {#if job.status === "downloading"}
                  <div><dt>Tempo</dt><dd>{formatBytes(job.speedBps)}/s</dd></div>
                {/if}
                <div><dt>Daten</dt><dd>{formatBytes(job.downloadedBytes)}{job.sizeBytes ? ` von ${formatBytes(job.sizeBytes)}` : ""}</dd></div>
                {#if job.destination}
                  <div class="destination"><dt>Ziel</dt><dd title={job.destination}>{job.destination}</dd></div>
                {/if}
              </dl>

              {#if job.error}
                <p class="job-error" role="alert"><strong>Fehler:</strong> {job.error}</p>
              {/if}

              <div class="job-actions" aria-label={`Aktionen für ${job.name}`}>
                {#if job.status === "downloading" || job.status === "queued"}
                  <button class="button secondary" type="button" disabled={busyJobs.has(job.id)} onclick={() => runJobAction(job.id, "pause_job")}>Ⅱ&nbsp; Pause</button>
                {:else if job.status === "paused"}
                  <button class="button primary small" type="button" disabled={busyJobs.has(job.id)} onclick={() => runJobAction(job.id, "resume_job")}>▶&nbsp; Fortsetzen</button>
                {:else if job.status === "failed"}
                  <button class="button primary small" type="button" disabled={busyJobs.has(job.id)} onclick={() => runJobAction(job.id, "rerun_job")}>↻&nbsp; Neu herunterladen</button>
                  <button class="button secondary" type="button" disabled={busyJobs.has(job.id)} onclick={() => runJobAction(job.id, "reprocess_job")}>Erneut verarbeiten</button>
                  <button class="button quiet danger" type="button" disabled={busyJobs.has(job.id)} onclick={() => deleteJob(job.id)}>Löschen</button>
                {:else if job.status === "complete"}
                  <button class="button primary small" type="button" disabled={busyJobs.has(job.id)} onclick={() => runJobAction(job.id, "rerun_job")}>↻&nbsp; Neu herunterladen</button>
                  <button class="button secondary" type="button" disabled={busyJobs.has(job.id)} onclick={() => runJobAction(job.id, "reprocess_job")}>Erneut verarbeiten</button>
                  <button class="button quiet danger" type="button" disabled={busyJobs.has(job.id)} onclick={() => deleteJob(job.id)}>Löschen</button>
                {/if}
              </div>
            </footer>
          </article>
        {/each}
      </section>
    {/if}
  </main>
</div>

{#if sheetOpen}
  <div class="sheet-backdrop">
    <div class="settings-sheet" role="dialog" aria-modal="true" aria-labelledby="settings-title" tabindex="-1" bind:this={settingsSheet}>
      <header class="sheet-header">
        <div><p class="eyebrow">Konfiguration</p><h2 id="settings-title">Einstellungen</h2></div>
        <button class="close-button" type="button" onclick={() => { if (!settingsBusy) sheetOpen = false; }} aria-label="Einstellungen schließen">×</button>
      </header>

      <form onsubmit={saveSettings}>
        <fieldset disabled={settingsLoading || settingsBusy || testing}>
          <legend>Newsserver</legend>
          <div class="form-grid server-grid">
            <label class="host-field"><span>Server</span><input required autocomplete="url" placeholder="news.example.net" bind:value={settings.host} /></label>
            <label><span>Port</span><input required type="number" min="1" max="65535" inputmode="numeric" bind:value={settings.port} /></label>
          </div>
          <label class="toggle-row"><span><strong>Verschlüsselte Verbindung</strong><small>TLS für die Verbindung zum Newsserver verwenden</small></span><input type="checkbox" bind:checked={settings.tls} /></label>
          <div class="form-grid">
            <label><span>Benutzername</span><input autocomplete="username" bind:value={settings.username} /></label>
            <label><span>Passwort</span><input type="password" autocomplete="current-password" bind:value={settings.password} /></label>
            <label><span>Verbindungen</span><input required type="number" min="1" inputmode="numeric" bind:value={settings.connections} /></label>
            <label><span>Gleichzeitige Downloads</span><input required type="number" min="1" inputmode="numeric" bind:value={settings.maxConcurrentDownloads} /></label>
          </div>
        </fieldset>

        <fieldset disabled={settingsLoading || settingsBusy || testing}>
          <legend>Speicherort</legend>
          <label><span>Zielordner</span><div class="path-control"><input required readonly bind:value={settings.downloadDir} /><button class="button secondary" type="button" onclick={chooseDownloadDir}>Auswählen …</button></div></label>
        </fieldset>

        {#if settingsError}<p class="form-message error" role="alert">{settingsError}</p>{/if}
        {#if testMessage}<p class="form-message success" role="status">{testMessage}</p>{/if}

        <footer class="sheet-actions">
          <button class="button secondary" type="button" onclick={testConnection} disabled={settingsLoading || testing || settingsBusy}>{testing ? "Wird getestet …" : "Verbindung testen"}</button>
          <div>
            <button class="button quiet" type="button" onclick={() => sheetOpen = false} disabled={settingsBusy}>Abbrechen</button>
            <button class="button primary" type="submit" disabled={settingsLoading || settingsBusy || testing}>{settingsBusy ? "Wird gespeichert …" : "Speichern"}</button>
          </div>
        </footer>
      </form>
    </div>
  </div>
{/if}

<style>
  :global(*) { box-sizing: border-box; }
  :global(:root) {
    font-family: ui-rounded, "SF Pro Rounded", "Avenir Next", sans-serif;
    color: var(--text);
    background: var(--canvas);
    font-synthesis: none;
    --canvas: hsl(215 24% 96%);
    --surface: hsl(210 25% 99%);
    --surface-raised: hsl(210 30% 100%);
    --surface-muted: hsl(214 20% 93%);
    --text: hsl(215 28% 17%);
    --text-muted: hsl(215 12% 42%);
    --line: hsl(214 18% 85%);
    --line-strong: hsl(214 16% 74%);
    --accent: hsl(211 92% 50%);
    --accent-hover: hsl(211 92% 44%);
    --accent-soft: hsl(211 92% 94%);
    --success: hsl(149 62% 35%);
    --success-soft: hsl(149 48% 92%);
    --danger: hsl(3 72% 50%);
    --danger-soft: hsl(3 72% 95%);
    --shadow-card: 0 1px 2px hsl(215 30% 20% / 0.06), 0 10px 30px hsl(215 30% 20% / 0.07);
    --shadow-sheet: 0 -12px 48px hsl(215 30% 15% / 0.2);
    --radius-sm: 0.5rem;
    --radius-md: 0.75rem;
    --radius-lg: 1rem;
    --radius-pill: 999rem;
    --space-1: 0.25rem;
    --space-2: 0.5rem;
    --space-3: 0.75rem;
    --space-4: 1rem;
    --space-5: 1.25rem;
    --space-6: 1.5rem;
    --space-8: 2rem;
    --space-10: 2.5rem;
    --control: 2.25rem;
    --content: 68rem;
    --transition: 160ms ease-out;
  }
  :global(body) { margin: 0; min-width: 20rem; min-height: 100vh; background: var(--canvas); }
  :global(button), :global(input) { font: inherit; }
  :global(button) { -webkit-tap-highlight-color: transparent; }
  :global(button:focus-visible), :global(input:focus-visible) { outline: 3px solid color-mix(in srgb, var(--accent), transparent 62%); outline-offset: 2px; }

  .app-shell { min-height: 100vh; padding-bottom: calc(var(--space-8) + env(safe-area-inset-bottom)); }
  .toolbar { position: sticky; top: 0; z-index: 10; display: flex; align-items: center; justify-content: space-between; gap: var(--space-4); min-height: 4rem; padding: calc(var(--space-2) + env(safe-area-inset-top)) max(var(--space-5), env(safe-area-inset-right)) var(--space-2) max(var(--space-5), env(safe-area-inset-left)); border-bottom: 1px solid color-mix(in srgb, var(--line), transparent 22%); background: color-mix(in srgb, var(--canvas), transparent 8%); backdrop-filter: saturate(160%) blur(18px); }
  .brand-block { display: flex; align-items: center; gap: var(--space-3); min-width: 0; }
  h1 { margin: 0; font-size: 1.15rem; letter-spacing: -0.025em; white-space: nowrap; }
  nav { display: flex; align-items: center; gap: var(--space-2); }
  .connection-chip { display: inline-flex; align-items: center; gap: var(--space-2); min-height: 1.75rem; padding: var(--space-1) var(--space-3); border: 1px solid var(--line); border-radius: var(--radius-pill); color: var(--text-muted); background: var(--surface); font-size: 0.75rem; font-weight: 650; cursor: pointer; transition: border-color var(--transition), background var(--transition); }
  .connection-chip:hover:not(:disabled) { border-color: var(--line-strong); background: var(--surface-raised); }
  .connection-chip:disabled { opacity: 0.55; cursor: wait; }
  .status-dot { width: 0.5rem; height: 0.5rem; border-radius: 50%; background: var(--danger); box-shadow: 0 0 0 3px var(--danger-soft); }
  .connection-chip.connected .status-dot { background: var(--success); box-shadow: 0 0 0 3px var(--success-soft); }

  main { width: min(calc(100% - var(--space-8)), var(--content)); margin: 0 auto; padding-top: var(--space-8); }
  .button { display: inline-flex; align-items: center; justify-content: center; gap: var(--space-2); min-height: var(--control); padding: var(--space-2) var(--space-4); border: 1px solid transparent; border-radius: var(--radius-sm); font-size: 0.875rem; font-weight: 700; line-height: 1; cursor: pointer; transition: background var(--transition), border-color var(--transition), transform var(--transition); }
  .button:active:not(:disabled) { transform: scale(0.98); }
  .button:disabled { opacity: 0.48; cursor: not-allowed; }
  .button.primary { color: hsl(210 30% 98%); background: var(--accent); }
  .button.primary:hover:not(:disabled) { background: var(--accent-hover); }
  .button.secondary { color: var(--text); border-color: var(--line); background: var(--surface); }
  .button.secondary:hover:not(:disabled) { border-color: var(--line-strong); background: var(--surface-muted); }
  .button.quiet { color: var(--text-muted); background: transparent; }
  .button.quiet:hover:not(:disabled) { color: var(--text); background: var(--surface-muted); }
  .button.quiet.danger { color: var(--danger); }
  .button.quiet.danger:hover:not(:disabled) { color: var(--danger); background: var(--danger-soft); }
  .icon-button { width: var(--control); padding: 0; color: var(--text); border-color: var(--line); background: var(--surface); font-size: 1.1rem; }
  .icon-button:hover { background: var(--surface-muted); }

  .alert { display: grid; grid-template-columns: auto 1fr auto; align-items: center; gap: var(--space-3); margin-bottom: var(--space-5); padding: var(--space-3) var(--space-4); border: 1px solid color-mix(in srgb, var(--danger), transparent 65%); border-radius: var(--radius-md); color: var(--danger); background: var(--danger-soft); font-size: 0.875rem; }
  .alert-messages { display: grid; gap: var(--space-1); }
  .alert button { border: 0; color: inherit; background: transparent; font-weight: 750; cursor: pointer; }
  .empty-state { display: flex; min-height: 62vh; flex-direction: column; align-items: center; justify-content: center; text-align: center; }
  .empty-state h2 { margin: var(--space-4) 0 var(--space-2); font-size: 1.4rem; letter-spacing: -0.025em; }
  .empty-state p { max-width: 34rem; margin: 0 0 var(--space-5); color: var(--text-muted); line-height: 1.55; }
  .empty-icon { display: grid; width: 4rem; height: 4rem; place-items: center; border: 1px solid var(--line); border-radius: var(--radius-lg); color: var(--accent); background: var(--surface); box-shadow: var(--shadow-card); font-size: 2rem; }
  .spinner { width: 1.75rem; height: 1.75rem; border: 3px solid var(--line); border-top-color: var(--accent); border-radius: 50%; animation: spin 0.8s linear infinite; }
  @keyframes spin { to { transform: rotate(360deg); } }

  .jobs { display: grid; gap: var(--space-5); }
  .job-card { overflow: hidden; border: 1px solid var(--line); border-radius: var(--radius-lg); background: var(--surface-raised); box-shadow: var(--shadow-card); }
  .job-card.failed { border-color: color-mix(in srgb, var(--danger), transparent 55%); }
  .job-header { display: flex; align-items: flex-start; justify-content: space-between; gap: var(--space-4); padding: var(--space-5) var(--space-5) var(--space-4); }
  .job-title { min-width: 0; }
  .job-title h2 { overflow: hidden; margin: 0 0 var(--space-1); font-size: 1.05rem; letter-spacing: -0.015em; text-overflow: ellipsis; white-space: nowrap; }
  .job-status { color: var(--text-muted); font-size: 0.8rem; font-weight: 600; }
  .percentage { color: var(--accent); font-size: 1.45rem; letter-spacing: -0.04em; font-variant-numeric: tabular-nums; }
  .percentage span { margin-left: 0.1em; font-size: 0.7em; }
  .failed .percentage { color: var(--danger); }
  .complete .percentage { color: var(--success); }
  .overall-track { height: 0.45rem; margin: 0 var(--space-5); overflow: hidden; border-radius: var(--radius-pill); background: var(--surface-muted); }
  .overall-track span { display: block; height: 100%; border-radius: inherit; background: var(--accent); transition: width 320ms ease-out; }
  .failed .overall-track span { background: var(--danger); }
  .complete .overall-track span { background: var(--success); }

  .pipeline { display: grid; grid-template-columns: repeat(5, 1fr); margin: var(--space-5); padding: 0; list-style: none; }
  .pipeline li { position: relative; display: flex; min-width: 0; flex-direction: column; align-items: center; gap: var(--space-2); color: var(--text-muted); font-size: 0.7rem; font-weight: 650; text-align: center; }
  .pipeline li::before { position: absolute; top: 0.7rem; right: 50%; z-index: 0; width: 100%; height: 2px; content: ""; background: var(--line); }
  .pipeline li:first-child::before { display: none; }
  .step-marker { position: relative; z-index: 1; display: grid; width: 1.4rem; height: 1.4rem; place-items: center; border: 2px solid var(--line); border-radius: 50%; color: var(--text-muted); background: var(--surface-raised); font-size: 0.65rem; }
  .pipeline li.done { color: var(--text); }
  .pipeline li.done::before { background: var(--success); }
  .pipeline li.done .step-marker { border-color: var(--success); color: var(--surface-raised); background: var(--success); }
  .pipeline li.current { color: var(--accent); }
  .pipeline li.current .step-marker { border-color: var(--accent); color: var(--accent); box-shadow: 0 0 0 3px var(--accent-soft); }
  .pipeline li.failed { color: var(--danger); }
  .pipeline li.failed .step-marker { border-color: var(--danger); color: var(--danger); box-shadow: 0 0 0 3px var(--danger-soft); }

  .job-footer { display: grid; grid-template-columns: 1fr auto; align-items: end; gap: var(--space-4); padding: var(--space-4) var(--space-5); border-top: 1px solid var(--line); background: color-mix(in srgb, var(--surface-muted), transparent 52%); }
  .metrics { display: flex; min-width: 0; flex-wrap: wrap; gap: var(--space-3) var(--space-5); margin: 0; }
  .metrics div { display: flex; gap: var(--space-2); min-width: 0; }
  .metrics dt { color: var(--text-muted); font-size: 0.75rem; }
  .metrics dd { margin: 0; font-size: 0.75rem; font-weight: 700; font-variant-numeric: tabular-nums; }
  .metrics .destination { max-width: min(28rem, 48vw); }
  .destination dd { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .job-error { grid-column: 1 / -1; margin: 0; padding: var(--space-2) var(--space-3); border-radius: var(--radius-sm); color: var(--danger); background: var(--danger-soft); font-size: 0.8rem; line-height: 1.4; }
  .job-actions { display: flex; flex-wrap: wrap; justify-content: flex-end; gap: var(--space-2); }
  .job-actions .button { min-height: 2rem; padding: var(--space-2) var(--space-3); font-size: 0.78rem; }

  .sheet-backdrop { position: fixed; z-index: 30; inset: 0; display: flex; align-items: flex-end; justify-content: center; padding-top: env(safe-area-inset-top); background: hsl(215 30% 10% / 0.38); }
  .settings-sheet { width: min(100%, 46rem); max-height: calc(100vh - var(--space-8)); overflow: auto; border: 1px solid var(--line); border-bottom: 0; border-radius: var(--radius-lg) var(--radius-lg) 0 0; background: var(--surface-raised); box-shadow: var(--shadow-sheet); }
  .sheet-header { position: sticky; z-index: 2; top: 0; display: flex; align-items: center; justify-content: space-between; padding: var(--space-5) var(--space-6); border-bottom: 1px solid var(--line); background: color-mix(in srgb, var(--surface-raised), transparent 5%); backdrop-filter: blur(18px); }
  .eyebrow { margin: 0 0 var(--space-1); color: var(--accent); font-size: 0.7rem; font-weight: 800; letter-spacing: 0.08em; text-transform: uppercase; }
  .sheet-header h2 { margin: 0; font-size: 1.35rem; letter-spacing: -0.025em; }
  .close-button { display: grid; width: var(--control); height: var(--control); place-items: center; border: 0; border-radius: 50%; color: var(--text-muted); background: var(--surface-muted); font-size: 1.45rem; cursor: pointer; }
  form { padding: var(--space-2) var(--space-6) calc(var(--space-6) + env(safe-area-inset-bottom)); }
  fieldset { margin: 0; padding: var(--space-5) 0; border: 0; border-bottom: 1px solid var(--line); }
  legend { width: 100%; margin-bottom: var(--space-4); padding: 0; color: var(--text); font-size: 0.9rem; font-weight: 800; }
  .form-grid { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: var(--space-4); }
  .server-grid { grid-template-columns: minmax(0, 3fr) minmax(7rem, 1fr); }
  label { display: grid; gap: var(--space-2); color: var(--text-muted); font-size: 0.78rem; font-weight: 700; }
  input:not([type="checkbox"]) { width: 100%; height: var(--control); padding: 0 var(--space-3); border: 1px solid var(--line-strong); border-radius: var(--radius-sm); color: var(--text); background: var(--surface); transition: border-color var(--transition), box-shadow var(--transition); }
  input:not([type="checkbox"]):focus { border-color: var(--accent); }
  input[readonly] { color: var(--text-muted); background: var(--surface-muted); }
  .toggle-row { display: flex; align-items: center; justify-content: space-between; gap: var(--space-5); margin: var(--space-4) 0; padding: var(--space-3) var(--space-4); border-radius: var(--radius-md); background: var(--surface-muted); }
  .toggle-row span { display: grid; gap: var(--space-1); color: var(--text); }
  .toggle-row small { color: var(--text-muted); font-weight: 500; }
  input[type="checkbox"] { width: 2.65rem; height: 1.5rem; margin: 0; appearance: none; border-radius: var(--radius-pill); background: var(--line-strong); cursor: pointer; transition: background var(--transition); }
  input[type="checkbox"]::before { display: block; width: 1.15rem; height: 1.15rem; margin: 0.175rem; border-radius: 50%; content: ""; background: var(--surface-raised); box-shadow: 0 1px 3px hsl(215 30% 15% / 0.25); transition: transform var(--transition); }
  input[type="checkbox"]:checked { background: var(--accent); }
  input[type="checkbox"]:checked::before { transform: translateX(1.15rem); }
  .path-control { display: grid; grid-template-columns: 1fr auto; gap: var(--space-2); }
  .form-message { margin: var(--space-4) 0 0; padding: var(--space-3); border-radius: var(--radius-sm); font-size: 0.82rem; font-weight: 650; }
  .form-message.error { color: var(--danger); background: var(--danger-soft); }
  .form-message.success { color: var(--success); background: var(--success-soft); }
  .sheet-actions { display: flex; align-items: center; justify-content: space-between; gap: var(--space-3); padding-top: var(--space-5); }
  .sheet-actions div { display: flex; gap: var(--space-2); }

  @media (max-width: 42rem) {
    .toolbar { padding-inline: max(var(--space-3), env(safe-area-inset-left)); }
    .connection-chip { max-width: 8.5rem; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    nav .primary { width: var(--control); padding: 0; font-size: 0; }
    nav .primary span { font-size: 1.25rem; }
    main { width: min(calc(100% - var(--space-4)), var(--content)); padding-top: var(--space-4); }
    .job-header { padding: var(--space-4); }
    .overall-track { margin-inline: var(--space-4); }
    .pipeline { margin: var(--space-5) var(--space-2); }
    .pipeline li { font-size: 0.62rem; }
    .job-footer { grid-template-columns: 1fr; padding: var(--space-4); }
    .metrics .destination { max-width: 82vw; }
    .job-actions { justify-content: flex-start; }
    .alert { grid-template-columns: 1fr; }
    .settings-sheet { max-height: calc(100vh - var(--space-4)); }
    .sheet-header, form { padding-inline: var(--space-4); }
    .form-grid, .server-grid { grid-template-columns: 1fr; }
    .path-control { grid-template-columns: 1fr; }
    .sheet-actions { align-items: stretch; flex-direction: column-reverse; }
    .sheet-actions > .button, .sheet-actions div, .sheet-actions div .button { flex: 1; }
  }

  @media (prefers-color-scheme: dark) {
    :global(:root) {
      --canvas: hsl(216 18% 11%);
      --surface: hsl(216 17% 15%);
      --surface-raised: hsl(216 17% 17%);
      --surface-muted: hsl(216 14% 21%);
      --text: hsl(210 22% 92%);
      --text-muted: hsl(214 12% 66%);
      --line: hsl(215 12% 27%);
      --line-strong: hsl(215 12% 38%);
      --accent: hsl(211 94% 62%);
      --accent-hover: hsl(211 94% 68%);
      --accent-soft: hsl(211 62% 21%);
      --success: hsl(149 56% 51%);
      --success-soft: hsl(149 40% 18%);
      --danger: hsl(3 78% 65%);
      --danger-soft: hsl(3 45% 20%);
      --shadow-card: 0 1px 2px hsl(215 30% 4% / 0.25), 0 12px 36px hsl(215 30% 4% / 0.24);
      --shadow-sheet: 0 -12px 48px hsl(215 30% 4% / 0.48);
    }
  }

  @media (prefers-reduced-motion: reduce) {
    *, *::before, *::after { scroll-behavior: auto !important; animation-duration: 0.01ms !important; animation-iteration-count: 1 !important; transition-duration: 0.01ms !important; }
  }
</style>
