import type { ToolBlock } from '../types';

export interface ToolSummaryItem {
  label: string;
  value: string;
}

export interface ToolPresentation {
  eyebrow: string;
  title: string;
  detail: string | null;
  parameterItems: ToolSummaryItem[];
  responseSummary: string;
  responseItems: ToolSummaryItem[];
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return value != null && typeof value === 'object' && !Array.isArray(value);
}

function collapseWhitespace(text: string): string {
  return text.replace(/\s+/g, ' ').trim();
}

function truncate(text: string, maxLength: number): string {
  if (text.length <= maxLength) return text;
  return `${text.slice(0, maxLength - 1).trimEnd()}…`;
}

function prettifyKey(key: string): string {
  return key
    .replace(/_/g, ' ')
    .replace(/([a-z0-9])([A-Z])/g, '$1 $2')
    .toLowerCase();
}

function parseStructuredText(raw: string | null | undefined): unknown {
  const trimmed = raw?.trim() ?? '';
  if (!trimmed) return null;

  try {
    return JSON.parse(trimmed) as unknown;
  } catch {
    return trimmed;
  }
}

function readString(source: Record<string, unknown> | null, keys: string[]): string | null {
  if (!source) return null;

  for (const key of keys) {
    const value = source[key];
    if (typeof value === 'string' && value.trim()) {
      return value.trim();
    }
  }

  return null;
}

function formatValue(value: unknown, key?: string, maxLength = 88): string {
  if (value == null) return '—';

  if (typeof value === 'string') {
    const compact = collapseWhitespace(value);
    const lowerKey = key?.toLowerCase() ?? '';
    if (['content', 'body', 'text', 'output', 'stdout', 'stderr', 'inline_base64'].includes(lowerKey) && compact.length > 16) {
      return `${compact.length.toLocaleString()} chars`;
    }
    return truncate(compact, maxLength) || '—';
  }

  if (typeof value === 'number' || typeof value === 'boolean') {
    return String(value);
  }

  if (Array.isArray(value)) {
    if (value.length === 0) return '0 items';
    const primitivePreview = value
      .filter((item) => ['string', 'number', 'boolean'].includes(typeof item))
      .slice(0, 3)
      .map((item) => collapseWhitespace(String(item)));

    if (primitivePreview.length > 0 && value.length <= 3) {
      return truncate(primitivePreview.join(', '), maxLength);
    }

    return `${value.length} items`;
  }

  if (isRecord(value)) {
    return `${Object.keys(value).length} fields`;
  }

  return truncate(String(value), maxLength);
}

function buildSummaryItems(value: unknown, fallbackLabel: string): ToolSummaryItem[] {
  if (value == null) return [];

  if (Array.isArray(value)) {
    return [{ label: 'items', value: `${value.length} items` }];
  }

  if (typeof value === 'string') {
    const compact = collapseWhitespace(value);
    if (fallbackLabel === 'response' && (value.includes('\n') || compact.length > 120)) {
      return [{ label: 'response', value: `${compact.length.toLocaleString()} chars` }];
    }
  }

  if (isRecord(value)) {
    return Object.entries(value)
      .filter(([, entryValue]) => entryValue != null && !(typeof entryValue === 'string' && !entryValue.trim()))
      .slice(0, 6)
      .map(([key, entryValue]) => ({
        label: prettifyKey(key),
        value: formatValue(entryValue, key),
      }));
  }

  return [{ label: fallbackLabel, value: formatValue(value) }];
}

function summarizeResponse(status: ToolBlock['status'], result: unknown): string {
  if (status === 'blocked') return 'Awaiting approval before the tool can continue.';
  if (status === 'running') return 'Preparing the tool call…';
  if (status === 'executing') return 'Processing the tool response…';

  if (typeof result === 'string') {
    const compact = collapseWhitespace(result);
    if (result.includes('\n') || compact.length > 120) {
      return `${compact.length.toLocaleString()} chars returned.`;
    }
    return compact ? truncate(compact, 140) : (status === 'success' ? 'Completed successfully.' : 'Completed with errors.');
  }

  if (Array.isArray(result)) {
    return `${result.length} items returned.`;
  }

  if (isRecord(result)) {
    const preferred = readString(result, ['summary', 'message', 'result', 'output', 'status', 'path']);
    if (preferred) return truncate(collapseWhitespace(preferred), 140);

    const keys = Object.keys(result).map(prettifyKey);
    if (keys.length === 0) return status === 'success' ? 'Completed successfully.' : 'Completed with errors.';
    return `Returned ${truncate(keys.slice(0, 3).join(' • '), 96)}.`;
  }

  return status === 'success' ? 'Completed successfully.' : 'Completed with errors.';
}

