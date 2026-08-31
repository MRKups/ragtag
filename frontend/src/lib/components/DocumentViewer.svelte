<script lang="ts">
  import { FileText } from '@lucide/svelte';
  import { api } from '$lib/api';
  import PdfViewer from './PdfViewer.svelte';

  let {
    filename = null as string | null,
    storagePath = null as string | null,
    page = 1,
    citedPages = [] as number[]
  } = $props<{
    filename?: string | null;
    storagePath?: string | null;
    page?: number;
    citedPages?: number[];
  }>();

  let pdfUrl = $derived.by(() => {
    if (storagePath) {
      // Tauri v2 asset protocol: convertFileSrc builds the correct URL
      // The assetProtocol in tauri.conf.json scopes to $APPLOCALDATA
      return api.assetUrl(storagePath);
    }
    return null;
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
  {:else if pdfUrl}
    <PdfViewer url={pdfUrl} {page} {citedPages} />
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
</style>
