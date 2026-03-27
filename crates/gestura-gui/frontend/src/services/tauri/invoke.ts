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

const isRecord = (value: unknown): value is Record<string, unknown> =>
  Boolean(value) && typeof value === 'object' && !Array.isArray(value);

const extractNestedMessage = (value: unknown): string | null => {
  if (typeof value === 'string' && value.trim()) return value.trim();
  if (value instanceof Error) {
    const nested = value.message.trim();
    if (nested) return nested;
  }
  if (isRecord(value)) {
    for (const key of ['message', 'error', 'reason']) {
      const candidate = value[key];
      if (typeof candidate === 'string' && candidate.trim()) return candidate.trim();
    }
  }
  return null;
};

const getCause = (value: unknown): unknown =>
  isRecord(value) ? value.cause : undefined;

const normalizeErrorMessage = (err: unknown): string => {
  if (err instanceof Error) {
    const directMessage = err.message.trim();
    if (directMessage) return directMessage;

    const nestedMessage = extractNestedMessage(getCause(err));
    if (nestedMessage) return nestedMessage;

    if (err.name.trim()) return err.name.trim();
  }

  if (typeof err === 'string' && err.trim()) return err.trim();

  if (isRecord(err)) {
    const nestedMessage = extractNestedMessage(err);
    if (nestedMessage) return nestedMessage;

    const causeMessage = extractNestedMessage(getCause(err));
    if (causeMessage) return causeMessage;
  }

  try {
    const json = JSON.stringify(err);
    if (json && json !== '{}' && json !== '""') return json;
  } catch {
    // Fall through to string coercion below.
  }

  const stringified = String(err).trim();
  if (stringified && stringified !== '[object Object]') return stringified;

  return 'Unknown Tauri invoke error';
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
