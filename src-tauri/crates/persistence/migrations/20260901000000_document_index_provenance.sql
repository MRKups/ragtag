ALTER TABLE documents ADD COLUMN indexed_extraction_version TEXT;
ALTER TABLE documents ADD COLUMN indexed_chunk_size INTEGER;
ALTER TABLE documents ADD COLUMN indexed_chunk_overlap INTEGER;
DELETE FROM app_settings WHERE key = 'extraction_version';
