/**
 * EditorPane — CodeMirror 6-powered file editor panel.
 *
 * Supports:
 * - Text editing with full syntax highlighting (Rust, TS, JS, CSS, HTML, JSON, Markdown, Python)
 * - Light / dark theme (follows CSS var(--bg-base))
 * - Image preview (kind === 'image')
 * - Binary file fallback (kind === 'binary')
 * - Cmd/Ctrl+S save (fires `onSave`)
 * - Scroll position persistence (reports to `onScrollChange`)
 * - Undo / redo via browser native shortcuts
 */
import React, { useEffect, useRef } from 'react';
import { EditorState, Transaction } from '@codemirror/state';
import { EditorView, keymap, lineNumbers, highlightActiveLine, drawSelection } from '@codemirror/view';
import { defaultKeymap, history, historyKeymap, indentWithTab } from '@codemirror/commands';
import { syntaxHighlighting, defaultHighlightStyle, bracketMatching, foldGutter, codeFolding } from '@codemirror/language';
import { searchKeymap, search } from '@codemirror/search';
import { javascript } from '@codemirror/lang-javascript';
import { css } from '@codemirror/lang-css';
import { html } from '@codemirror/lang-html';
import { rust } from '@codemirror/lang-rust';
import { python } from '@codemirror/lang-python';
import { json } from '@codemirror/lang-json';
import { markdown } from '@codemirror/lang-markdown';
import type { EditorTab } from '../types';
import type { EditorLanguage } from '../utils/language';
import { parseMarkdown } from '../utils/markdown';
import './EditorPane.css';

// ─── theme ────────────────────────────────────────────────────────────────────

const lightTheme = EditorView.theme({
  '&': { background: 'var(--bg-editor-base)', color: 'var(--text-primary)', height: '100%' },
  '.cm-content': { fontFamily: "'JetBrains Mono','Fira Code',monospace", fontSize: '13px', lineHeight: '1.6' },
  '.cm-gutters': { background: 'var(--bg-glass)', borderRight: '1px solid var(--glass-border)', color: 'var(--text-secondary)' },
  '.cm-activeLineGutter': { background: 'rgba(var(--accent-primary-rgb,37,99,235),0.08)' },
  '.cm-activeLine': { background: 'rgba(var(--accent-primary-rgb,37,99,235),0.05)' },
  '.cm-cursor': { borderLeftColor: 'var(--accent-primary)' },
  '.cm-selectionBackground': { background: 'rgba(var(--accent-primary-rgb,37,99,235),0.18)' },
  '.cm-scroller': { overflow: 'auto' },
  '.cm-focused .cm-selectionBackground': { background: 'rgba(var(--accent-primary-rgb,37,99,235),0.25)' },
}, { dark: false });

const darkTheme = EditorView.theme({
  '&': { background: 'var(--bg-editor-base)', color: 'var(--text-primary)', height: '100%' },
  '.cm-content': { fontFamily: "'JetBrains Mono','Fira Code',monospace", fontSize: '13px', lineHeight: '1.6' },
  '.cm-gutters': { background: 'rgba(28,28,30,0.6)', borderRight: '1px solid var(--glass-border)', color: 'var(--text-secondary)' },
  '.cm-activeLineGutter': { background: 'rgba(var(--accent-primary-rgb,96,165,250),0.1)' },
  '.cm-activeLine': { background: 'rgba(var(--accent-primary-rgb,96,165,250),0.06)' },
  '.cm-cursor': { borderLeftColor: 'var(--accent-primary)' },
  '.cm-selectionBackground': { background: 'rgba(var(--accent-primary-rgb,96,165,250),0.2)' },
  '.cm-scroller': { overflow: 'auto' },
  '.cm-focused .cm-selectionBackground': { background: 'rgba(var(--accent-primary-rgb,96,165,250),0.28)' },
}, { dark: true });

// ─── language extensions ──────────────────────────────────────────────────────

