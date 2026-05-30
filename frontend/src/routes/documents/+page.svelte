<script lang="ts">
    import DocumentUpload from "$lib/components/DocumentUpload.svelte";
    import DocumentList from "$lib/components/DocumentList.svelte";

    let documentList: DocumentList;
</script>

<div class="page-container">
    <div class="panel">
        <div class="panel-content">
            <DocumentUpload on:uploadComplete={() => documentList?.refresh()} />
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
    }

    .panel {
        width: 50%;
        height: 100%;
        overflow: hidden;
    }

    .panel:first-child {
        border-right: 1px solid #374151;
    }
    
    .panel-content {
        padding: 2rem;
        height: 100%;
        overflow-y: auto;
    }
</style>
