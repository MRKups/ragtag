<script lang="ts">
  import { FileText, BookOpen } from '@lucide/svelte';
  import type { Citation } from '$lib/types';
  import { citationSelection, type CitationSelection } from '$lib/citations';
  import ConfidenceBadge from './ConfidenceBadge.svelte';

  let {
    citations = [] as Citation[],
    onCitationClick = (_citation: CitationSelection) => {}
  } = $props<{
    citations?: Citation[];
    onCitationClick?: (citation: CitationSelection) => void;
  }>();

  function handleClick(citation: Citation) {
    onCitationClick(citationSelection(citation));
  }
</script>

{#if citations.length > 0}
  <div class="citations-container">
    <h3 class="citations-title">
      <BookOpen size={16} />
      Sources
    </h3>
    <div class="citations-grid">
      {#each citations as citation, i}
        <button class="citation-card citation-btn" onclick={() => handleClick(citation)}>
          <div class="citation-header">
            <span class="citation-number">[{i + 1}]</span>
            <span class="citation-source" title={citation.filename}>
              <FileText size={14} class="source-icon" />
              {citation.filename}
            </span>
          </div>
          <div class="citation-details">
            <span class="badge">Page {citation.page}</span>
            {#if !citation.is_hybrid}
              <ConfidenceBadge score={citation.score} />
            {/if}
          </div>
          <p class="citation-text">"{citation.chunk_content}"</p>
        </button>
      {/each}
    </div>
  </div>
{/if}

<style>
  .citations-container {
    margin-top: 2rem;
    padding-top: 1.5rem;
    border-top: 1px solid var(--border-color, #27272a);
  }

  .citations-title {
    font-size: 0.875rem;
    font-weight: 600;
    color: var(--text-muted, #71717a);
    text-transform: uppercase;
    letter-spacing: 0.05em;
    margin-bottom: 0.75rem;
    display: flex;
    align-items: center;
    gap: 0.5rem;
  }

  .citations-grid {
    display: grid;
    gap: 0.75rem;
    grid-template-columns: minmax(0, 1fr);
    width: 100%;
    min-width: 0;
  }

  .citation-card {
    background: var(--citation-card-bg, #18181b);
    border: 1px solid var(--border-card, #2e2e33);
    border-radius: 0.5rem;
    padding: 0.75rem;
    font-size: 0.875rem;
    transition: all 0.2s;
    cursor: pointer;
    color: inherit;
    font-family: inherit;
    text-align: left;
    width: 100%;
    min-width: 0;
    max-width: 100%;
    box-sizing: border-box;
    overflow: hidden;
    line-height: inherit;
  }

  .citation-card:hover {
    background: var(--citation-card-hover, #26262b);
    border-color: var(--border-card-hover, #3f3f46);
  }

  .citation-header {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    margin-bottom: 0.5rem;
    min-width: 0;
  }

  .citation-number {
    font-weight: bold;
    color: var(--citation-badge-text, #60a5fa);
    background: var(--citation-badge-bg, rgba(59, 130, 246, 0.15));
    padding: 0 0.4rem;
    border-radius: 0.25rem;
    font-size: 0.75rem;
    flex-shrink: 0;
  }

  .citation-source {
    font-weight: 500;
    color: var(--text-primary, #f4f4f5);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    display: flex;
    align-items: center;
    gap: 0.25rem;
    min-width: 0;
  }

  .citation-details {
    display: flex;
    gap: 0.5rem;
    margin-bottom: 0.5rem;
    min-width: 0;
  }

  .badge {
    background: var(--border-color, #27272a);
    color: var(--text-primary, #f4f4f5);
    padding: 0.1rem 0.4rem;
    border-radius: 0.25rem;
    font-size: 0.75rem;
  }

  .citation-text {
    color: var(--text-secondary, #9ca3af);
    font-style: italic;
    line-height: 1.4;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
    overflow-wrap: anywhere;
    word-break: break-word;
    max-width: 100%;
    margin: 0;
  }
</style>
