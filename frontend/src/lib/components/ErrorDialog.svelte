<script lang="ts">
  import { tick } from 'svelte';
  import { errorDialog } from '$lib/stores/error-dialog.svelte';

  let closeButton = $state<HTMLButtonElement>();

  $effect(() => {
    if (!errorDialog.active) return;

    void tick().then(() => closeButton?.focus());
  });

  function dismiss() {
    errorDialog.dismiss();
  }

  function handleKeydown(event: KeyboardEvent) {
    if (event.key !== 'Escape' || !errorDialog.active) return;

    event.preventDefault();
    dismiss();
  }

  async function runAction() {
    const action = errorDialog.active?.action;
    if (!action) return;

    dismiss();
    await action.run();
  }
</script>

<svelte:window onkeydown={handleKeydown} />

{#if errorDialog.active}
  <div class="error-dialog-overlay">
    <button
      type="button"
      class="error-dialog-backdrop"
      aria-label="Dismiss error dialog"
      tabindex="-1"
      onclick={dismiss}
    ></button>
    <dialog
      class="error-dialog"
      open
      aria-labelledby="error-dialog-title"
      aria-describedby="error-dialog-description"
    >
      <div class="error-dialog-header">
        <div class="title-row">
          <h2 id="error-dialog-title">{errorDialog.active.title}</h2>
          {#if errorDialog.active.code || errorDialog.active.transient !== undefined}
            <div class="badges">
              {#if errorDialog.active.code}
                <span class="error-badge code-badge">{errorDialog.active.code.toUpperCase()}</span>
              {/if}
              {#if errorDialog.active.transient !== undefined}
                <span class="error-badge {errorDialog.active.transient ? 'transient-badge' : 'terminal-badge'}">
                  {errorDialog.active.transient ? 'Retryable' : 'Terminal'}
                </span>
              {/if}
            </div>
          {/if}
        </div>
      </div>

      <div id="error-dialog-description" class="error-dialog-description">
        <p class="error-dialog-message">{errorDialog.active.message}</p>
        {#if errorDialog.active.guidance}
          <div class="error-dialog-recovery">
            <span class="recovery-label">Suggested Action:</span>
            <p class="error-dialog-guidance">{errorDialog.active.guidance}</p>
          </div>
        {/if}
      </div>

      <div class="error-dialog-actions">
        <button bind:this={closeButton} type="button" class="dismiss-button" onclick={dismiss}>Close</button>
        {#if errorDialog.active.action}
          <button type="button" class="action-button" onclick={runAction}>
            {errorDialog.active.action.label}
          </button>
        {/if}
      </div>
    </dialog>
  </div>
{/if}

<style>
  .error-dialog-overlay {
    position: fixed;
    z-index: 100;
    inset: 0;
    display: grid;
    place-items: center;
    padding: 1rem;
    background: rgba(2, 6, 23, 0.72);
  }

  .error-dialog-backdrop {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    padding: 0;
    cursor: default;
    background: transparent;
    border: 0;
  }

  .error-dialog {
    position: relative;
    z-index: 1;
    width: min(100%, 32rem);
    margin: 0;
    padding: 1.5rem;
    color: var(--text-primary, #f4f4f5);
    background: var(--bg-card, #1e1e22);
    border: 1px solid var(--border-card, #2e2e33);
    border-radius: 0.75rem;
    box-shadow: 0 24px 60px rgba(0, 0, 0, 0.42);
  }

  .error-dialog-header {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 1rem;
  }

  .title-row {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }

  .badges {
    display: flex;
    flex-wrap: wrap;
    gap: 0.375rem;
    align-items: center;
  }

  .error-badge {
    display: inline-block;
    padding: 0.125rem 0.375rem;
    border-radius: 0.25rem;
    font-size: 0.6875rem;
    font-weight: 700;
    letter-spacing: 0.04em;
    line-height: 1.2;
    text-transform: uppercase;
  }

  .code-badge {
    color: #94a3b8;
    background: rgba(148, 163, 184, 0.12);
    border: 1px solid #475569;
  }

  .transient-badge {
    color: #fcd34d;
    background: rgba(245, 158, 11, 0.12);
    border: 1px solid rgba(245, 158, 11, 0.3);
  }

  .terminal-badge {
    color: #fca5a5;
    background: rgba(239, 68, 68, 0.12);
    border: 1px solid rgba(239, 68, 68, 0.3);
  }

  h2 {
    margin: 0;
    color: #f9fafb;
    font-size: 1.25rem;
    font-weight: 650;
    letter-spacing: -0.02em;
  }

  .error-dialog-description {
    margin-top: 1rem;
  }

  .error-dialog-message {
    margin: 0;
    font-size: 0.9375rem;
    line-height: 1.55;
    color: #e2e8f0;
  }

  .error-dialog-recovery {
    margin-top: 0.875rem;
    padding: 0.75rem 0.875rem;
    background: rgba(15, 23, 42, 0.6);
    border-left: 3px solid #38bdf8;
    border-radius: 0 0.375rem 0.375rem 0;
  }

  .recovery-label {
    display: block;
    font-size: 0.75rem;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: #38bdf8;
    margin-bottom: 0.25rem;
  }

  .error-dialog-guidance {
    margin: 0;
    font-size: 0.875rem;
    line-height: 1.5;
    color: #cbd5e1;
  }

  .error-dialog-actions {
    display: flex;
    justify-content: flex-end;
    gap: 0.75rem;
    margin-top: 1.5rem;
    padding-top: 1.25rem;
    border-top: 1px solid #475569;
  }

  .dismiss-button,
  .action-button {
    min-height: 2.5rem;
    padding: 0.5rem 0.875rem;
    border-radius: 0.5rem;
    font: inherit;
    font-size: 0.875rem;
    font-weight: 600;
    cursor: pointer;
  }

  .dismiss-button {
    color: var(--text-primary, #f4f4f5);
    background: var(--bg-card-hover, #26262b);
    border: 1px solid var(--border-card, #2e2e33);
  }

  .dismiss-button:hover {
    background: var(--border-card-hover, #3f3f46);
  }

  .action-button {
    color: white;
    background: #2563eb;
    border: 1px solid #3b82f6;
  }

  .action-button:hover {
    background: #1d4ed8;
  }

  .dismiss-button:focus-visible,
  .action-button:focus-visible {
    outline: 2px solid #60a5fa;
    outline-offset: 2px;
  }

  @media (max-width: 30rem) {
    .error-dialog {
      padding: 1.25rem;
    }

    .error-dialog-actions {
      flex-direction: column-reverse;
    }

    .dismiss-button,
    .action-button {
      width: 100%;
    }
  }
</style>
