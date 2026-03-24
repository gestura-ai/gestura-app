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

interface TaskToolResultPresentation {
  summary: string;
  items: ToolSummaryItem[];
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

function titleTokens(text: string | null | undefined, maxWords: number): string | null {
  const compact = collapseWhitespace(text ?? '');
  if (!compact) return null;

  const tokens = compact
    .split(/[^A-Za-z0-9._/-]+/)
    .map((token) => token.trim())
    .filter(Boolean)
    .slice(0, maxWords);

  return tokens.length >= 2 ? tokens.join(' ') : null;
}

function contextualVerbTitle(verb: string, detail: string | null | undefined, fallback: string, maxWords = 3): string {
  const tokens = titleTokens(detail, maxWords);
  return tokens ? `${verb} ${tokens}` : fallback;
}

function urlHost(url: string | null | undefined): string | null {
  if (!url) return null;
  try {
    return new URL(url).host || null;
  } catch {
    return null;
  }
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

function isUuidLike(value: string | null | undefined): boolean {
  if (!value) return false;
  return /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(value.trim());
}

function humanizeTaskStatus(status: string | null | undefined): string | null {
  const normalized = status?.trim().toLowerCase();
  if (!normalized) return null;

  switch (normalized) {
    case 'notstarted':
    case 'not_started':
      return 'Not started';
    case 'inprogress':
    case 'in_progress':
      return 'In progress';
    case 'completed':
      return 'Completed';
    case 'blocked':
      return 'Blocked';
    case 'cancelled':
      return 'Cancelled';
    default:
      return normalized.replace(/_/g, ' ');
  }
}

function taskDetailCandidate(args: Record<string, unknown> | null): string | null {
  const name = readString(args, ['name']);
  if (name) return name;

  const taskId = readString(args, ['task_id']);
  return taskId && !isUuidLike(taskId) ? taskId : null;
}

function describeTaskActivity(args: Record<string, unknown> | null): Pick<ToolPresentation, 'eyebrow' | 'title' | 'detail'> {
  const operation = (readString(args, ['operation', 'action', 'mode', 'subcommand']) ?? '').toLowerCase();
  const detail = taskDetailCandidate(args);
  const status = humanizeTaskStatus(readString(args, ['status', 'state', 'new_status', 'target_status']));

  switch (operation) {
    case 'create':
      return { eyebrow: 'task tool', title: 'Creating task', detail };
    case 'update_status':
      if (status === 'In progress') return { eyebrow: 'task tool', title: 'Starting task work', detail };
      if (status === 'Completed') return { eyebrow: 'task tool', title: 'Marking task complete', detail };
      if (status === 'Blocked') return { eyebrow: 'task tool', title: 'Marking task blocked', detail };
      if (status === 'Cancelled') return { eyebrow: 'task tool', title: 'Cancelling task', detail };
      if (status === 'Not started') return { eyebrow: 'task tool', title: 'Resetting task status', detail };
      return { eyebrow: 'task tool', title: 'Updating task status', detail };
    case 'update':
      return { eyebrow: 'task tool', title: 'Editing task details', detail };
    case 'delete':
      return { eyebrow: 'task tool', title: 'Removing task', detail };
    case 'list':
    case 'get_hierarchy':
      return { eyebrow: 'task tool', title: 'Reviewing task plan', detail };
    default:
      return { eyebrow: 'task tool', title: 'Shaping task plan', detail };
  }
}

function extractTaskToolResultPresentation(args: Record<string, unknown> | null, result: unknown, status: ToolBlock['status']): TaskToolResultPresentation | null {
  if (typeof result !== 'string') return null;

  const compact = collapseWhitespace(result);
  if (!compact) {
    return {
      summary: status === 'success' ? 'Task update completed.' : 'Task update failed.',
      items: [],
    };
  }

  const taskId = readString(args, ['task_id']);

  if (compact.startsWith("Missing required field 'status' for update_status operation.")) {
    const providedFields = compact.match(/Provided fields: ([^.]+)\./)?.[1] ?? null;
    const retryExample = compact.match(/Retry with (\{.+?\}) using one of:/)?.[1] ?? null;

    return {
      summary: 'Task status update needs an explicit status value.',
      items: [
        taskId ? { label: 'task id', value: taskId } : null,
        providedFields ? { label: 'provided fields', value: providedFields } : null,
        retryExample ? { label: 'retry example', value: retryExample } : null,
      ].filter((item): item is ToolSummaryItem => item != null),
    };
  }

  if (compact.startsWith('Missing required update fields for update operation.')) {
    const providedFields = compact.match(/Provided fields: ([^.]+)\./)?.[1] ?? null;
    const retryExample = compact.match(/Retry with (\{.+?\})\./)?.[1] ?? null;

    return {
      summary: 'Task detail update needs a name or description change.',
      items: [
        taskId ? { label: 'task id', value: taskId } : null,
        providedFields ? { label: 'provided fields', value: providedFields } : null,
        retryExample ? { label: 'retry example', value: retryExample } : null,
      ].filter((item): item is ToolSummaryItem => item != null),
    };
  }

  const createdMatch = compact.match(/^Created task '(.+?)' \(ID: ([^)]+)\)(?: Description: (.+?))?(?: Status: (.+))?$/);
  if (createdMatch) {
    return {
      summary: `Created task “${createdMatch[1]}”.`,
      items: [
        { label: 'task', value: createdMatch[1] },
        { label: 'task id', value: createdMatch[2] },
        createdMatch[4] ? { label: 'status', value: createdMatch[4] } : null,
      ].filter((item): item is ToolSummaryItem => item != null),
    };
  }

  const statusUpdateMatch = compact.match(/^Updated task ([^ ]+) status to ([A-Za-z_]+)$/);
  if (statusUpdateMatch) {
    const humanStatus = humanizeTaskStatus(statusUpdateMatch[2]) ?? statusUpdateMatch[2];
    return {
      summary: `Updated task status to ${humanStatus}.`,
      items: [
        { label: 'task id', value: statusUpdateMatch[1] },
        { label: 'status', value: humanStatus },
      ],
    };
  }

  const updateMatch = compact.match(/^Updated task ([^:]+): (.+)$/);
  if (updateMatch) {
    return {
      summary: 'Updated task details.',
      items: [
        { label: 'task id', value: updateMatch[1] },
        { label: 'changes', value: updateMatch[2] },
      ],
    };
  }

  const deletedMatch = compact.match(/^Deleted task '(.+?)' \(ID: ([^)]+)\)$/);
  if (deletedMatch) {
    return {
      summary: `Deleted task “${deletedMatch[1]}”.`,
      items: [
        { label: 'task', value: deletedMatch[1] },
        { label: 'task id', value: deletedMatch[2] },
      ],
    };
  }

