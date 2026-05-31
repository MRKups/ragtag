<script lang="ts">
  import { createEventDispatcher } from 'svelte';
  import { Upload, FileText, CheckCircle, AlertCircle, X } from 'lucide-svelte';
  import { api } from '$lib/api';

  const dispatch = createEventDispatcher();

  let isDragging = $state(false);
  let isUploading = $state(false);
  let fileInput: HTMLInputElement;

  // Track each selected file with its upload status
  interface FileStatus {
    file: File;
    status: 'pending' | 'uploading' | 'success' | 'error';
    progress: number;
    errorMsg?: string;
  }

  // We only handle one file at a time, but UI can still list what's selected
  let files: FileStatus[] = $state([]);

  function handleDragOver(e: DragEvent) {
    e.preventDefault();
    isDragging = true;
  }

  function handleDragLeave() {
    isDragging = false;
  }

  async function handleDrop(e: DragEvent) {
    e.preventDefault();
    isDragging = false;
    const droppedFiles = e.dataTransfer?.files;
    if (droppedFiles && droppedFiles.length > 0) {
      addFiles(Array.from(droppedFiles));
    }
  }

  async function handleFileSelect(e: Event) {
    const target = e.target as HTMLInputElement;
    if (target.files && target.files.length > 0) {
      addFiles(Array.from(target.files));
    }
    target.value = '';
  }

  function addFiles(newFiles: File[]) {
    const allowedExtensions = ['.pdf']; // Currently only PDF
    const maxFileSize = 50 * 1024 * 1024; // 50MB

    const newFileStatuses = newFiles.map(f => {
      const name = f.name.toLowerCase();
      const hasValidExtension = allowedExtensions.some(ext => name.endsWith(ext));

      if (!hasValidExtension) {
        return { file: f, status: 'error' as const, progress: 0, errorMsg: 'Unsupported file type' };
      }
      if (f.size > maxFileSize) {
        return { file: f, status: 'error' as const, progress: 0, errorMsg: 'File too large (>50MB)' };
      }

      return { file: f, status: 'pending' as const, progress: 0 };
    });

    files = [...files, ...newFileStatuses];
  }

  function removeFile(index: number) {
    files = files.filter((_, i) => i !== index);
  }

  async function uploadAll() {
    const pendingFiles = files.filter(f => f.status === 'pending' || f.status === 'error');
    if (pendingFiles.length === 0) return;

    isUploading = true;
    
    files = files.map(f => 
      (f.status === 'pending' || f.status === 'error') ? { ...f, status: 'uploading' } : f
    );

    let successCount = 0;

    for (let i = 0; i < files.length; i++) {
      if (files[i].status !== 'uploading') continue;
      
      try {
        await api.uploadDocument(files[i].file);
        files[i] = { ...files[i], status: 'success', errorMsg: 'Uploaded' };
        successCount++;
      } catch (e) {
        files[i] = { ...files[i], status: 'error', errorMsg: e instanceof Error ? e.message : 'Upload failed' };
      }
    }

    if (successCount > 0) {
      dispatch('uploadComplete');
    }

    isUploading = false;
  }
</script>

