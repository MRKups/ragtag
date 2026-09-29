<script lang="ts">
    import DocumentUpload from "$lib/components/DocumentUpload.svelte";
    import DocumentList from "$lib/components/DocumentList.svelte";

    let documentList: DocumentList;
</script>

<div class="page-container">
    <div class="panel">
        <div class="panel-content">
            <DocumentUpload onUploadComplete={() => documentList?.refresh()} />
        </div>
    </div>
    <div class="panel">
        <div class="panel-content">
            <DocumentList bind:this={documentList} />
        </div>
    </div>
</div>

<style>
    .page-container {
        display: flex;
        width: 100%;
        height: 100%;
        background-color: var(--bg-app, #121214);
    }

    .panel {
        width: 50%;
        height: 100%;
        overflow: hidden;
    }

    .panel:first-child {
        border-right: 1px solid var(--border-color, #27272a);
        background-color: var(--bg-panel, #121214);
    }

    .panel:last-child {
        background-color: var(--bg-panel-alt, #18181b);
    }

    .panel-content {
        padding: 2rem;
        height: 100%;
        overflow-y: auto;
    }
</style>