  const failedPrefix = compact.match(/^Failed to (create task|update task status|update task|delete task): (.+)$/);
  if (failedPrefix) {
    return {
      summary: truncate(failedPrefix[2], 160),
      items: failedPrefix[1] ? [{ label: 'operation', value: failedPrefix[1] }] : [],
    };
  }

  return {
    summary: truncate(compact, 160),
    items: taskId ? [{ label: 'task id', value: taskId }] : [],
  };
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
      return {
        eyebrow: 'web search',
        title: contextualVerbTitle('Researching', query, 'Reviewing research findings'),
        detail: query,
      };
    case 'web':
      return {
        eyebrow: 'web tool',
        title: contextualVerbTitle('Reviewing', urlHost(url) ?? url, 'Reviewing source material', 4),
        detail: url,
      };
    case 'task':
    case 'tasks':
      return describeTaskActivity(args);
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
  const taskResult = ['task', 'tasks'].includes(block.name.toLowerCase()) && isRecord(parsedArgs)
    ? extractTaskToolResultPresentation(parsedArgs, parsedResult, block.status)
    : null;

  return {
    eyebrow: activity.eyebrow,
    title: activity.title,
    detail: activity.detail,
    parameterItems: buildSummaryItems(parsedArgs, 'input'),
    responseSummary: taskResult?.summary ?? summarizeResponse(block.status, parsedResult),
    responseItems: taskResult?.items ?? buildSummaryItems(parsedResult, 'response'),
  };
}