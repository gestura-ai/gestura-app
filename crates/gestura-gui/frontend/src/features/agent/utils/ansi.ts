/**
 * Lightweight ANSI SGR escape sequence → HTML converter.
 * Handles colors, bold, dim, italic, underline, and reset.
 * Ported from agent.html.
 */

import { escapeHtml } from './markdown';

const COLOR_MAP = ['black', 'red', 'green', 'yellow', 'blue', 'magenta', 'cyan', 'white'] as const;

/** Convert raw terminal output with ANSI escapes into safe HTML. */
export function ansiToHtml(raw: string): string {
  const SGR = new RegExp(String.fromCharCode(27) + "\\[([0-9;]*)m", "g");
  let html = '';
  let last = 0;
  const openSpans: number[] = [];

  for (const m of raw.matchAll(SGR)) {
    if (m.index !== undefined && m.index > last) {
      html += escapeHtml(raw.slice(last, m.index));
    }
    last = (m.index ?? 0) + m[0].length;

    const codes = m[1] ? m[1].split(';').map(Number) : [0];
    for (const c of codes) {
      if (c === 0) {
        html += '</span>'.repeat(openSpans.length);
        openSpans.length = 0;
      } else if (c === 1) {
        html += '<span class="ansi-bold">';
        openSpans.push(1);
      } else if (c === 2) {
        html += '<span class="ansi-dim">';
        openSpans.push(1);
      } else if (c === 3) {
        html += '<span class="ansi-italic">';
        openSpans.push(1);
      } else if (c === 4) {
        html += '<span class="ansi-underline">';
        openSpans.push(1);
      } else if (c >= 30 && c <= 37) {
        html += `<span class="ansi-${COLOR_MAP[c - 30]}">`;
        openSpans.push(1);
      } else if (c >= 90 && c <= 97) {
        html += `<span class="ansi-bright-${COLOR_MAP[c - 90]}">`;
        openSpans.push(1);
      }
      // 38/48 (256-color & truecolor) intentionally ignored for weight
    }
  }

  if (last < raw.length) html += escapeHtml(raw.slice(last));
  html += '</span>'.repeat(openSpans.length);
  return html;
}

