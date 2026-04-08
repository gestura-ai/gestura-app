import { describe, expect, it } from 'vitest';

import { ansiToHtml } from './ansi';

describe('ansiToHtml', () => {
  it('removes terminal cursor-control redraw sequences from inline transcripts', () => {
    const html = ansiToHtml('\u001b[?25l⠋\r\u001b[1G\u001b[0Kdone\n\u001b[?25h');

    expect(html).toContain('done');
    expect(html).not.toContain('?25');
    expect(html).not.toContain('⠋');
  });

  it('keeps only the latest carriage-return rewrite in a chunk', () => {
    const html = ansiToHtml('step 1\rstep 2\n');

    expect(html).toContain('step 2');
    expect(html).not.toContain('step 1');
  });

  it('preserves ansi styling sequences', () => {
    const html = ansiToHtml('\u001b[31merror\u001b[0m');

    expect(html).toContain('ansi-red');
    expect(html).toContain('error');
  });
});