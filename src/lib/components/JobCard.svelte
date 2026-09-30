<script lang="ts">
  import type { JobView } from "$lib/jobs";
  import { formatBytes, overallProgress, statusLabel, stepState } from "$lib/jobs";

  type Step = { key: string; label: string };

  interface Props {
    job: JobView;
    busy: boolean;
    pipeline: readonly Step[];
    onpause: () => void;
    onresume: () => void;
    onrerun: () => void;
    onreprocess: () => void;
    ondelete: () => void;
  }

  let { job, busy, pipeline, onpause, onresume, onrerun, onreprocess, ondelete }: Props = $props();
</script>

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
        <button class="button secondary" type="button" disabled={busy} onclick={onpause}>Ⅱ&nbsp; Pause</button>
      {:else if job.status === "paused"}
        <button class="button primary" type="button" disabled={busy} onclick={onresume}>▶&nbsp; Fortsetzen</button>
      {:else if job.status === "failed"}
        <button class="button primary" type="button" disabled={busy} onclick={onrerun}>↻&nbsp; Neu herunterladen</button>
        <button class="button secondary" type="button" disabled={busy} onclick={onreprocess}>Erneut verarbeiten</button>
        <button class="button quiet danger" type="button" disabled={busy} onclick={ondelete}>Löschen</button>
      {:else if job.status === "complete"}
        <button class="button primary" type="button" disabled={busy} onclick={onrerun}>↻&nbsp; Neu herunterladen</button>
        <button class="button quiet danger" type="button" disabled={busy} onclick={ondelete}>Löschen</button>
      {/if}
    </div>
  </footer>
</article>

<style>
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

  @media (max-width: 42rem) {
    .job-header { padding: var(--space-4); }
    .overall-track { margin-inline: var(--space-4); }
    .pipeline { margin: var(--space-5) var(--space-2); }
    .pipeline li { font-size: 0.62rem; }
    .job-footer { grid-template-columns: 1fr; padding: var(--space-4); }
    .metrics .destination { max-width: 82vw; }
    .job-actions { justify-content: flex-start; }
  }
</style>
