<script lang="ts">
  import { marked } from 'marked';
  import DOMPurify from 'dompurify';
  import type { Citation } from '$lib/types';
  import CitationList from './CitationList.svelte';

  let {
    text = '',
    citations = [] as Citation[],
    isStreaming = false,
    error = null as string | null,
    onCitationClick = (_citation: { filename: string; storagePath: string; documentId: string; page: number }) => {}
  } = $props<{
    text?: string;
    citations?: Citation[];
    isStreaming?: boolean;
    error?: string | null;
    onCitationClick?: (citation: { filename: string; storagePath: string; documentId: string; page: number }) => void;
  }>();

  let contentEl: HTMLDivElement | null = $state(null);

  let renderedHtml = $derived.by(() => {
    if (!text) return '';
    const parsedHtml = marked.parse(text) as string;
    return DOMPurify.sanitize(parsedHtml);
  });

  $effect(() => {
    if (contentEl) {
      contentEl.scrollTop = contentEl.scrollHeight;
    }
  });

</script>

<div class="result-container" class:has-content={!!text || isStreaming}>
  {#if error}
    <div class="error-box">
      <strong>Error:</strong> {error}
    </div>
  {/if}

  {#if text || isStreaming}
    <div class="prose" bind:this={contentEl}>
      {@html renderedHtml}
      {#if isStreaming}
        <span class="cursor-blink">|</span>
      {/if}
    </div>
  {/if}

  {#if citations.length > 0 && !isStreaming}
    <CitationList {citations} {onCitationClick} />
  {/if}
</div>

<style>
  .result-container {
    width: 100%;
  }

  .result-container.has-content {
    background: #1f2937;
    border-radius: 1rem;
    box-shadow: 0 4px 6px -1px rgba(0, 0, 0, 0.3), 0 2px 4px -1px rgba(0, 0, 0, 0.1);
    padding: 2rem;
    border: 1px solid #374151;
    box-sizing: border-box;
  }

  .prose {
    color: #d1d5db;
    line-height: 1.7;
    font-size: 1.05rem;
    overflow-wrap: break-word;
    word-break: break-word;
    max-height: 60vh;
    overflow-y: auto;
  }

  .prose :global(p) {
    margin-bottom: 1.25em;
    margin-top: 0;
  }

  .prose :global(ul), .prose :global(ol) {
    margin-bottom: 1.25em;
    padding-left: 1.5em;
    margin-top: 0;
  }

  .prose :global(li) {
    margin-bottom: 0.5em;
  }

  .prose :global(strong) {
    color: #f9fafb;
    font-weight: 600;
  }

  .prose :global(h1), .prose :global(h2), .prose :global(h3), .prose :global(h4) {
    color: #f3f4f6;
    font-weight: 600;
    margin-top: 1.5em;
    margin-bottom: 0.75em;
  }

  .prose :global(code) {
    background-color: #374151;
    padding: 0.2em 0.4em;
    border-radius: 0.25em;
    font-family: monospace;
    font-size: 0.9em;
  }

  .prose :global(pre) {
    background-color: #111827;
    padding: 1em;
    border-radius: 0.5em;
    overflow-x: auto;
    margin-bottom: 1.25em;
    border: 1px solid #374151;
  }

  .prose :global(pre code) {
    background-color: transparent;
    padding: 0;
    border-radius: 0;
    color: #e5e7eb;
  }

  .cursor-blink {
    animation: blink 1s step-end infinite;
    color: #3b82f6;
    font-weight: bold;
  }

  @keyframes blink {
    50% { opacity: 0; }
  }

  .error-box {
    padding: 1rem;
    background-color: #450a0a;
    border: 1px solid #7f1d1d;
    border-radius: 0.5rem;
    color: #fca5a5;
    font-size: 0.9rem;
    margin-bottom: 1rem;
  }

  .prose::-webkit-scrollbar {
    width: 6px;
  }

  .prose::-webkit-scrollbar-track {
    background: transparent;
  }

  .prose::-webkit-scrollbar-thumb {
    background: #475569;
    border-radius: 3px;
  }
</style>
