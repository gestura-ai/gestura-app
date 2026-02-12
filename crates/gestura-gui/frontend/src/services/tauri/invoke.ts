import { invoke as tauriInvoke } from '@tauri-apps/api/core';

/**
 * Error thrown by {@link invokeTauri}.
 *
 * Note: do not log args here; some payloads can contain secrets (e.g. API keys in config).
 */
export class TauriInvokeError extends Error {
  public readonly command: string;
  public readonly cause: unknown;

  public constructor(command: string, message: string, cause: unknown) {
    super(message);
    this.name = 'TauriInvokeError';
    this.command = command;
    this.cause = cause;
  }
}

const normalizeErrorMessage = (err: unknown): string => {
  if (err instanceof Error && typeof err.message === 'string' && err.message.trim()) {
    return err.message;
  }
  if (typeof err === 'string' && err.trim()) return err;
  try {
    return JSON.stringify(err);
  } catch {
    return String(err);
  }
};

/**
 * Central wrapper for Tauri IPC.
 *
 * Phase 2.1: provide one place to normalize errors and later add typing per command.
 */
export const invokeTauri = async <R>(command: string, args?: Record<string, unknown>): Promise<R> => {
  try {
    return await tauriInvoke<R>(command, args);
  } catch (err) {
    const message = normalizeErrorMessage(err);
    // Intentionally omit args from logs.
    console.error(`[tauri.invoke] ${command} failed: ${message}`);
    throw new TauriInvokeError(command, message, err);
  }
};
