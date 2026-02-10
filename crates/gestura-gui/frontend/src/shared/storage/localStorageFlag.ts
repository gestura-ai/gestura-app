/**
 * Read a boolean-like localStorage flag.
 *
 * Semantics intentionally match the legacy code: the flag is considered "set" if
 * the key exists (regardless of its string value).
 */
export function getLocalStorageFlag(key: string): boolean {
  try {
    if (typeof window === 'undefined') return false;
    return window.localStorage.getItem(key) !== null;
  } catch {
    return false;
  }
}

/**
 * Set/clear a localStorage flag.
 *
 * When setting true we store the string "true" for readability.
 * When setting false we remove the item entirely.
 */
export function setLocalStorageFlag(key: string, value: boolean): void {
  try {
    if (typeof window === 'undefined') return;

    if (value) {
      window.localStorage.setItem(key, 'true');
    } else {
      window.localStorage.removeItem(key);
    }
  } catch {
    // Best-effort only: localStorage can fail in some environments.
  }
}
