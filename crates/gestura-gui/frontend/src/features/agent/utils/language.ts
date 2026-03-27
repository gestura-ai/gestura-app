/**
 * language.ts — maps file extension / basename to a CodeMirror 6 language key.
 *
 * The returned string is used by EditorPane to load the correct @codemirror/lang-* extension.
 */

export type EditorLanguage =
  | 'javascript'
  | 'typescript'
  | 'jsx'
  | 'tsx'
  | 'css'
  | 'html'
  | 'json'
  | 'markdown'
  | 'rust'
  | 'python'
  | 'plain';

const EXT_MAP: Record<string, EditorLanguage> = {
  js: 'javascript',
  mjs: 'javascript',
  cjs: 'javascript',
  jsx: 'jsx',
  ts: 'typescript',
  mts: 'typescript',
  cts: 'typescript',
  tsx: 'tsx',
  css: 'css',
  scss: 'css',
  less: 'css',
  html: 'html',
  htm: 'html',
  xml: 'html',
  svg: 'html',
  json: 'json',
  jsonc: 'json',
  json5: 'json',
  md: 'markdown',
  mdx: 'markdown',
  rs: 'rust',
  py: 'python',
  pyi: 'python',
  toml: 'plain',
  yaml: 'plain',
  yml: 'plain',
  sh: 'plain',
  bash: 'plain',
  zsh: 'plain',
  fish: 'plain',
  txt: 'plain',
  lock: 'plain',
  gitignore: 'plain',
  env: 'plain',
};

/**
 * Derive the best editor language from a relative file path.
 *
 * @param relPath  Relative path from workspace root (e.g. "src/main.rs")
 * @returns        A language key understood by EditorPane.
 */
export function languageFromPath(relPath: string): EditorLanguage {
  const name = relPath.split('/').pop() ?? relPath;
  // Handle dotfiles (e.g. ".env", ".gitignore")
  if (name.startsWith('.')) {
    const sub = name.slice(1);
    if (sub in EXT_MAP) return EXT_MAP[sub];
    return 'plain';
  }
  const ext = name.split('.').pop()?.toLowerCase() ?? '';
  return EXT_MAP[ext] ?? 'plain';
}

/** Returns true when the file should support rendered markdown preview. */
export function isMarkdownPath(relPath: string): boolean {
  return languageFromPath(relPath) === 'markdown';
}

