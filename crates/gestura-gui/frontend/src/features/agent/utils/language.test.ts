import { describe, expect, it } from 'vitest';
import { isMarkdownPath, languageFromPath } from './language';

describe('languageFromPath', () => {
  it('detects TypeScript', () => {
    expect(languageFromPath('src/main.ts')).toBe('typescript');
    expect(languageFromPath('src/main.mts')).toBe('typescript');
    expect(languageFromPath('src/main.cts')).toBe('typescript');
  });

  it('detects TSX', () => {
    expect(languageFromPath('src/App.tsx')).toBe('tsx');
  });

  it('detects JavaScript', () => {
    expect(languageFromPath('scripts/build.js')).toBe('javascript');
    expect(languageFromPath('scripts/build.mjs')).toBe('javascript');
    expect(languageFromPath('scripts/build.cjs')).toBe('javascript');
  });

  it('detects JSX', () => {
    expect(languageFromPath('src/App.jsx')).toBe('jsx');
  });

  it('detects Rust', () => {
    expect(languageFromPath('src/main.rs')).toBe('rust');
    expect(languageFromPath('crates/lib/src/lib.rs')).toBe('rust');
  });

  it('detects Python', () => {
    expect(languageFromPath('scripts/run.py')).toBe('python');
    expect(languageFromPath('stubs/types.pyi')).toBe('python');
  });

  it('detects CSS and CSS-like', () => {
    expect(languageFromPath('src/App.css')).toBe('css');
    expect(languageFromPath('src/App.scss')).toBe('css');
    expect(languageFromPath('src/App.less')).toBe('css');
  });

  it('detects HTML and XML', () => {
    expect(languageFromPath('public/index.html')).toBe('html');
    expect(languageFromPath('public/index.htm')).toBe('html');
    expect(languageFromPath('data/feed.xml')).toBe('html');
    expect(languageFromPath('icons/logo.svg')).toBe('html');
  });

  it('detects JSON variants', () => {
    expect(languageFromPath('tsconfig.json')).toBe('json');
    expect(languageFromPath('settings.jsonc')).toBe('json');
    expect(languageFromPath('settings.json5')).toBe('json');
  });

  it('detects Markdown', () => {
    expect(languageFromPath('README.md')).toBe('markdown');
    expect(languageFromPath('docs/guide.mdx')).toBe('markdown');
    expect(isMarkdownPath('README.md')).toBe(true);
    expect(isMarkdownPath('docs/guide.mdx')).toBe(true);
  });

  it('does not treat non-markdown files as markdown preview candidates', () => {
    expect(isMarkdownPath('src/main.ts')).toBe(false);
    expect(isMarkdownPath('docs/config.rst')).toBe(false);
  });

  it('handles dotfiles', () => {
    expect(languageFromPath('.env')).toBe('plain');
    expect(languageFromPath('.gitignore')).toBe('plain');
  });

  it('returns plain for unknown extensions', () => {
    expect(languageFromPath('Makefile')).toBe('plain');
    expect(languageFromPath('file.unknown')).toBe('plain');
    expect(languageFromPath('no-extension')).toBe('plain');
  });

  it('is case-insensitive for extensions', () => {
    expect(languageFromPath('src/Main.TS')).toBe('typescript');
    expect(languageFromPath('src/App.TSX')).toBe('tsx');
    expect(languageFromPath('src/lib.RS')).toBe('rust');
  });

  it('uses the last segment of a nested path', () => {
    expect(languageFromPath('crates/foo/src/lib.rs')).toBe('rust');
    expect(languageFromPath('a/b/c/d.tsx')).toBe('tsx');
  });
});