function describeActivity(toolName: string, parsedArgs: unknown): Pick<ToolPresentation, 'eyebrow' | 'title' | 'detail'> {
  const normalizedToolName = toolName.trim() || 'tool';
  const key = normalizedToolName.toLowerCase();
  const args = isRecord(parsedArgs) ? parsedArgs : null;
  const operation = readString(args, ['operation', 'action', 'mode', 'subcommand']);
  const path = readString(args, ['path', 'target', 'file_path']);
  const command = readString(args, ['command']);
  const query = readString(args, ['query', 'search']);
  const url = readString(args, ['url']);

  switch (key) {
    case 'file':
      switch ((operation ?? '').toLowerCase()) {
        case 'read':
          return { eyebrow: 'file tool', title: 'Reading file', detail: path };
        case 'write':
        case 'edit':
        case 'update':
          return { eyebrow: 'file tool', title: 'Updating file', detail: path };
        case 'search':
          return { eyebrow: 'file tool', title: 'Searching files', detail: query ?? path };
        case 'list':
        case 'tree':
          return { eyebrow: 'file tool', title: 'Inspecting workspace', detail: path };
        case 'delete':
        case 'remove':
          return { eyebrow: 'file tool', title: 'Removing file', detail: path };
        default:
          return { eyebrow: 'file tool', title: 'Working with files', detail: path ?? query };
      }
    case 'git':
      switch ((operation ?? '').toLowerCase()) {
        case 'status':
          return { eyebrow: 'git tool', title: 'Checking git status', detail: path };
        case 'diff':
          return { eyebrow: 'git tool', title: 'Reviewing git diff', detail: path };
        case 'log':
          return { eyebrow: 'git tool', title: 'Reviewing git history', detail: path };
        default:
          return { eyebrow: 'git tool', title: 'Inspecting repository state', detail: path };
      }
    case 'code':
      switch ((operation ?? '').toLowerCase()) {
        case 'symbols':
        case 'outline':
          return { eyebrow: 'code tool', title: 'Inspecting code structure', detail: path };
        case 'references':
        case 'definition':
          return { eyebrow: 'code tool', title: 'Tracing code references', detail: path };
        case 'lint':
          return { eyebrow: 'code tool', title: 'Reviewing code diagnostics', detail: path };
        case 'test':
          return { eyebrow: 'code tool', title: 'Reviewing test information', detail: path };
        default:
          return { eyebrow: 'code tool', title: 'Analyzing code context', detail: path };
      }
    case 'web_search':
      return { eyebrow: 'web search', title: 'Searching the web', detail: query };
    case 'web':
      return { eyebrow: 'web tool', title: 'Fetching web page', detail: url };
    case 'task':
    case 'tasks':
      return { eyebrow: 'task tool', title: 'Updating task plan', detail: readString(args, ['task_id', 'name']) };
    case 'mcp':
      return {
        eyebrow: 'mcp tool',
        title: 'Calling MCP tool',
        detail: readString(args, ['tool', 'tool_name', 'server', 'server_name']),
      };
    case 'screenshot':
      return { eyebrow: 'screenshot tool', title: 'Capturing screen context', detail: readString(args, ['path', 'region']) };
    case 'screen_record':
      return { eyebrow: 'screen recorder', title: 'Recording screen activity', detail: readString(args, ['path']) };
    case 'shell':
      return { eyebrow: 'shell tool', title: 'Running shell command', detail: command };
    default:
      return { eyebrow: `${normalizedToolName} tool`, title: `Running ${normalizedToolName}`, detail: path ?? command ?? query ?? url };
  }
}

export function buildToolPresentation(block: ToolBlock): ToolPresentation {
  const parsedArgs = parseStructuredText(block.args);
  const parsedResult = parseStructuredText(block.result);
  const activity = describeActivity(block.name, parsedArgs);

  return {
    eyebrow: activity.eyebrow,
    title: activity.title,
    detail: activity.detail,
    parameterItems: buildSummaryItems(parsedArgs, 'input'),
    responseSummary: summarizeResponse(block.status, parsedResult),
    responseItems: buildSummaryItems(parsedResult, 'response'),
  };
}