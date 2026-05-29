CREATE TABLE documents (
    id TEXT PRIMARY KEY,
    
    -- File info
    filename TEXT NOT NULL,
    file_type TEXT NOT NULL,
    file_size_bytes BIGINT,
    file_hash BLOB,
    storage_path TEXT NOT NULL,
    
    -- Processing state
    status TEXT NOT NULL DEFAULT 'queued',     
    
    -- Indexing metadata
    chunk_count INTEGER,
    embedding_model TEXT,
    embedding_attempts INTEGER DEFAULT 0,
    
    -- Timestamps
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    processing_started_at DATETIME,
    indexed_at DATETIME,
    
    -- Error tracking
    error_message TEXT,
    retry_count INTEGER DEFAULT 0
);

CREATE INDEX idx_documents_status ON documents(status);
CREATE UNIQUE INDEX idx_documents_file_hash ON documents(file_hash);
CREATE INDEX idx_documents_created ON documents(created_at DESC);

CREATE TABLE app_settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
