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

/**
 * Check whether a line is a GFM table separator row (e.g. `| --- | :---: | ---: |`).
 * Requires at least one pipe and at least one dash group of 3+ dashes.
 */
function isMarkdownTableSeparatorLine(line: string): boolean {
  const src = (line ?? '').trim();
  if (!src) return false;
  if (!src.includes('|') || !src.includes('-')) return false;
  return /^\|?\s*:?-{3,}:?\s*(\|\s*:?-{3,}:?\s*)+\|?$/.test(src);
}

/**
 * Split a GFM table row into trimmed cell strings.
 * - Strips leading/trailing pipes.
 * - Handles pipes inside inline code spans (`` `a | b` ``).
 * - Handles escaped pipes (`\|`).
 */
function splitMarkdownTableRow(line: string): string[] {
  let src = (line ?? '').trim();
  if (src.startsWith('|')) src = src.slice(1);
  if (src.endsWith('|')) src = src.slice(0, -1);

  const cells: string[] = [];
  let cell = '';
  let inCode = false;
  let escaped = false;

  for (let idx = 0; idx < src.length; idx++) {
    const ch = src[idx];
    if (escaped) {
      cell += ch;
      escaped = false;
      continue;
    }
    if (ch === '\\') {
      if (src[idx + 1] === '|') {
        escaped = true;
        continue;
      }
      cell += ch;
      continue;
    }
    if (ch === '`') {
      inCode = !inCode;
      cell += ch;
      continue;
    }
    if (ch === '|' && !inCode) {
      cells.push(cell.trim());
      cell = '';
      continue;
    }
    cell += ch;
  }
  cells.push(cell.trim());
  return cells;
}

/**
 * Parse column alignment from a GFM separator row.
 */
