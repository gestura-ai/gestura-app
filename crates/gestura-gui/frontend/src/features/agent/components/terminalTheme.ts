function readCssColor(name: string, fallback: string): string {
  if (typeof window === 'undefined') return fallback;
  const value = window.getComputedStyle(document.documentElement).getPropertyValue(name).trim();
  return value || fallback;
}

export function buildTerminalTheme() {
  const accent = readCssColor('--accent-primary', '#3b82f6');
  const background = readCssColor('--bg-base', '#111827');
  const elevated = readCssColor('--bg-glass-strong', background);
  const foreground = readCssColor('--text-primary', '#e5e7eb');
  const secondary = readCssColor('--text-secondary', '#94a3b8');

  return {
    background,
    foreground,
    cursor: accent,
    cursorAccent: background,
    selectionBackground: 'rgba(59, 130, 246, 0.18)',
    black: background,
    red: '#ef4444',
    green: '#22c55e',
    yellow: '#f59e0b',
    blue: accent,
    magenta: '#a855f7',
    cyan: '#06b6d4',
    white: foreground,
    brightBlack: secondary,
    brightRed: '#f87171',
    brightGreen: '#4ade80',
    brightYellow: '#fbbf24',
    brightBlue: '#60a5fa',
    brightMagenta: '#c084fc',
    brightCyan: '#22d3ee',
    brightWhite: elevated,
  };
}