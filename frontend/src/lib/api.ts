import { invoke, Channel } from '@tauri-apps/api/core';
import type { Document, TokenEvent } from './types';

export const api = {
  ping: () => invoke<string>('ping'),
  
  uploadDocument: async (file: File): Promise<string> => {
    const arrayBuffer = await file.arrayBuffer();
    const bytes = Array.from(new Uint8Array(arrayBuffer));
    
    return invoke<string>('upload_document', {
      filename: file.name,
      fileBytes: bytes
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
  
  setSetting: (key: string, value: string) => invoke<void>('set_setting', { key, value })
};
