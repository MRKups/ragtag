import { invoke, Channel } from '@tauri-apps/api/core';
import type { Document, TokenEvent } from './types';

export const api = {
  beginFirstRunInit: () => invoke<void>('begin_first_run_init'),
  
  uploadDocument: async (file: File): Promise<string> => {
    const arrayBuffer = await file.arrayBuffer();
    const bytes = new Uint8Array(arrayBuffer);
    
    return invoke<string>('upload_document', bytes, {
      headers: {
        'Content-Type': 'application/octet-stream',
        'X-Filename': file.name,
      }
    });
  },
  
  executeQuery: (query: string, onEvent: (e: TokenEvent) => void): Promise<void> => {
    const channel = new Channel<TokenEvent>();
    channel.onmessage = (event) => onEvent(event);
    
    return invoke<void>('execute_query', {
      query,
      channel
    });
  },
  
  listDocuments: () => invoke<Document[]>('list_documents'),
  
  deleteDocument: (id: string) => invoke<void>('delete_document', { id }),
  
  getSettings: () => invoke<Record<string, string>>('get_settings'),
  
  setSetting: (key: string, value: string) => invoke<void>('set_setting', { key, value }),

  setApiKey: (key: string) => invoke<boolean>('set_api_key', { key }),

  hasApiKey: () => invoke<boolean>('has_api_key'),

  deleteApiKey: () => invoke<boolean>('delete_api_key'),

  getStorageInfo: () => invoke<{
    database_bytes: number;
    vector_store_bytes: number;
    documents_bytes: number;
    models_bytes: number;
  }>('get_storage_info'),

  compactVectorStore: () => invoke<{ status: string }>('compact_vector_store'),

  vacuumDatabase: () => invoke<{ status: string }>('vacuum_database'),

  resetAllData: (deleteModels: boolean) => invoke<{ requires_restart: boolean }>('reset_all_data', { deleteModels })
};
