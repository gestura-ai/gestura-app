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
    
    document.documentElement.style.setProperty('--accent', accentColor);
    document.documentElement.style.setProperty('--accent-contrast', accentContrast);
  };

  return null; // This component only manages theme, no UI
};

export default ThemeController;
