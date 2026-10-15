// Coordinates Tauri IPC commands and event listeners across the native desktop boundary (DEV-06).
import { invoke, Channel, convertFileSrc } from '@tauri-apps/api/core';
import { listen, type Event, type UnlistenFn } from '@tauri-apps/api/event';
import type {
  AppErrorPayload,
  ApiKeyStatus,
  AppLifecycleState,
  Document,
  DocumentPreview,
  ExternalProviderConnection,
  IngestionProgress,
  Platform,
  StorageInfo,
  TokenEvent
} from './types';

export const api = {
  beginFirstRunInit: () => invoke<void>('begin_first_run_init'),
  retryLocalModelInitialization: () => invoke<void>('retry_local_model_initialization'),
  getAppState: () => invoke<AppLifecycleState>('get_app_state'),
  cancelModelDownload: () => invoke<void>('cancel_model_download'),
  
  uploadDocument: (file: File, onProgress: (progress: IngestionProgress) => void): Promise<string> => {
    const channel = new Channel<IngestionProgress>();
    channel.onmessage = onProgress;

    return file.arrayBuffer().then((buffer) =>
      invoke<string>('upload_document', {
        filename: file.name,
        fileBytes: new Uint8Array(buffer),
        channel,
      })
    );
  },
  executeQuery: (query: string, requestId: string, onEvent: (e: TokenEvent) => void): Promise<void> => {
    const channel = new Channel<TokenEvent>();
    channel.onmessage = (event) => onEvent(event);
    
    return invoke<void>('execute_query', {
      query,
      requestId,
      channel
    });
  },

  cancelQuery: (requestId: string) => invoke<void>('cancel_query', { requestId }),

  listen<T>(event: string, handler: (event: Event<T>) => void): Promise<UnlistenFn> {
    return listen<T>(event, handler);
  },

  listenExternalInferenceFailed: (
    handler: (event: Event<AppErrorPayload>) => void
  ): Promise<UnlistenFn> => listen<AppErrorPayload>('external_inference_failed', handler),

  assetUrl: (path: string) => convertFileSrc(path),
  
  listDocuments: () => invoke<Document[]>('list_documents'),
  getDocumentPreview: (documentId: string) =>
    invoke<DocumentPreview>('get_document_preview', { documentId }),
  deleteDocument: (id: string) => invoke<void>('delete_document', { id }),
  
  getSettings: () => invoke<Record<string, string>>('get_settings'),
  
  setSetting: (key: string, value: string) => invoke<void>('set_setting', { key, value }),

  setApiKey: (key: string) => invoke<ApiKeyStatus>('set_api_key', { key }),

  testExternalProvider: (baseUrl: string, apiKey: string | null) =>
    invoke<ExternalProviderConnection>('test_external_provider', { baseUrl, apiKey }),

  setExternalProviderConfig: (baseUrl: string, model: string) =>
    invoke<void>('set_external_provider_config', { baseUrl, model }),

  getApiKeyStatus: () => invoke<ApiKeyStatus>('get_api_key_status'),

  deleteApiKey: () => invoke<boolean>('delete_api_key'),

  getStorageInfo: () => invoke<StorageInfo>('get_storage_info'),

  compactVectorStore: () => invoke<{ status: string }>('compact_vector_store'),

  vacuumDatabase: () => invoke<{ status: string }>('vacuum_database'),

  resetAllData: (deleteModels: boolean) => invoke<{ restarting: boolean }>('reset_all_data', { deleteModels }),

  getPlatform: () => invoke<Platform>('get_platform'),

  openExternalUrl: (url: string) => invoke<void>('open_external_url', { url })
};
