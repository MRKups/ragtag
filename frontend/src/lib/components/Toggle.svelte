<script lang="ts">
  let { checked = $bindable(), label = "", description = "", icon = null, onchange = undefined }: {
    checked: boolean;
    label?: string;
    description?: string;
    icon?: any;
    onchange?: (value: boolean) => void;
  } = $props();

  function toggle() {
    checked = !checked;
    onchange?.(checked);
  }
</script>

<div class="setting-item" role="switch" aria-checked={checked} tabindex="0" onclick={toggle} onkeydown={(e) => (e.key === 'Enter' || e.key === ' ') && (e.preventDefault(), toggle())}>
  <div class="setting-info">
    <div class="setting-label">
      {#if icon}
        {@const Icon = icon}
        <Icon size={16} />
      {/if}
      {label}
    </div>
    {#if description}
      <div class="setting-desc">{description}</div>
    {/if}
  </div>

  <div class="toggle-switch {checked ? 'checked' : ''}">
    <div class="toggle-slider"></div>
  </div>
</div>

<style>
  .setting-item {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 1.25rem;
    background-color: var(--bg-card, #1f2937);
    border: 1px solid var(--border-card, #374151);
    border-radius: 0.75rem;
    margin-bottom: 1rem;
    cursor: pointer;
    transition: border-color 0.2s;
  }

  .setting-item:hover {
    border-color: var(--border-card-hover, #4b5563);
  }

  .setting-info {
    flex: 1;
    padding-right: 1.5rem;
  }

  .setting-label {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    font-size: 0.9375rem;
    font-weight: 500;
    color: var(--text-heading, #f3f4f6);
    margin-bottom: 0.25rem;
  }

  .setting-desc {
    font-size: 0.8125rem;
    color: var(--text-secondary, #9ca3af);
    line-height: 1.4;
  }

  .toggle-switch {
    position: relative;
    width: 50px;
    height: 26px;
    background-color: var(--border-card, #374151);
    border-radius: 34px;
    transition: 0.4s;
    flex-shrink: 0;
  }

  .toggle-switch.checked {
    background-color: var(--primary, #2563eb);
  }

  .toggle-slider {
    position: absolute;
    height: 20px;
    width: 20px;
    left: 3px;
    bottom: 3px;
    background-color: white;
    border-radius: 50%;
    transition: 0.4s;
  }

  .toggle-switch.checked .toggle-slider {
    transform: translateX(24px);
  }
</style>