function parseMarkdownTableAlignments(sepLine: string, colCount: number): Array<'left' | 'right' | 'center' | null> {
  const parts = splitMarkdownTableRow(sepLine);
  const out: Array<'left' | 'right' | 'center' | null> = [];
  for (let idx = 0; idx < colCount; idx++) {
    const p = (parts[idx] ?? '').trim();
    const starts = p.startsWith(':');
    const ends = p.endsWith(':');
    if (starts && ends) out.push('center');
    else if (ends) out.push('right');
    else if (starts) out.push('left');
    else out.push(null);
  }
  return out;
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

function renderMarkdownTextLines(lines: string[]): string {
  return lines
    .map((line) => renderInlineMarkdown(line.trimEnd()))
    .join('<br />');
}

function renderMarkdownListItemParts(lines: string[]): { className: string; innerHtml: string } {
  const [firstLine = '', ...remainingLines] = lines;
  const remainderHtml = remainingLines.length > 0
    ? `<br />${renderMarkdownTextLines(remainingLines)}`
    : '';
  // Support nested inline markdown in list items
  const checkMatch = firstLine.match(/^\[([ xX])\]\s+(.*)$/);
  if (checkMatch) {
    const checked = checkMatch[1].toLowerCase() === 'x';
    return {
      className: ` class="task-item${checked ? ' checked' : ''}"`,
      innerHtml: `<input type="checkbox" ${checked ? 'checked ' : ''}disabled />${renderInlineMarkdown(checkMatch[2])}${remainderHtml}`,
    };
  }
  return {
    className: '',
    innerHtml: renderMarkdownTextLines(lines),
  };
}

type MarkdownListMatch = {
  indent: number;
  ordered: boolean;
  content: string;
  start: number | null;
};

function leadingIndentWidth(line: string): number {
  let width = 0;
  for (const ch of line) {
    if (ch === ' ') width += 1;
    else if (ch === '\t') width += 4;
    else break;
  }
  return width;
}

function matchMarkdownListItem(line: string): MarkdownListMatch | null {
  const match = line.match(/^(\s*)([-+*]|\d+\.)\s+(.+?)\s*$/);
  if (!match) return null;

  const marker = match[2];
  const ordered = /\d+\./.test(marker);
  return {
    indent: leadingIndentWidth(match[1]),
    ordered,
    content: match[3],
    start: ordered ? parseInt(marker, 10) : null,
  };
}

function renderMarkdownList(lines: string[], startIndex: number): { html: string; nextIndex: number } {
  const firstItem = matchMarkdownListItem(lines[startIndex]);
  if (!firstItem) return { html: '', nextIndex: startIndex };

  const baseIndent = firstItem.indent;
  const ordered = firstItem.ordered;
  const tag = ordered ? 'ol' : 'ul';
  const startAttr = ordered && firstItem.start != null && firstItem.start > 0
    ? ` start="${firstItem.start}"`
    : '';
  const items: string[] = [];

  let i = startIndex;
  while (i < lines.length) {
    const itemMatch = matchMarkdownListItem(lines[i]);
    if (!itemMatch || itemMatch.indent < baseIndent || itemMatch.ordered !== ordered) break;

    if (itemMatch.indent > baseIndent) {
      if (items.length === 0) break;
      const nested = renderMarkdownList(lines, i);
      items[items.length - 1] = items[items.length - 1].replace('</li>', `${nested.html}</li>`);
      i = nested.nextIndex;
      continue;
    }

    const itemLines = [itemMatch.content];
    const nestedBlocks: string[] = [];
    i += 1;

    while (i < lines.length) {
      const nextLine = lines[i];

      if (nextLine.trim() === '') {
        const nextNonBlankIndex = i + 1;
        const nextNonBlank = lines[nextNonBlankIndex];
        const nextNestedItem = nextNonBlank != null ? matchMarkdownListItem(nextNonBlank) : null;
        if (nextNestedItem && nextNestedItem.indent > baseIndent) {
          i = nextNonBlankIndex;
          continue;
        }
        i += 1;
        break;
      }

      const nextMatch = matchMarkdownListItem(nextLine);
      if (nextMatch) {
        if (nextMatch.indent > baseIndent) {
          const nested = renderMarkdownList(lines, i);
          nestedBlocks.push(nested.html);
          i = nested.nextIndex;
          continue;
        }
        break;
      }

      if (leadingIndentWidth(nextLine) > baseIndent) {
        itemLines.push(nextLine.trim());
        i += 1;
        continue;
      }

      break;
    }

    const itemParts = renderMarkdownListItemParts(itemLines);
    items.push(`<li${itemParts.className}>${itemParts.innerHtml}${nestedBlocks.join('')}</li>`);
  }

  return {
    html: `<${tag}${startAttr}>${items.join('')}</${tag}>`,
    nextIndex: i,
  };
}

// ─── Block markdown ───────────────────────────────────────────────────────────

export function parseMarkdown(input: string): string {
  const lines = input.split('\n');
  const out: string[] = [];
  const codeTokens: string[] = [];
  const paragraphBuf: string[] = [];

  function flushParagraph(buf: string[]): void {
    if (buf.length === 0) return;
    out.push(`<p>${renderMarkdownTextLines(buf)}</p>`);
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

    // Table (GFM: header row followed by separator row)
    if (line.includes('|') && i + 1 < lines.length && isMarkdownTableSeparatorLine(lines[i + 1])) {
      flushParagraph(paragraphBuf);
      const headers = splitMarkdownTableRow(line);
      const aligns = parseMarkdownTableAlignments(lines[i + 1], headers.length);
      i += 2;
      const rowLines: string[] = [];
      while (i < lines.length && !/^\s*$/.test(lines[i]) && lines[i].includes('|')) {
        rowLines.push(lines[i]);
        i++;
      }
      const thead = `<thead><tr>${headers.map((h, idx) => {
        const a = aligns[idx];
        const style = a ? ` style="text-align:${a}"` : '';
        return `<th${style}>${renderInlineMarkdown(h)}</th>`;
      }).join('')}</tr></thead>`;
      const tbodyRows = rowLines.map((rl) => {
        const cells = splitMarkdownTableRow(rl);
        return `<tr>${headers.map((_h, idx) => {
          const a = aligns[idx];
          const style = a ? ` style="text-align:${a}"` : '';
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
      const rendered = renderMarkdownList(lines, i);
      out.push(rendered.html);
      i = rendered.nextIndex;
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

