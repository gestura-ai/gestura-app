import type { ShellLine } from '../types';

function shellPromptPrefix(): string {
  if (typeof navigator !== 'undefined') {
    const platform = ((navigator as Navigator & { userAgentData?: { platform?: string } }).userAgentData?.platform
      ?? navigator.platform
      ?? '')
      .toLowerCase();
    if (platform.includes('win')) return '>';
  }
  return '$';
}

export function buildShellCommandLine(command: string): ShellLine | null {
  const trimmed = command.trim();
  if (!trimmed) return null;

  return {
    stream: 'Stdout',
    data: `${shellPromptPrefix()} ${trimmed}\r\n`,
  };
}