<script lang="ts">
  let { children } = $props();
</script>

{@render children()}

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

  /* Geteilte Buttons (Toolbar, Job-Karten, Einstellungs-Sheet) */
  :global(.button) { display: inline-flex; align-items: center; justify-content: center; gap: var(--space-2); min-height: var(--control); padding: var(--space-2) var(--space-4); border: 1px solid transparent; border-radius: var(--radius-sm); font-size: 0.875rem; font-weight: 700; line-height: 1; cursor: pointer; transition: background var(--transition), border-color var(--transition), transform var(--transition); }
  :global(.button:active:not(:disabled)) { transform: scale(0.98); }
  :global(.button:disabled) { opacity: 0.48; cursor: not-allowed; }
  :global(.button.primary) { color: hsl(210 30% 98%); background: var(--accent); }
  :global(.button.primary:hover:not(:disabled)) { background: var(--accent-hover); }
  :global(.button.secondary) { color: var(--text); border-color: var(--line); background: var(--surface); }
  :global(.button.secondary:hover:not(:disabled)) { border-color: var(--line-strong); background: var(--surface-muted); }
  :global(.button.quiet) { color: var(--text-muted); background: transparent; }
  :global(.button.quiet:hover:not(:disabled)) { color: var(--text); background: var(--surface-muted); }
  :global(.button.quiet.danger) { color: var(--danger); }
  :global(.button.quiet.danger:hover:not(:disabled)) { color: var(--danger); background: var(--danger-soft); }
  :global(.button.icon-button) { width: var(--control); padding: 0; color: var(--text); border-color: var(--line); background: var(--surface); font-size: 1.1rem; }
  :global(.button.icon-button:hover) { background: var(--surface-muted); }

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
    :global(*), :global(*::before), :global(*::after) { scroll-behavior: auto !important; animation-duration: 0.01ms !important; animation-iteration-count: 1 !important; transition-duration: 0.01ms !important; }
  }
</style>