function langExtension(lang: EditorLanguage) {
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

export interface EditorPaneProps {
  tab: EditorTab;
  isDark: boolean;
  onContentChange: (tabId: string, content: string) => void;
  onSave: (tabId: string) => void | Promise<boolean>;
  onScrollChange?: (tabId: string, offset: number) => void;
}

// ─── component ────────────────────────────────────────────────────────────────

export const EditorPane: React.FC<EditorPaneProps> = ({
  tab,
  isDark,
  onContentChange,
  onSave,
  onScrollChange,
}) => {
  const containerRef = useRef<HTMLDivElement>(null);
  const viewRef = useRef<EditorView | null>(null);
  const isExternalSyncRef = useRef(false);

  // Stable callback refs — so the editor effect never needs to re-run just
  // because the parent re-renders and passes new function references.
  const onSaveRef = useRef(onSave);
  const onContentChangeRef = useRef(onContentChange);
  const onScrollChangeRef = useRef(onScrollChange);
  useEffect(() => { onSaveRef.current = onSave; }, [onSave]);
  useEffect(() => { onContentChangeRef.current = onContentChange; }, [onContentChange]);
  useEffect(() => { onScrollChangeRef.current = onScrollChange; }, [onScrollChange]);

  // Build / rebuild the CodeMirror editor when tab identity, language, or
  // theme changes.  Content changes are synced separately (see effect below)
  // to avoid destroying and recreating the view on every keystroke.
  useEffect(() => {
    if (!containerRef.current) return;

    const saveCmd = keymap.of([{
      key: 'Mod-s',
      run: () => { void onSaveRef.current(tab.id); return true; },
    }]);

    const updateListener = EditorView.updateListener.of((update) => {
      if (update.docChanged && !isExternalSyncRef.current) {
        onContentChangeRef.current(tab.id, update.state.doc.toString());
      }
      if (update.geometryChanged || update.docChanged) {
        onScrollChangeRef.current?.(tab.id, update.view.scrollDOM.scrollTop);
      }
    });

    const state = EditorState.create({
      doc: tab.content,
      extensions: [
        history(),
        lineNumbers(),
        highlightActiveLine(),
        drawSelection(),
        bracketMatching(),
        foldGutter(),
        codeFolding(),
        syntaxHighlighting(defaultHighlightStyle),
        search({ top: true }),
        keymap.of([...defaultKeymap, ...historyKeymap, ...searchKeymap, indentWithTab]),
        saveCmd,
        langExtension(tab.language as EditorLanguage),
        isDark ? darkTheme : lightTheme,
        updateListener,
        EditorView.lineWrapping,
      ],
    });

    const view = new EditorView({ state, parent: containerRef.current });
    viewRef.current = view;

    if (tab.scrollOffset > 0) {
      // Wait one frame so the editor has laid out before restoring scroll.
      requestAnimationFrame(() => {
        if (viewRef.current) {
          viewRef.current.scrollDOM.scrollTop = tab.scrollOffset;
        }
      });
    }

    return () => {
      view.destroy();
      viewRef.current = null;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [tab.id, tab.language, isDark]); // intentionally excludes tab.content — synced below

  // Sync content pushed from outside (e.g. agent writes to file via Tauri).
  useEffect(() => {
    const view = viewRef.current;
    if (!view) return;
    const current = view.state.doc.toString();
    if (current !== tab.content) {
      const prevScrollTop = view.scrollDOM.scrollTop;
      isExternalSyncRef.current = true;
      try {
        view.dispatch({
          changes: { from: 0, to: current.length, insert: tab.content },
          annotations: Transaction.addToHistory.of(false),
        });
      } finally {
        isExternalSyncRef.current = false;
      }

      // Best-effort preserve scroll position when the agent refreshes an open file.
      requestAnimationFrame(() => {
        const v = viewRef.current;
        if (!v) return;
        v.scrollDOM.scrollTop = prevScrollTop;
      });
    }
  }, [tab.content]);

  if (tab.viewMode === 'preview' && tab.kind === 'text') {
    return (
      <div className="editor-pane editor-pane--markdown-preview">
        <div className="editor-preview-toolbar">
          <span className="editor-preview-badge">Rendered</span>
          <span className="editor-preview-hint">Read-only</span>
        </div>
        <div
          className="editor-markdown-preview markdown-body"
          data-testid="markdown-preview"
          dangerouslySetInnerHTML={{ __html: parseMarkdown(tab.content) }}
        />
      </div>
    );
  }

  if (tab.kind === 'image') {
    return (
      <div className="editor-pane editor-pane--image">
        <img src={tab.content} alt={tab.label} className="editor-image-preview" />
      </div>
    );
  }

  if (tab.kind === 'binary') {
    return (
      <div className="editor-pane editor-pane--binary">
        <span>⚠ Cannot display binary file: {tab.label}</span>
      </div>
    );
  }

  return <div ref={containerRef} className="editor-pane editor-pane--text" />;
};

