<script module lang="ts">
  export type SelectOption = {
    value: string;
    label: string;
    disabled?: boolean;
  };
</script>

<script lang="ts">
  import { onMount, tick } from 'svelte';

  let {
    id,
    options,
    value = $bindable(''),
    placeholder,
    disabled = false,
    ariaLabel,
    onvaluechange = () => {}
  }: {
    id: string;
    options: SelectOption[];
    value?: string;
    placeholder: string;
    disabled?: boolean;
    ariaLabel?: string;
    onvaluechange?: (value: string) => void;
  } = $props();

  let root: HTMLDivElement;
  let trigger: HTMLButtonElement;
  let isOpen = $state(false);
  let activeIndex = $state(-1);
  let selectedOption = $derived(options.find((option: SelectOption) => option.value === value));
  let selectedIndex = $derived(options.findIndex((option: SelectOption) => option.value === value));
  let listboxId = $derived(`${id}-listbox`);

  function optionId(index: number) {
    return `${id}-option-${index}`;
  }

  function isSelectable(index: number) {
    return index >= 0 && index < options.length && !options[index].disabled;
  }

  function firstEnabledIndex() {
    return options.findIndex((option: SelectOption) => !option.disabled);
  }

  function lastEnabledIndex() {
    for (let index = options.length - 1; index >= 0; index -= 1) {
      if (!options[index].disabled) return index;
    }

    return -1;
  }

  function nextEnabledIndex(currentIndex: number, direction: 1 | -1) {
    if (options.length === 0) return -1;

    for (let step = 1; step <= options.length; step += 1) {
      const index = (currentIndex + direction * step + options.length) % options.length;
      if (isSelectable(index)) return index;
    }

    return -1;
  }

  function preferredActiveIndex() {
    if (isSelectable(activeIndex)) return activeIndex;
    if (isSelectable(selectedIndex)) return selectedIndex;
    return firstEnabledIndex();
  }

  function close(restoreFocus = false) {
    isOpen = false;
    if (restoreFocus) trigger?.focus();
  }

  async function openAndFocus(index: number) {
    if (index === -1 || disabled) return;

    isOpen = true;
    activeIndex = index;
    await tick();
    document.getElementById(optionId(index))?.focus();
  }

  function toggleList() {
    if (disabled) return;

    if (isOpen) {
      close();
      return;
    }

    isOpen = true;
    activeIndex = preferredActiveIndex();
  }

  function selectOption(index: number) {
    const option = options[index];
    if (!option || option.disabled || disabled) return;

    const previousValue = value;
    value = option.value;
    if (value !== previousValue) onvaluechange(value);
    activeIndex = index;
    close(true);
  }

  function handleTriggerKeydown(event: KeyboardEvent) {
    if (disabled) return;

    if (event.key === 'Escape' && isOpen) {
      event.preventDefault();
      close(true);
      return;
    }

    let index = -1;
    if (event.key === 'ArrowDown') {
      const current = preferredActiveIndex();
      index = isOpen ? nextEnabledIndex(current, 1) : current;
    } else if (event.key === 'ArrowUp') {
      const current = preferredActiveIndex();
      index = isOpen ? nextEnabledIndex(current, -1) : current === -1 ? -1 : lastEnabledIndex();
    } else if (event.key === 'Home') {
      index = firstEnabledIndex();
    } else if (event.key === 'End') {
      index = lastEnabledIndex();
    } else {
      return;
    }

    event.preventDefault();
    void openAndFocus(index);
  }

  function handleOptionKeydown(event: KeyboardEvent, index: number) {
    if (event.key === 'Enter' || event.key === ' ') {
      event.preventDefault();
      selectOption(index);
      return;
    }

    if (event.key === 'Escape') {
      event.preventDefault();
      close(true);
      return;
    }

    let nextIndex = -1;
    if (event.key === 'ArrowDown') {
      nextIndex = nextEnabledIndex(index, 1);
    } else if (event.key === 'ArrowUp') {
      nextIndex = nextEnabledIndex(index, -1);
    } else if (event.key === 'Home') {
      nextIndex = firstEnabledIndex();
    } else if (event.key === 'End') {
      nextIndex = lastEnabledIndex();
    } else {
      return;
    }

    event.preventDefault();
    void openAndFocus(nextIndex);
  }

  $effect(() => {
    if (isOpen && !isSelectable(activeIndex)) {
      activeIndex = isSelectable(selectedIndex) ? selectedIndex : firstEnabledIndex();
    }
  });

  onMount(() => {
    function handlePointerDown(event: PointerEvent) {
      if (isOpen && root && !event.composedPath().includes(root)) close();
    }

    function handleDocumentKeydown(event: KeyboardEvent) {
      if (isOpen && event.key === 'Escape') close(true);
    }

    document.addEventListener('pointerdown', handlePointerDown);
    document.addEventListener('keydown', handleDocumentKeydown);

    return () => {
      document.removeEventListener('pointerdown', handlePointerDown);
      document.removeEventListener('keydown', handleDocumentKeydown);
    };
  });