<div class="upload-container">
  <h2 class="title">
    <Upload size={20} />
    Upload Manuals
  </h2>

  <!-- Drop Zone -->
  <div
    role="button"
    tabindex="0"
    class="drop-zone {isDragging ? 'dragging' : ''} {isUploading ? 'disabled' : ''}"
    ondragover={handleDragOver}
    ondragleave={handleDragLeave}
    ondrop={handleDrop}
    onclick={() => !isUploading && fileInput.click()}
    onkeydown={(e) => e.key === 'Enter' && !isUploading && fileInput.click()}
  >
    <input
      bind:this={fileInput}
      type="file"
      accept=".pdf"
      multiple
      class="hidden"
      onchange={handleFileSelect}
      disabled={isUploading}
    />

    <div class="drop-content">
      <div class="icon-circle">
        <FileText size={24} color="#9ca3af" />
      </div>
      <div class="text-content">
        <p class="primary-text">Click to upload or drag and drop</p>
        <p class="secondary-text">PDF (max 50MB)</p>
      </div>
    </div>
  </div>

  <!-- File List -->
  {#if files.length > 0}
    <div class="file-list">
      {#each files as file, i}
        <div class="file-item">
          <FileText size={20} color="#9ca3af" class="file-icon" />
          
          <div class="file-info">
            <p class="file-name" title={file.file.name}>
              {file.file.name}
            </p>
            {#if file.errorMsg}
              <p class="file-status {file.status}">
                {file.errorMsg}
              </p>
            {:else}
              <p class="file-size">
                {(file.file.size / 1024 / 1024).toFixed(2)} MB
              </p>
            {/if}
          </div>

          <div class="file-action">
            {#if file.status === 'uploading'}
              <div class="spinner-sm"></div>
            {:else if file.status === 'success'}
              <CheckCircle size={20} color="#22c55e" />
            {:else if file.status === 'error'}
              <AlertCircle size={20} color="#ef4444" />
            {:else}
              <button 
                class="remove-btn"
                onclick={(e) => { e.stopPropagation(); removeFile(i); }}
              >
                <X size={20} />
              </button>
            {/if}
          </div>
        </div>
      {/each}
    </div>

    <div class="actions">
      <button
        class="upload-btn"
        onclick={uploadAll}
        disabled={isUploading || !files.some(f => f.status === 'pending' || f.status === 'error')}
      >
        {#if isUploading}
          <div class="spinner-xs"></div>
          Uploading...
        {:else}
          <Upload size={16} />
          Upload {files.filter(f => f.status === 'pending' || f.status === 'error').length} Files
        {/if}
      </button>
    </div>
  {/if}
</div>

<style>
  .upload-container {
    height: 100%;
    display: flex;
    flex-direction: column;
    box-sizing: border-box;
  }

  .title {
    font-size: 1.25rem;
    font-weight: 600;
    color: white;
    margin: 0 0 1.5rem 0;
    display: flex;
    align-items: center;
    gap: 0.75rem;
    padding-bottom: 1rem;
    border-bottom: 1px solid #374151;
  }

  .hidden {
    display: none;
  }

  /* Drop Zone */
  .drop-zone {
    position: relative;
    border: 2px dashed #4b5563;
    border-radius: 0.75rem;
    padding: 3rem 2rem;
    text-align: center;
    transition: all 0.2s ease-in-out;
    cursor: pointer;
    margin-bottom: 1.5rem;
    background-color: rgba(31, 41, 55, 0.5);
  }

  .drop-zone:hover {
    border-color: #6b7280;
  }

  .drop-zone.dragging {
    border-color: #3b82f6;
    background-color: rgba(30, 58, 138, 0.2);
  }

  .drop-zone.disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .drop-content {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 0.75rem;
  }

  .icon-circle {
    padding: 0.75rem;
    background-color: #374151;
    border-radius: 9999px;
  }

  .text-content {
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
  }

  .primary-text {
    font-size: 0.875rem;
    font-weight: 500;
    color: #e5e7eb;
    margin: 0;
  }

  .secondary-text {
    font-size: 0.75rem;
    color: #6b7280;
    margin: 0;
  }

  /* File List */
  .file-list {
    flex: 1;
    overflow-y: auto;
    min-height: 0;
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
    margin-bottom: 1rem;
    padding-right: 0.5rem;
  }

  .file-item {
    background-color: rgba(55, 65, 81, 0.5);
    border-radius: 0.5rem;
    padding: 0.75rem;
    display: flex;
    align-items: center;
    gap: 0.75rem;
  }

  .file-icon {
    flex-shrink: 0;
  }

  .file-info {
    flex: 1;
    min-width: 0;
  }

  .file-name {
    font-size: 0.875rem;
    color: #e5e7eb;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    margin: 0;
  }

  .file-size {
    font-size: 0.75rem;
    color: #6b7280;
    margin: 0;
  }

  .file-status {
    font-size: 0.75rem;
    margin: 0;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .file-status.success { color: #4ade80; }
  .file-status.error { color: #f87171; }

  .file-action {
    flex-shrink: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    width: 20px;
    height: 20px;
  }

  .remove-btn {
    background: none;
    border: none;
    padding: 0;
    color: #6b7280;
    cursor: pointer;
    display: flex;
    align-items: center;
    justify-content: center;
    transition: color 0.2s;
  }

  .remove-btn:hover {
    color: #f87171;
  }

  /* Actions */
  .actions {
    margin-top: auto;
    display: flex;
    flex-direction: column;
    gap: 1rem;
  }

  .upload-btn {
    width: 100%;
    padding: 0.5rem 1rem;
    background-color: #2563eb;
    color: white;
    border: none;
    border-radius: 0.5rem;
    font-weight: 500;
    cursor: pointer;
    transition: background-color 0.2s;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 0.5rem;
  }

  .upload-btn:hover:not(:disabled) {
    background-color: #1d4ed8;
  }

  .upload-btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  /* Spinners */
  .spinner-sm {
    width: 1.25rem;
    height: 1.25rem;
    border: 2px solid #3b82f6;
    border-top-color: transparent;
    border-radius: 50%;
    animation: spin 1s linear infinite;
  }

  .spinner-xs {
    width: 1rem;
    height: 1rem;
    border: 2px solid white;
    border-top-color: transparent;
    border-radius: 50%;
    animation: spin 1s linear infinite;
  }

  @keyframes spin {
    to { transform: rotate(360deg); }
  }
</style>
