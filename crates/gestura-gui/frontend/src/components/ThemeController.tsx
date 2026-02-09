import React, { useEffect } from 'react';

interface UiSettings {
  theme_mode: string;
  accent?: string;
}

interface ThemeControllerProps {
  uiSettings: UiSettings;
  onUpdate: (settings: UiSettings) => void;
}

const ThemeController: React.FC<ThemeControllerProps> = ({ uiSettings }) => {
  useEffect(() => {
    // Apply theme on mount and when settings change
    applyTheme(uiSettings.theme_mode, uiSettings.accent || 'blue');

    // Listen for system theme changes
    const mediaQuery = window.matchMedia('(prefers-color-scheme: dark)');
    const handleChange = () => {
      if (uiSettings.theme_mode === 'system') {
        applyTheme('system', uiSettings.accent || 'blue');
      }
    };

    mediaQuery.addEventListener('change', handleChange);
    return () => mediaQuery.removeEventListener('change', handleChange);
  }, [uiSettings]);

  const hexToRgb = (hex: string): string | null => {
    const cleaned = hex.trim().replace(/^#/, '');
    if (cleaned.length !== 6) return null;
    const r = parseInt(cleaned.slice(0, 2), 16);
    const g = parseInt(cleaned.slice(2, 4), 16);
    const b = parseInt(cleaned.slice(4, 6), 16);
    if (Number.isNaN(r) || Number.isNaN(g) || Number.isNaN(b)) return null;
    return `${r}, ${g}, ${b}`;
  };

  const applyTheme = (mode: string, accent: string) => {
    const isDark = mode === 'system'
      ? window.matchMedia('(prefers-color-scheme: dark)').matches
      : mode === 'dark';

    document.documentElement.setAttribute('data-theme', isDark ? 'dark' : 'light');

    // Apply accent colors
    const accents = {
      blue: { light: '#2563eb', dark: '#60a5fa' },
      emerald: { light: '#10b981', dark: '#34d399' },
      amber: { light: '#f59e0b', dark: '#fbbf24' },
      purple: { light: '#8b5cf6', dark: '#a78bfa' },
      rose: { light: '#f43f5e', dark: '#fb7185' },
    };

    const accentColors = accents[accent as keyof typeof accents] || accents.blue;
    const accentColor = isDark ? accentColors.dark : accentColors.light;
    const accentContrast = isDark ? '#1e293b' : '#ffffff';

    const el = document.documentElement;

    el.style.setProperty('--accent', accentColor);
    el.style.setProperty('--accent-contrast', accentContrast);

    // Keep RGB token in sync for rgba() usage across the app (fallbacks exist in CSS).
    const rgb = hexToRgb(accentColor);
    if (rgb) {
      el.style.setProperty('--accent-rgb', rgb);
    }
  };

  return null; // This component only manages theme, no UI
};

export default ThemeController;
