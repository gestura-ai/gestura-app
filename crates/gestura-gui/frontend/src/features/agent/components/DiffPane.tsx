/**
 * DiffPane — side-by-side git diff viewer using @codemirror/merge.
 *
 * Shows the HEAD version (original) alongside the working-tree version
 * (modified), with inline change markers.  Rendered when `tab.isDiffView`
 * is true and a git diff is available.
 */
import React, { useEffect, useRef } from 'react';
import { MergeView } from '@codemirror/merge';
import { EditorView } from '@codemirror/view';
import { syntaxHighlighting, defaultHighlightStyle } from '@codemirror/language';
import { javascript } from '@codemirror/lang-javascript';
import { css } from '@codemirror/lang-css';
import { html } from '@codemirror/lang-html';
import { rust } from '@codemirror/lang-rust';
import { python } from '@codemirror/lang-python';
import { json } from '@codemirror/lang-json';
import { markdown } from '@codemirror/lang-markdown';
import type { EditorLanguage } from '../utils/language';
import './DiffPane.css';

// ─── theme (shared with EditorPane, inline to avoid circular import) ──────────

const lightTheme = EditorView.theme({
  '&': { background: 'var(--bg-base)', color: 'var(--text-primary)', height: '100%' },
  '.cm-content': { fontFamily: "'JetBrains Mono','Fira Code',monospace", fontSize: '13px', lineHeight: '1.6' },
  '.cm-gutters': { background: 'var(--bg-glass)', borderRight: '1px solid var(--glass-border)', color: 'var(--text-secondary)' },
  '.cm-scroller': { overflow: 'auto' },
}, { dark: false });

const darkTheme = EditorView.theme({
  '&': { background: 'var(--bg-base)', color: 'var(--text-primary)', height: '100%' },
  '.cm-content': { fontFamily: "'JetBrains Mono','Fira Code',monospace", fontSize: '13px', lineHeight: '1.6' },
  '.cm-gutters': { background: 'rgba(28,28,30,0.6)', borderRight: '1px solid var(--glass-border)', color: 'var(--text-secondary)' },
  '.cm-scroller': { overflow: 'auto' },
}, { dark: true });

function langExt(lang: EditorLanguage) {
  switch (lang) {
    case 'javascript': return javascript({ jsx: false });
    case 'jsx': return javascript({ jsx: true });
    case 'typescript': return javascript({ typescript: true, jsx: false });
    case 'tsx': return javascript({ typescript: true, jsx: true });
    case 'css': return css();
    case 'html': return html();
    case 'rust': return rust();
    case 'python': return python();
    case 'json': return json();
    case 'markdown': return markdown();
    default: return [];
  }
}

// ─── props ────────────────────────────────────────────────────────────────────

export interface DiffPaneProps {
  /** Original content (HEAD / committed version). */
  original: string;
  /** Modified content (working-tree / current tab content). */
  modified: string;
  language: EditorLanguage;
  isDark: boolean;
}

// ─── component ────────────────────────────────────────────────────────────────

export const DiffPane: React.FC<DiffPaneProps> = ({ original, modified, language, isDark }) => {
  const containerRef = useRef<HTMLDivElement>(null);
  const mergeViewRef = useRef<MergeView | null>(null);

  useEffect(() => {
    if (!containerRef.current) return;

    const sharedExts = [
      syntaxHighlighting(defaultHighlightStyle),
      langExt(language),
      isDark ? darkTheme : lightTheme,
      EditorView.lineWrapping,
      EditorView.editable.of(false),
    ];

    const mv = new MergeView({
      parent: containerRef.current,
      a: {
        doc: original,
        extensions: sharedExts,
      },
      b: {
        doc: modified,
        extensions: sharedExts,
      },
      // Show deletions on the left, additions on the right (unified chunks)
      highlightChanges: true,
      gutter: true,
    });

    mergeViewRef.current = mv;

    return () => {
      mv.destroy();
      mergeViewRef.current = null;
    };
  }, [original, modified, language, isDark]);

  return <div ref={containerRef} className="diff-pane" />;
};

export default DiffPane;

