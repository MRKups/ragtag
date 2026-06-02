<script lang="ts">
  import { FileText, BookOpen } from 'lucide-svelte';
  import type { Citation } from '$lib/types';

  let {
    citations = [] as Citation[],
    onCitationClick = (_citation: { filename: string; storagePath: string; documentId: string; page: number }) => {}
  } = $props<{
    citations?: Citation[];
    onCitationClick?: (citation: { filename: string; storagePath: string; documentId: string; page: number }) => void;
  }>();

  function handleClick(citation: Citation) {
    onCitationClick({
      filename: citation.filename,
      storagePath: citation.storage_path,
      documentId: citation.document_id,
      page: citation.page
    });
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
    border-top: 1px solid #374151;
  }

  .citations-title {
    font-size: 0.875rem;
    font-weight: 600;
    color: #6b7280;
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
    grid-template-columns: 1fr;
  }

  .citation-card {
    background: #111827;
    border: 1px solid #374151;
    border-radius: 0.5rem;
    padding: 0.75rem;
    font-size: 0.875rem;
    transition: all 0.2s;
    cursor: pointer;
    color: inherit;
    font-family: inherit;
    text-align: left;
    width: 100%;
    line-height: inherit;
  }

  .citation-card:hover {
    background: #1f2937;
    border-color: #4b5563;
  }

  .citation-header {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    margin-bottom: 0.5rem;
  }

  .citation-number {
    font-weight: bold;
    color: #60a5fa;
    background: #1e3a8a;
    padding: 0 0.4rem;
    border-radius: 0.25rem;
    font-size: 0.75rem;
  }

  .citation-source {
    font-weight: 500;
    color: #e5e7eb;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    display: flex;
    align-items: center;
    gap: 0.25rem;
  }

  .citation-details {
    display: flex;
    gap: 0.5rem;
    margin-bottom: 0.5rem;
  }

  .badge {
    background: #374151;
    color: #d1d5db;
    padding: 0.1rem 0.4rem;
    border-radius: 0.25rem;
    font-size: 0.75rem;
  }

  .citation-text {
    color: #9ca3af;
    font-style: italic;
    line-height: 1.4;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
    margin: 0;
  }
</style>
