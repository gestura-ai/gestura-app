/**
 * Lightweight ANSI SGR escape sequence → HTML converter.
 * Handles colors, bold, dim, italic, underline, and reset.
 * Ported from agent.html.
 */

import { escapeHtml } from './markdown';

const COLOR_MAP = ['black', 'red', 'green', 'yellow', 'blue', 'magenta', 'cyan', 'white'] as const;
const ESC = String.fromCharCode(27);
const BEL = String.fromCharCode(7);
const OSC_SEQUENCE = new RegExp(`${ESC}\\][^${BEL}${ESC}]*(?:${BEL}|${ESC}\\\\)`, 'g');
const DCS_SEQUENCE = new RegExp(`${ESC}P[\\s\\S]*?${ESC}\\\\`, 'g');
const CSI_SEQUENCE = new RegExp(`${ESC}\\[[0-9:;<=>?]*[ -/]*[@-~]`, 'g');
const SINGLE_ESCAPE_SEQUENCE = new RegExp(`${ESC}[@-Z\\-_]`, 'g');

function stripNonSgrSequences(raw: string): string {
  return raw
    .replace(OSC_SEQUENCE, '')
    .replace(DCS_SEQUENCE, '')
    .replace(CSI_SEQUENCE, (sequence) => (sequence.endsWith('m') ? sequence : ''))
    .replace(SINGLE_ESCAPE_SEQUENCE, '');
}

function normalizeTerminalText(raw: string): string {
  const sanitized = stripNonSgrSequences(raw);
  let normalized = '';
  let currentLine = '';

  for (let index = 0; index < sanitized.length; index += 1) {
    const char = sanitized[index];

    if (char === '\x1b' && sanitized[index + 1] === '[') {
      let sgrEnd = index + 2;
      while (sgrEnd < sanitized.length && sanitized[sgrEnd] !== 'm') {
        sgrEnd += 1;
      }
      if (sgrEnd < sanitized.length) {
        currentLine += sanitized.slice(index, sgrEnd + 1);
        index = sgrEnd;
        continue;
      }
    }

    if (char === '\r') {
      currentLine = '';
      continue;
    }

    if (char === '\b') {
      currentLine = currentLine.slice(0, -1);
      continue;
    }

    if (char === '\n') {
      normalized += `${currentLine}\n`;
      currentLine = '';
      continue;
    }

    if (char === '\t' || char >= ' ') {
      currentLine += char;
    }
  }

  return normalized + currentLine;
}

/** Convert raw terminal output with ANSI escapes into safe HTML. */
export function ansiToHtml(raw: string): string {
  const normalized = normalizeTerminalText(raw);
  const SGR = new RegExp(String.fromCharCode(27) + "\\[([0-9;]*)m", "g");
  let html = '';
  let last = 0;
  const openSpans: number[] = [];

  for (const m of normalized.matchAll(SGR)) {
    if (m.index !== undefined && m.index > last) {
      html += escapeHtml(normalized.slice(last, m.index));
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

  if (last < normalized.length) html += escapeHtml(normalized.slice(last));
  html += '</span>'.repeat(openSpans.length);
  return html;
}

