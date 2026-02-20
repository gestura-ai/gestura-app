/**
 * Lightweight Markdown → HTML renderer ported from agent.html.
 * No external dependencies; consistent with the legacy rendering.
 */

// ─── Helpers ─────────────────────────────────────────────────────────────────

export function escapeHtml(t: string): string {
  return t
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;');
}

function makeSafeToken(prefix: string, idx: number): string {
  return `\x00${prefix}_${idx}\x00`;
}

function escapeHtmlText(t: string): string {
  return t.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
}

function splitMarkdownTableRow(row: string): string[] {
  return row
    .replace(/^\|/, '')
    .replace(/\|$/, '')
    .split('|')
    .map((c) => c.trim());
}

// ─── Inline markdown ──────────────────────────────────────────────────────────

export function renderInlineMarkdown(text: string): string {
  let s = escapeHtmlText(text);
  // Bold + italic
  s = s.replace(/\*\*\*(.+?)\*\*\*/g, '<strong><em>$1</em></strong>');
  // Bold
  s = s.replace(/\*\*(.+?)\*\*/g, '<strong>$1</strong>');
  s = s.replace(/__(.+?)__/g, '<strong>$1</strong>');
  // Italic
  s = s.replace(/\*([^*\n]+)\*/g, '<em>$1</em>');
  s = s.replace(/_([^_\n]+)_/g, '<em>$1</em>');
  // Strikethrough
  s = s.replace(/~~(.+?)~~/g, '<del>$1</del>');
  // Inline code
  s = s.replace(/`([^`\n]+)`/g, '<code>$1</code>');
  // Links
  s = s.replace(/\[([^\]]+)\]\((https?:\/\/[^)]+)\)/g, '<a href="$2" target="_blank" rel="noopener noreferrer">$1</a>');
  // Bare URLs
  s = s.replace(/(https?:\/\/[^\s<>"']+)/g, '<a href="$1" target="_blank" rel="noopener noreferrer">$1</a>');
  return s;
}

function renderMarkdownListItemHtml(text: string): string {
  // Support nested inline markdown in list items
  const checkMatch = text.match(/^\[([ xX])\]\s+(.*)$/);
  if (checkMatch) {
    const checked = checkMatch[1].toLowerCase() === 'x';
    return `<li class="task-item${checked ? ' checked' : ''}"><input type="checkbox" ${checked ? 'checked ' : ''}disabled />${renderInlineMarkdown(checkMatch[2])}</li>`;
  }
  return `<li>${renderInlineMarkdown(text)}</li>`;
}

// ─── Block markdown ───────────────────────────────────────────────────────────

export function parseMarkdown(input: string): string {
  const lines = input.split('\n');
  const out: string[] = [];
  const codeTokens: string[] = [];
  const paragraphBuf: string[] = [];

  function flushParagraph(buf: string[]): void {
    if (buf.length === 0) return;
    out.push(`<p>${renderInlineMarkdown(buf.join(' '))}</p>`);
    buf.length = 0;
  }

  let i = 0;
  while (i < lines.length) {
    const line = lines[i];

    // Fenced code block
    const fenceMatch = line.match(/^(`{3,}|~{3,})\s*(\S*)/);
    if (fenceMatch) {
      flushParagraph(paragraphBuf);
      const fence = fenceMatch[1];
      const lang = fenceMatch[2] || '';
      i++;
      const codeLines: string[] = [];
      while (i < lines.length && !lines[i].startsWith(fence)) {
        codeLines.push(lines[i]);
        i++;
      }
      i++; // skip closing fence
      const codeHtml = `<pre><code class="language-${escapeHtml(lang)}">${escapeHtmlText(codeLines.join('\n'))}</code></pre>`;
      const token = makeSafeToken('CODEBLOCK', codeTokens.length);
      codeTokens.push(codeHtml);
      out.push(token);
      continue;
    }

    // Headings
    const headingMatch = line.match(/^(#{1,6})\s+(.+?)\s*#*\s*$/);
    if (headingMatch) {
      flushParagraph(paragraphBuf);
      const level = headingMatch[1].length;
      out.push(`<h${level}>${renderInlineMarkdown(headingMatch[2])}</h${level}>`);
      i++;
      continue;
    }

    // Horizontal rule
    if (/^[-*_]{3,}\s*$/.test(line)) {
      flushParagraph(paragraphBuf);
      out.push('<hr />');
      i++;
      continue;
    }

    // Blockquote
    if (line.startsWith('> ')) {
      flushParagraph(paragraphBuf);
      const bqLines: string[] = [];
      while (i < lines.length && lines[i].startsWith('> ')) {
        bqLines.push(lines[i].slice(2));
        i++;
      }
      out.push(`<blockquote>${parseMarkdown(bqLines.join('\n'))}</blockquote>`);
      continue;
    }

    // Table
    const tableSepMatch = lines[i + 1]?.match(/^\|?[\s:|]+(\|[\s:|]+)*\|?\s*$/);
    if (line.trim().startsWith('|') && tableSepMatch) {
      flushParagraph(paragraphBuf);
      const headers = splitMarkdownTableRow(line);
      const sepCells = splitMarkdownTableRow(lines[i + 1]);
      const aligns = sepCells.map((c) => {
        if (/^:-+:$/.test(c.trim())) return 'center';
        if (/^-+:$/.test(c.trim())) return 'right';
        if (/^:-+$/.test(c.trim())) return 'left';
        return null;
      });
      i += 2;
      const rowLines: string[] = [];
      while (i < lines.length && lines[i].trim().startsWith('|')) {
        rowLines.push(lines[i]);
        i++;
      }
      const thead = `<thead><tr>${headers.map((h, idx) => {
        const style = aligns[idx] ? ` style="text-align:${aligns[idx]}"` : '';
        return `<th${style}>${renderInlineMarkdown(h)}</th>`;
      }).join('')}</tr></thead>`;
      const tbodyRows = rowLines.map((rl) => {
        const cells = splitMarkdownTableRow(rl);
        return `<tr>${headers.map((_h, idx) => {
          const style = aligns[idx] ? ` style="text-align:${aligns[idx]}"` : '';
          return `<td${style}>${renderInlineMarkdown(cells[idx] ?? '')}</td>`;
        }).join('')}</tr>`;
      }).join('');
      out.push(`<div class="md-table-wrapper"><table>${thead}<tbody>${tbodyRows}</tbody></table></div>`);
      continue;
    }

    // Lists
    const ulMatch = line.match(/^\s*([-+*])\s+(.+?)\s*$/);
    const olMatch = line.match(/^\s*(\d+)\.\s+(.+?)\s*$/);
    if (ulMatch || olMatch) {
      flushParagraph(paragraphBuf);
      const isOrdered = !!olMatch;
      const orderedStart = isOrdered ? parseInt((olMatch as RegExpMatchArray)[1], 10) : null;
      const items: string[] = [];
      while (i < lines.length) {
        const l = lines[i];
        const um = l.match(/^\s*([-+*])\s+(.+?)\s*$/);
        const om = l.match(/^\s*(\d+)\.\s+(.+?)\s*$/);
        if (isOrdered ? !om : !um) break;
        items.push(renderMarkdownListItemHtml(isOrdered ? (om as RegExpMatchArray)[2] : (um as RegExpMatchArray)[2]));
        i++;
      }
      const tag = isOrdered ? 'ol' : 'ul';
      const startAttr = isOrdered && orderedStart != null && orderedStart > 0 ? ` start="${orderedStart}"` : '';
      out.push(`<${tag}${startAttr}>${items.join('')}</${tag}>`);
      continue;
    }

    // Blank line → flush paragraph
    if (line.trim() === '') {
      flushParagraph(paragraphBuf);
      i++;
      continue;
    }

    paragraphBuf.push(line);
    i++;
  }

  flushParagraph(paragraphBuf);
  let html = out.join('');

  // Restore code blocks
  for (let j = 0; j < codeTokens.length; j++) {
    html = html.split(makeSafeToken('CODEBLOCK', j)).join(codeTokens[j]);
  }
  return html;
}

