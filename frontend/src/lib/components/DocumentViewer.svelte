<script lang="ts">
  import { FileText } from '@lucide/svelte';
  import { api } from '$lib/api';
  import type { DocumentPreview } from '$lib/types';
  import MarkdownPreview from './MarkdownPreview.svelte';
  import PdfViewer from './PdfViewer.svelte';

  let {
    documentId = null as string | null,
    filename = null as string | null,
    storagePath = null as string | null,
    page = 1,
    citedPages = [] as number[]
  } = $props<{
    documentId?: string | null;
    filename?: string | null;
    storagePath?: string | null;
    page?: number;
    citedPages?: number[];
  }>();

  let isPdf = $derived(filename?.toLowerCase().endsWith('.pdf') ?? false);
  let sourceUrl = $derived.by(() => {
    if (isPdf && storagePath) {
      // Tauri v2 asset protocol: convertFileSrc builds the correct URL.
      // The assetProtocol in tauri.conf.json scopes to $APPLOCALDATA.
      return api.assetUrl(storagePath);
    }
    return null;
  });
  let documentPreview = $state<DocumentPreview | null>(null);
  let documentPreviewFailed = $state(false);

  $effect(() => {
    if (!documentId || isPdf) {
      documentPreview = null;
      documentPreviewFailed = false;
      return;
    }

    let cancelled = false;
    documentPreview = null;
    documentPreviewFailed = false;
    void api.getDocumentPreview(documentId)
      .then((preview) => {
        if (!cancelled) documentPreview = preview;
      })
      .catch(() => {
        if (!cancelled) documentPreviewFailed = true;
      });

    return () => {
      cancelled = true;
    };
  });
</script>

<div class="viewer-container">
  {#if !filename}
    <div class="doc-placeholder">
      <div class="icon-circle">
        <FileText size={48} color="#9ca3af" />
      </div>
      <h3>Document Preview</h3>
      <p>Select a citation to view the source document.</p>
    </div>
  {:else if sourceUrl && isPdf}
    <PdfViewer url={sourceUrl} {page} {citedPages} />
  {:else if !isPdf && !documentId}
    <div class="doc-placeholder">
      <div class="icon-circle">
        <FileText size={48} color="#9ca3af" />
      </div>
      <h3>{filename}</h3>
      <p>A preview is not available for this source.</p>
    </div>
  {:else if !isPdf}
    <section class="text-preview" aria-label={`Source preview: ${filename}`}>
      <header class="text-preview-header">
        <FileText size={24} color="#9ca3af" />
        <h3>{filename}</h3>
      </header>
      {#if documentPreviewFailed}
        <p class="text-preview-status">Ragtag could not read this source preview.</p>
      {:else if documentPreview === null}
        <p class="text-preview-status">Loading source preview…</p>
      {:else if documentPreview.kind === 'plain_text'}
        <pre>{documentPreview.content}</pre>
        {#if documentPreview.truncated}
          <p class="text-preview-status">Preview is limited to the first 1 MiB of this source.</p>
        {/if}
      {:else}
        <MarkdownPreview blocks={documentPreview.blocks} />
        {#if documentPreview.truncated}
          <p class="text-preview-status">Preview is limited to the first 1 MiB of this source.</p>
        {/if}
      {/if}
    </section>
  {:else}
    <div class="doc-placeholder">
      <div class="icon-circle">
        <FileText size={48} color="#9ca3af" />
      </div>
      <h3>{filename}</h3>
      <p>Unable to locate document on disk.</p>
    </div>
  {/if}
</div>

<style>
  .viewer-container {
    width: 100%;
    height: 100%;
    display: flex;
    flex-direction: column;
    background-color: #0f172a;
  }

  .doc-placeholder {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    text-align: center;
    color: #9ca3af;
    gap: 1rem;
    height: 100%;
    padding: 2rem;
  }

  .icon-circle {
    width: 5rem;
    height: 5rem;
    border-radius: 50%;
    background-color: #1e293b;
    display: flex;
    align-items: center;
    justify-content: center;
    margin-bottom: 0.5rem;
  }

  .doc-placeholder h3 {
    color: #e5e7eb;
    font-size: 1.25rem;
    font-weight: 600;
    margin: 0;
  }

  .doc-placeholder p {
    margin: 0;
    line-height: 1.5;
  }
  .text-preview {
    flex: 1;
    min-height: 0;
    overflow: auto;
    padding: 1.5rem;
    color: #d1d5db;
  }

  .text-preview-header {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    margin-bottom: 1rem;
    color: #e5e7eb;
  }

  .text-preview-header h3 {
    margin: 0;
    font-size: 1rem;
    font-weight: 600;
  }

  .text-preview-status {
    color: #9ca3af;
  }

  .text-preview pre {
    margin: 0;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    font: 0.875rem/1.6 ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace;
  }
</style>
