<script lang="ts">
  import { Upload, FileText, CheckCircle, AlertCircle, X } from '@lucide/svelte';
  import { api } from '$lib/api';
  import { documentProgressState } from '$lib/stores/document-progress.svelte';
  import { errorDialog } from '$lib/stores/error-dialog.svelte';
  import { errorMessage, isAppError, isTransientError } from '$lib/error';

  let { onUploadComplete }: { onUploadComplete?: () => void } = $props();

  let isDragging = $state(false);
  let isUploading = $state(false);
  let fileInput: HTMLInputElement;

  // Track each selected file with its upload status
  interface FileStatus {
    id: number;
    file: File;
    status: 'pending' | 'uploading' | 'success' | 'error' | 'rejected';
    errorMsg?: string;
  }

  // Files are staged together and uploaded sequentially to respect queue backpressure.
  let files: FileStatus[] = $state([]);
  let nextFileId = 0;

  function handleDragOver(e: DragEvent) {
    e.preventDefault();
    if (!isUploading) isDragging = true;
  }

  function handleDragLeave() {
    isDragging = false;
  }

  function handleDrop(e: DragEvent) {
    e.preventDefault();
    isDragging = false;
    if (isUploading) return;

    const droppedFiles = e.dataTransfer?.files;
    if (droppedFiles && droppedFiles.length > 0) {
      addFiles(Array.from(droppedFiles));
    }
  }

  function handleFileSelect(e: Event) {
    const target = e.target as HTMLInputElement;
    const selectedFiles = target.files ? Array.from(target.files) : [];
    target.value = '';

    if (isUploading || selectedFiles.length === 0) return;
    addFiles(selectedFiles);
  }

  function addFiles(newFiles: File[]) {
    if (isUploading) return;

    const maxFileSize = 50 * 1024 * 1024;
    const oversizedFiles = newFiles.filter((file) => file.size > maxFileSize);
    if (oversizedFiles.length > 0) {
      errorDialog.report({
        dedupeKey: 'upload-file-size',
        title: 'Files cannot be uploaded',
        message: `${oversizedFiles.length} selected file${oversizedFiles.length === 1 ? '' : 's'} exceed${oversizedFiles.length === 1 ? 's' : ''} Ragtag’s 50 MB upload limit.`,
        guidance: 'Remove the oversized files and try again.'
      });
    }

    const newFileStatuses = newFiles.map((file) => {
      if (file.size > maxFileSize) {
        return {
          id: nextFileId++,
          file,
          status: 'rejected' as const,
          errorMsg: 'File exceeds the 50 MB limit'
        };
      }

      return { id: nextFileId++, file, status: 'pending' as const };
    });

    files = [...files, ...newFileStatuses];
  }

  function removeFile(id: number) {
    if (isUploading) return;
    files = files.filter((file) => file.id !== id);
  }

  function isUploadCandidate(file: FileStatus) {
    return file.status === 'pending' || file.status === 'error';
  }

  function updateUploadStatus(
    id: number,
    status: Extract<FileStatus['status'], 'success' | 'error' | 'rejected'>,
    errorMsg: string
  ) {
    files = files.map((file) => file.id === id ? { ...file, status, errorMsg } : file);
  }

  async function uploadAll() {
    const uploadTargets = files
      .filter(isUploadCandidate)
      .map(({ id, file }) => ({ id, file }));
    if (uploadTargets.length === 0) return;

    isUploading = true;

    files = files.map(file =>
      isUploadCandidate(file) ? { ...file, status: 'uploading' } : file
    );

    let successCount = 0;

    for (const target of uploadTargets) {
      try {
        await api.uploadDocument(target.file, (progress) => documentProgressState.record(progress));
        updateUploadStatus(target.id, 'success', 'Queued');
        successCount++;
      } catch (error) {
        const appErr = isAppError(error) ? error : null;
        const rejected = appErr
          ? appErr.code === 'invalid_input' || appErr.code === 'duplicate_document'
          : errorMessage(error).startsWith('Invalid input:') || errorMessage(error).startsWith('A document with this content already exists:');
        updateUploadStatus(target.id, rejected ? 'rejected' : 'error', rejected ? 'Upload rejected' : 'Upload did not start');
        errorDialog.report({
          dedupeKey: rejected ? 'document-upload-rejected' : `document-upload-${target.file.name}`,
          title: appErr?.operation ?? (rejected ? 'File cannot be uploaded' : 'Document upload failed'),
          message: appErr?.detail ?? (rejected
            ? 'Ragtag could not accept this file as a PDF, DOCX, or UTF-8 text document.'
            : 'Ragtag could not add this document to the processing queue.'),
          guidance: appErr?.recovery ?? (rejected
            ? 'Choose a valid PDF, DOCX, or UTF-8 text file and try again.'
            : 'Check that Ragtag can access its local data directory, then try again.'),
          code: appErr?.code,
          transient: isTransientError(error),
        });
      }
    }
    if (successCount > 0) {
      onUploadComplete?.();
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
      multiple
      accept=".pdf,.docx,text/*,.md,.markdown,.rst,.csv,.tsv,.json,.yaml,.yml,.toml,.xml,.html,.htm,.log,.ini,.cfg,.conf,.properties"
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
        <p class="secondary-text">PDF, DOCX, or UTF-8 text files (max 50 MB)</p>
      </div>
    </div>
  </div>

  <!-- File List -->
  {#if files.length > 0}
    <div class="file-list">
      {#each files as file (file.id)}
        <div class="file-item">
          
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
                type="button"
                class="remove-btn"
                onclick={() => removeFile(file.id)}
                disabled={isUploading}
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
        disabled={isUploading || !files.some(isUploadCandidate)}
      >
        {#if isUploading}
          <div class="spinner-xs"></div>
          Uploading...
        {:else}
          <Upload size={16} />
          Upload {files.filter(isUploadCandidate).length} Files
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
    white-space: normal;
    word-break: break-word;
  }

  .file-status.success { color: #4ade80; }
  .file-status.error { color: #f87171; }
  .file-status.rejected { color: #f87171; }

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
