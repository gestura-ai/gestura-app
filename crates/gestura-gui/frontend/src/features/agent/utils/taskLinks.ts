import type { Task, TaskHierarchy } from '../types';

export const TASK_LINK_SCHEME = 'gestura-task://';

type LinkCandidate = {
  source: string;
  taskId: string;
  label: string;
};

function flattenTasks(tasks: TaskHierarchy): Task[] {
  return tasks.flatMap((task) => [task, ...flattenTasks(task.subtasks ?? [])]);
}

function isWordChar(ch: string | undefined): boolean {
  return ch != null && /[A-Za-z0-9]/.test(ch);
}

function hasBoundary(text: string, index: number, source: string): boolean {
  const before = index > 0 ? text[index - 1] : undefined;
  const after = index + source.length < text.length ? text[index + source.length] : undefined;
  const first = source[0];
  const last = source[source.length - 1];

  if (isWordChar(first) && isWordChar(before)) return false;
  if (isWordChar(last) && isWordChar(after)) return false;
  return true;
}

function buildCandidates(tasks: TaskHierarchy): LinkCandidate[] {
  const flat = flattenTasks(tasks);
  const nameCounts = new Map<string, number>();

  for (const task of flat) {
    const name = task.name.trim();
    if (!name) continue;
    nameCounts.set(name, (nameCounts.get(name) ?? 0) + 1);
  }

  const candidates: LinkCandidate[] = flat.flatMap((task) => {
    const name = task.name.trim();
    const result: LinkCandidate[] = [{
      source: task.id,
      taskId: task.id,
      label: name || task.id,
    }];

    if (name && nameCounts.get(name) === 1) {
      result.push({ source: name, taskId: task.id, label: name });
    }

    return result;
  });

  candidates.sort((left, right) => right.source.length - left.source.length);
  return candidates;
}

function findNextMatch(text: string, start: number, candidates: LinkCandidate[]) {
  for (let index = start; index < text.length; index += 1) {
    for (const candidate of candidates) {
      if (!text.startsWith(candidate.source, index)) continue;
      if (!hasBoundary(text, index, candidate.source)) continue;
      return { index, candidate };
    }
  }

  return null;
}

function replaceTextNode(doc: Document, text: string, candidates: LinkCandidate[]): DocumentFragment | null {
  const fragment = doc.createDocumentFragment();
  let cursor = 0;
  let matched = false;

  while (cursor < text.length) {
    const match = findNextMatch(text, cursor, candidates);
    if (!match) break;

    matched = true;
    if (match.index > cursor) {
      fragment.append(text.slice(cursor, match.index));
    }

    const anchor = doc.createElement('a');
    anchor.href = taskLinkHref(match.candidate.taskId);
    anchor.dataset.taskId = match.candidate.taskId;
    anchor.dataset.taskLabel = match.candidate.label;

    const label = doc.createElement('span');
    label.className = 'task-link-label';
    label.textContent = match.candidate.label;
    anchor.append(label);

    fragment.append(anchor);
    cursor = match.index + match.candidate.source.length;
  }

  if (!matched) return null;
  if (cursor < text.length) {
    fragment.append(text.slice(cursor));
  }
  return fragment;
}

function shouldSkipElement(node: Node): boolean {
  if (node.nodeType !== Node.ELEMENT_NODE) return false;
  const tagName = (node as Element).tagName;
  return ['A', 'CODE', 'PRE', 'SCRIPT', 'STYLE'].includes(tagName);
}

function walkAndLinkify(node: Node, doc: Document, candidates: LinkCandidate[]): void {
  if (shouldSkipElement(node)) return;

  if (node.nodeType === Node.TEXT_NODE) {
    const parent = node.parentNode;
    if (!parent) return;
    const replacement = replaceTextNode(doc, node.textContent ?? '', candidates);
    if (replacement) parent.replaceChild(replacement, node);
    return;
  }

  for (const child of Array.from(node.childNodes)) {
    walkAndLinkify(child, doc, candidates);
  }
}

export function enhanceTaskReferenceHtml(html: string, tasks: TaskHierarchy): string {
  if (!html || tasks.length === 0 || typeof document === 'undefined') return html;

  const candidates = buildCandidates(tasks);
  if (candidates.length === 0) return html;

  const template = document.createElement('template');
  template.innerHTML = html;
  walkAndLinkify(template.content, document, candidates);
  return template.innerHTML;
}

export function taskLinkHref(taskId: string): string {
  return `${TASK_LINK_SCHEME}${encodeURIComponent(taskId)}`;
}