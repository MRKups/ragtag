<script lang="ts">
  import { Send, Loader2, Square } from 'lucide-svelte';

  let {
    isLoading = false,
    isStreaming = false,
    onSubmit = () => {},
    onCancel = () => {},
    text = $bindable('')
  } = $props<{
    isLoading?: boolean;
    isStreaming?: boolean;
    onSubmit?: (text: string) => void;
    onCancel?: () => void;
    text?: string;
  }>();

  let textarea: HTMLTextAreaElement;
  let disabled = $derived(isLoading || isStreaming);

  function handleSubmit() {
    if (!text.trim() || disabled) return;
    onSubmit(text);
  }

  function handleCancel() {
    onCancel();
  }

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      if (isStreaming) {
        handleCancel();
      } else {
        handleSubmit();
      }
    }
  }

  $effect(() => {
    if (textarea && text !== undefined) {
      textarea.style.height = 'auto';
      const scrollHeight = textarea.scrollHeight;
      textarea.style.height = scrollHeight + 'px';
      textarea.style.overflowY = scrollHeight > 400 ? 'auto' : 'hidden';
    }
  });
</script>

<div class="input-wrapper">
  <div class="relative-container">
    <textarea
      bind:this={textarea}
      bind:value={text}
      onkeydown={handleKeydown}
      placeholder="Ask a question about your technical manuals..."
      rows="1"
      disabled={disabled}
      class="query-textarea"
    ></textarea>

    <button
      onclick={isStreaming ? handleCancel : handleSubmit}
      disabled={!isStreaming && (!text.trim() || disabled)}
      class="submit-btn {isStreaming ? 'stop' : text.trim() && !disabled ? 'active' : 'disabled'}"
      aria-label={isStreaming ? 'Stop generation' : 'Submit query'}
    >
      {#if isStreaming}
        <Square size={20} />
      {:else if isLoading}
        <div class="spinner-wrapper">
          <Loader2 class="animate-spin" size={20} />
        </div>
      {:else}
        <Send size={20} />
      {/if}
    </button>
  </div>
</div>

<style>
  .input-wrapper {
    width: 100%;
  }

  .relative-container {
    position: relative;
    width: 100%;
  }

  .query-textarea {
    display: block;
    width: 100%;
    min-height: 5rem;
    max-height: 25rem;
    overflow-y: hidden;
    resize: none;
    border-radius: 0.75rem;
    border: 1px solid #374151;
    background-color: #111827;
    padding: 1rem 3.5rem 1rem 1rem;
    font-size: 1rem;
    color: white;
    box-shadow: 0 1px 2px 0 rgba(0, 0, 0, 0.05);
    font-family: inherit;
    line-height: 1.5;
    box-sizing: border-box;
  }

  .query-textarea:focus {
    outline: none;
    border-color: #3b82f6;
    box-shadow: 0 0 0 1px #3b82f6;
  }

  .query-textarea:disabled {
    background-color: #1f2937;
    color: #6b7280;
    cursor: not-allowed;
  }

  .query-textarea::placeholder {
    color: #6b7280;
  }

  .submit-btn {
    position: absolute;
    top: 50%;
    right: 0.625rem;
    transform: translateY(-50%);
    border-radius: 0.5rem;
    padding: 0.5rem;
    color: white;
    transition: background-color 0.2s;
    border: none;
    display: flex;
    align-items: center;
    justify-content: center;
    cursor: pointer;
  }

  .submit-btn.active {
    background-color: #2563eb;
  }

  .submit-btn.active:hover {
    background-color: #1d4ed8;
  }

  .submit-btn.disabled {
    background-color: #374151;
    cursor: not-allowed;
    color: #9ca3af;
  }

  .submit-btn.stop {
    background-color: #dc2626;
  }

  .submit-btn.stop:hover {
    background-color: #b91c1c;
  }

  .spinner-wrapper {
    animation: spin 1s linear infinite;
    display: flex;
  }

  @keyframes spin {
    from { transform: rotate(0deg); }
    to { transform: rotate(360deg); }
  }
</style>
