<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { open } from "@tauri-apps/plugin-dialog";
  import { tick } from "svelte";
  import type { AppSettings } from "$lib/jobs";
  import { defaultSettings, errorText } from "$lib/jobs";

  interface Props {
    onclose: () => void;
    onbackend: () => void;
  }

  let { onclose, onbackend }: Props = $props();

  let settings = $state<AppSettings>({ ...defaultSettings });
  let sheet: HTMLElement | undefined = $state();
  let loading = $state(false);
  let saving = $state(false);
  let testing = $state(false);
  let testMessage = $state("");
  let errorMessage = $state("");
  let disposed = false;

  function handleKeydown(event: KeyboardEvent): void {
    if (event.key === "Escape" && !saving) onclose();
  }

  async function chooseDownloadDir(): Promise<void> {
    errorMessage = "";
    try {
      const directory = await open({ directory: true, multiple: false, defaultPath: settings.downloadDir || undefined });
      if (typeof directory === "string") settings.downloadDir = directory;
    } catch (error) {
      errorMessage = `Ordner konnte nicht ausgewählt werden: ${errorText(error)}`;
    }
  }

  async function testConnection(): Promise<void> {
    errorMessage = "";
    testMessage = "";
    testing = true;
    try {
      await invoke("test_server", { settings });
      testMessage = "Verbindung erfolgreich hergestellt.";
    } catch (error) {
      errorMessage = `Verbindung fehlgeschlagen: ${errorText(error)}`;
    } finally {
      testing = false;
    }
  }

  async function save(event: SubmitEvent): Promise<void> {
    event.preventDefault();
    errorMessage = "";
    testMessage = "";
    saving = true;
    try {
      await invoke("save_settings", { settings });
      onbackend();
      onclose();
    } catch (error) {
      errorMessage = `Einstellungen konnten nicht gespeichert werden: ${errorText(error)}`;
      onbackend();
    } finally {
      saving = false;
    }
  }

  $effect(() => {
    (async () => {
      loading = true;
      await tick();
      sheet?.focus();
      try {
        settings = await invoke<AppSettings>("get_settings");
      } catch (error) {
        errorMessage = `Einstellungen konnten nicht geladen werden: ${errorText(error)}`;
      } finally {
        if (!disposed) loading = false;
      }
    })();
    return () => { disposed = true; };
  });
</script>

<svelte:window onkeydown={handleKeydown} />

<div class="sheet-backdrop">
  <div class="settings-sheet" role="dialog" aria-modal="true" aria-labelledby="settings-title" tabindex="-1" bind:this={sheet}>
    <header class="sheet-header">
      <div><p class="eyebrow">Konfiguration</p><h2 id="settings-title">Einstellungen</h2></div>
      <button class="close-button" type="button" onclick={() => { if (!saving) onclose(); }} aria-label="Einstellungen schließen">×</button>
    </header>

    <form onsubmit={save}>
      <fieldset disabled={loading || saving || testing}>
        <legend>Newsserver</legend>
        <div class="form-grid server-grid">
          <label><span>Server</span><input required autocomplete="url" placeholder="news.example.net" bind:value={settings.host} /></label>
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

      <fieldset disabled={loading || saving || testing}>
        <legend>Speicherort</legend>
        <label><span>Zielordner</span><div class="path-control"><input required readonly bind:value={settings.downloadDir} /><button class="button secondary" type="button" onclick={chooseDownloadDir}>Auswählen …</button></div></label>
      </fieldset>

      {#if errorMessage}<p class="form-message error" role="alert">{errorMessage}</p>{/if}
      {#if testMessage}<p class="form-message success" role="status">{testMessage}</p>{/if}

      <footer class="sheet-actions">
        <button class="button secondary" type="button" onclick={testConnection} disabled={loading || testing || saving}>{testing ? "Wird getestet …" : "Verbindung testen"}</button>
        <div>
          <button class="button quiet" type="button" onclick={onclose} disabled={saving}>Abbrechen</button>
          <button class="button primary" type="submit" disabled={loading || saving || testing}>{saving ? "Wird gespeichert …" : "Speichern"}</button>
        </div>
      </footer>
    </form>
  </div>
</div>

<style>
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
    .settings-sheet { max-height: calc(100vh - var(--space-4)); }
    .sheet-header, form { padding-inline: var(--space-4); }
    .form-grid, .server-grid { grid-template-columns: 1fr; }
    .path-control { grid-template-columns: 1fr; }
    .sheet-actions { align-items: stretch; flex-direction: column-reverse; }
    .sheet-actions > .button, .sheet-actions div, .sheet-actions div .button { flex: 1; }
  }
</style>
