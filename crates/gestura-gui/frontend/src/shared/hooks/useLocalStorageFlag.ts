import { useCallback, useState } from 'react';
import { getLocalStorageFlag, setLocalStorageFlag } from '../storage/localStorageFlag';

export type LocalStorageFlagState = readonly [boolean, (value: boolean) => void];

/**
 * React hook wrapper around a boolean localStorage flag.
 *
 * NOTE: This hook is intentionally minimal and does not subscribe to cross-tab
 * `storage` events. Call the setter returned by this hook to keep UI state in sync.
 */
export function useLocalStorageFlag(key: string): LocalStorageFlagState {
  const [flag, setFlag] = useState<boolean>(() => getLocalStorageFlag(key));

  const update = useCallback(
    (value: boolean) => {
      setFlag(value);
      setLocalStorageFlag(key, value);
    },
    [key]
  );

  return [flag, update] as const;
}
