<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { open } from "@tauri-apps/plugin-dialog";
  import { onMount } from "svelte";
  import JobCard from "$lib/components/JobCard.svelte";
  import SettingsSheet from "$lib/components/SettingsSheet.svelte";
  import type { BackendStatus, DownloadEvent, JobView } from "$lib/jobs";
  import {
    CLEAR_ARM_MS,
    DEBOUNCE_MS,
    clamp,
    errorText,
    isRemovable,
    mergeJobs,
    pipeline,
  } from "$lib/jobs";

  let jobs = $state<JobView[]>([]);
  let backend = $state<BackendStatus>({ configured: false, connected: false, error: null, version: "" });
  let loading = $state(true);
  let loadError = $state("");
  let globalError = $state("");
  let adding = $state(false);
  let sheetOpen = $state(false);
  let busyJobs = $state<Set<number>>(new Set());
  let deleting = $state(false);
  let clearArmed = $state(false);
  let clearArmTimer: ReturnType<typeof setTimeout> | undefined;
  let reloadTimer: ReturnType<typeof setTimeout> | undefined;
  let loadSequence = 0;
  let removableCount = $derived(jobs.filter(isRemovable).length);

  async function loadJobs(): Promise<void> {
    const sequence = ++loadSequence;
    try {
      const next = await invoke<JobView[]>("list_jobs");
      if (sequence === loadSequence) {
        jobs = mergeJobs(jobs, next);
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
      backend = { configured: false, connected: false, error: errorText(error), version: "" };
    }
  }

  function scheduleReload(immediate = false): void {
    if (reloadTimer) clearTimeout(reloadTimer);
    reloadTimer = setTimeout(() => void loadJobs(), immediate ? 0 : DEBOUNCE_MS);
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
    clearArmTimer = setTimeout(() => { clearArmed = false; }, CLEAR_ARM_MS);
  }

  async function clearAllJobs(): Promise<void> {
    clearArmed = false;
    if (clearArmTimer) clearTimeout(clearArmTimer);
    globalError = "";
    deleting = true;
    try {
      const removed = await invoke<number>("clear_jobs");
      if (removed > 0) {
        jobs = jobs.filter((job) => !isRemovable(job));
      }
    } catch (error) {
      globalError = `Alle löschen fehlgeschlagen: ${errorText(error)}`;
    } finally {
      deleting = false;
    }
  }

  onMount(() => {
    let disposed = false;
    const unlisteners: UnlistenFn[] = [];
    void Promise.all([loadJobs(), loadBackend()]);
    void Promise.all([
      listen<DownloadEvent>("download-event", ({ payload }) => updateFromEvent(payload)),
      listen<number[]>("nzb-imported", () => scheduleReload(true)),
      listen<string>("nzb-import-error", ({ payload }) => { globalError = payload; })
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

<div class="app-shell">
  <header class="toolbar">
    <div class="brand-block">
      <h1>NZB Deck</h1>
      <button class:connected={backend.connected} class="connection-chip" type="button" onclick={() => { sheetOpen = true; }} disabled={sheetOpen} aria-label="Verbindungseinstellungen öffnen">
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
      <button class="button icon-button" type="button" onclick={() => { sheetOpen = true; }} disabled={sheetOpen} aria-label="Einstellungen öffnen" title="Einstellungen">
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
          <JobCard
            {job}
            pipeline={pipeline}
            busy={busyJobs.has(job.id)}
            onpause={() => runJobAction(job.id, "pause_job")}
            onresume={() => runJobAction(job.id, "resume_job")}
            onrerun={() => runJobAction(job.id, "rerun_job")}
            onreprocess={() => runJobAction(job.id, "reprocess_job")}
            ondelete={() => deleteJob(job.id)}
          />
        {/each}
      </section>
    {/if}
  </main>

  <footer class="version-bar" aria-label="App-Version">
    <span>NZB Deck v{backend.version || "–"}</span>
  </footer>
</div>

{#if sheetOpen}
  <SettingsSheet onclose={() => { sheetOpen = false; }} onbackend={loadBackend} />
{/if}

<style>
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

  .version-bar { position: fixed; bottom: calc(var(--space-2) + env(safe-area-inset-bottom)); left: max(var(--space-3), env(safe-area-inset-left)); z-index: 5; padding: 0.1rem 0.5rem; border-radius: var(--radius-sm); color: var(--text-muted); background: color-mix(in srgb, var(--surface-raised), transparent 18%); font-size: 0.72rem; line-height: 1.4; letter-spacing: 0.02em; opacity: 0.85; pointer-events: none; user-select: none; }

  @media (max-width: 42rem) {
    .toolbar { padding-inline: max(var(--space-3), env(safe-area-inset-left)); }
    .connection-chip { max-width: 8.5rem; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    nav .primary { width: var(--control); padding: 0; font-size: 0; }
    nav .primary span { font-size: 1.25rem; }
    main { width: min(calc(100% - var(--space-4)), var(--content)); padding-top: var(--space-4); }
    .alert { grid-template-columns: 1fr; }
  }
</style>
