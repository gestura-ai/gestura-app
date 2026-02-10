import { useCallback, useEffect, useRef, useState } from 'react';

export interface UseAsyncStateOptions<T> {
  /** If true (default), the loader runs once on mount. */
  immediate?: boolean;
  /** Optional initial data. */
  initialData?: T;
  /** Optional message for console.error when the loader fails. */
  errorMessage?: string;
}

export interface ReloadOptions {
  /** If omitted, defaults to true only for the first load. */
  showLoading?: boolean;
  /** If true (default), clears `error` before loading. */
  clearError?: boolean;
}

export interface AsyncState<T> {
  data: T | undefined;
  loading: boolean;
  error: unknown;
  /** Re-runs the loader. Returns the resolved data, or undefined on error. */
  reload: (options?: ReloadOptions) => Promise<T | undefined>;
  /** Exposed for rare cases where the caller needs to mutate cached data. */
  setData: React.Dispatch<React.SetStateAction<T | undefined>>;
}

/**
 * Small helper to standardize the common "loading + error + reload" pattern.
 *
 * Guarantees:
 * - avoids stale loaders via refs
 * - ignores out-of-order responses
 * - avoids state updates after unmount
 */
export function useAsyncState<T>(loader: () => Promise<T>, options?: UseAsyncStateOptions<T>): AsyncState<T> {
  const immediate = options?.immediate ?? true;
  const errorMessage = options?.errorMessage;
  const [data, setData] = useState<T | undefined>(options?.initialData);
  const [loading, setLoading] = useState<boolean>(immediate);
  const [error, setError] = useState<unknown>(undefined);

  const mountedRef = useRef(false);
  const loaderRef = useRef(loader);
  const requestSeqRef = useRef(0);
  const hasLoadedOnceRef = useRef(false);

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  useEffect(() => {
    loaderRef.current = loader;
  }, [loader]);

  const reload = useCallback(
    async (reloadOptions?: ReloadOptions): Promise<T | undefined> => {
      const seq = ++requestSeqRef.current;
      const showLoading = reloadOptions?.showLoading ?? !hasLoadedOnceRef.current;
      const clearError = reloadOptions?.clearError ?? true;

      if (clearError && mountedRef.current) setError(undefined);
      if (showLoading && mountedRef.current) setLoading(true);

      try {
        const next = await loaderRef.current();
        if (!mountedRef.current) return undefined;
        if (seq !== requestSeqRef.current) return undefined;

        setData(next);
        setError(undefined);
        setLoading(false);
        hasLoadedOnceRef.current = true;
        return next;
      } catch (e) {
        if (!mountedRef.current) return undefined;
        if (seq !== requestSeqRef.current) return undefined;

        setError(e);
        setLoading(false);
        hasLoadedOnceRef.current = true;
        if (errorMessage) console.error(errorMessage, e);
        return undefined;
      }
    },
    [errorMessage]
  );

  useEffect(() => {
    if (!immediate) return;
    void reload({ showLoading: true });
  }, [immediate, reload]);

  return { data, loading, error, reload, setData };
}