</script>

<div class="select" bind:this={root}>
  <button
    bind:this={trigger}
    type="button"
    class="trigger"
    {id}
    {disabled}
    aria-label={ariaLabel}
    aria-controls={listboxId}
    aria-expanded={isOpen}
    aria-haspopup="listbox"
    onclick={toggleList}
    onkeydown={handleTriggerKeydown}
  >
    <span class:placeholder={!selectedOption} class="value">{selectedOption?.label ?? placeholder}</span>
    <span class="chevron" aria-hidden="true"></span>
  </button>

  {#if isOpen}
    <ul id={listboxId} class="listbox" role="listbox" aria-label={ariaLabel ?? placeholder}>
      {#each options as option, index (option.value)}
        <li
          id={optionId(index)}
          class="option"
          class:selected={option.value === value}
          class:disabled={option.disabled}
          role="option"
          aria-disabled={option.disabled || undefined}
          aria-selected={option.value === value}
          tabindex={option.disabled ? -1 : activeIndex === index ? 0 : -1}
          onclick={() => selectOption(index)}
          onkeydown={(event) => handleOptionKeydown(event, index)}
        >
          {option.label}
        </li>
      {/each}
    </ul>
  {/if}
</div>

<style>
  .select {
    --select-surface: var(--page-bg, #121214);
    --select-surface-hover: var(--surface-soft, rgba(255, 255, 255, 0.05));
    --select-surface-raised: var(--surface, #1e1e22);
    --select-border: var(--border, #27272a);
    --select-border-strong: var(--border-strong, #3f3f46);
    --select-text: var(--text-strong, #f4f4f5);
    --select-text-subtle: var(--text-subtle, #71717a);
    --select-primary: var(--primary-ring, #3b82f6);
    --select-primary-soft: var(--primary-soft, rgba(59, 130, 246, 0.12));
    --select-focus-ring: var(--focus-ring, rgba(59, 130, 246, 0.2));
    --select-shadow: var(--shadow-raised, 0 14px 30px rgba(0, 0, 0, 0.25));
    position: relative;
    width: 100%;
  }

  .trigger {
    display: flex;
    align-items: center;
    justify-content: space-between;
    width: 100%;
    min-height: 2.75rem;
    padding: 0.625rem 0.75rem;
    color: var(--select-text);
    font: inherit;
    font-size: 0.875rem;
    line-height: 1.25;
    text-align: left;
    cursor: pointer;
    background: var(--select-surface);
    border: 1px solid var(--select-border);
    border-radius: var(--radius-sm, 0.5rem);
    outline: none;
    transition: border-color 160ms ease-out, box-shadow 160ms ease-out, background-color 160ms ease-out;
  }

  .trigger:hover:not(:disabled) {
    border-color: var(--select-border-strong);
  }

  .trigger:focus-visible {
    border-color: var(--select-primary);
    box-shadow: 0 0 0 3px var(--select-focus-ring);
  }

  .trigger:disabled {
    color: var(--select-text-subtle);
    cursor: not-allowed;
    opacity: 0.65;
  }

  .value {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .value.placeholder {
    color: var(--select-text-subtle);
  }

  .chevron {
    flex: 0 0 auto;
    width: 0.5rem;
    height: 0.5rem;
    margin-left: 0.75rem;
    border-right: 1.5px solid currentColor;
    border-bottom: 1.5px solid currentColor;
    transform: rotate(45deg) translateY(-0.125rem);
    transition: transform 160ms ease-out;
  }

  .trigger[aria-expanded="true"] .chevron {
    transform: rotate(225deg) translateY(-0.125rem);
  }

  .listbox {
    position: absolute;
    z-index: 20;
    top: calc(100% + 0.375rem);
    left: 0;
    width: 100%;
    max-height: 16rem;
    padding: 0.25rem;
    margin: 0;
    overflow-y: auto;
    list-style: none;
    background: var(--select-surface-raised);
    border: 1px solid var(--select-border-strong);
    border-radius: var(--radius-sm, 0.5rem);
    box-shadow: var(--select-shadow);
  }

  .option {
    padding: 0.625rem 0.75rem;
    color: var(--select-text);
    font-size: 0.875rem;
    line-height: 1.25;
    cursor: pointer;
    border-radius: calc(var(--radius-sm, 0.5rem) - 0.25rem);
    outline: none;
  }

  .option:hover,
  .option:focus-visible {
    background: var(--select-surface-hover);
  }

  .option:focus-visible {
    box-shadow: 0 0 0 2px var(--select-focus-ring);
  }

  .option.selected {
    color: var(--select-text);
    background: var(--select-primary-soft);
  }

  .option.disabled {
    color: var(--select-text-subtle);
    cursor: not-allowed;
  }

  .option.disabled:hover {
    background: transparent;
  }
</style>
