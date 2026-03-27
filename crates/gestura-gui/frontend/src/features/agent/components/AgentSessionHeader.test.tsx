import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { AgentSessionHeader } from './AgentSessionHeader';

const getAvailableLlmProvidersMock = vi.fn();
const getSessionLlmConfigMock = vi.fn();
const getConfigMock = vi.fn();
const getApiKeyMock = vi.fn();
const listAnthropicModelsMock = vi.fn();
const listGeminiModelsMock = vi.fn();
const listGrokModelsMock = vi.fn();
const listOllamaModelsMock = vi.fn();
const listOpenAiModelsMock = vi.fn();
const setSessionLlmProviderMock = vi.fn();
const setSessionLlmModelMock = vi.fn();

vi.mock('../../../services/tauri/agent', () => ({
  getAvailableLlmProviders: (...args: unknown[]) => getAvailableLlmProvidersMock(...args),
  getSessionLlmConfig: (...args: unknown[]) => getSessionLlmConfigMock(...args),
  getConfig: (...args: unknown[]) => getConfigMock(...args),
  getApiKey: (...args: unknown[]) => getApiKeyMock(...args),
  listAnthropicModels: (...args: unknown[]) => listAnthropicModelsMock(...args),
  listGeminiModels: (...args: unknown[]) => listGeminiModelsMock(...args),
  listGrokModels: (...args: unknown[]) => listGrokModelsMock(...args),
  listOllamaModels: (...args: unknown[]) => listOllamaModelsMock(...args),
  listOpenAiModels: (...args: unknown[]) => listOpenAiModelsMock(...args),
  setSessionLlmProvider: (...args: unknown[]) => setSessionLlmProviderMock(...args),
  setSessionLlmModel: (...args: unknown[]) => setSessionLlmModelMock(...args),
}));

describe('AgentSessionHeader', () => {
  const onShowToast = vi.fn();

  afterEach(() => {
    cleanup();
  });

  beforeEach(() => {
    vi.clearAllMocks();

    getAvailableLlmProvidersMock.mockResolvedValue({
      openai: true,
      anthropic: true,
      gemini: false,
      grok: false,
      ollama: false,
    });
    getSessionLlmConfigMock.mockResolvedValue({ provider: 'openai', model: 'gpt-4o' });
    getConfigMock.mockResolvedValue({
      llm: {
        primary: 'anthropic',
        openai: { model: 'gpt-4o' },
        anthropic: { model: 'claude-sonnet-4-5' },
      },
    });
    getApiKeyMock.mockResolvedValue('test-api-key');
    listOpenAiModelsMock.mockResolvedValue([
      { id: 'gpt-4o', name: 'GPT-4o' },
      { id: 'gpt-4.1-mini', name: 'GPT-4.1 Mini' },
    ]);
    listAnthropicModelsMock.mockResolvedValue([
      { id: 'claude-sonnet-4-5', name: 'Claude Sonnet 4.5' },
      { id: 'claude-opus-4-1', name: 'Claude Opus 4.1' },
    ]);
    listGeminiModelsMock.mockResolvedValue([]);
    listGrokModelsMock.mockResolvedValue([]);
    listOllamaModelsMock.mockResolvedValue([]);
    setSessionLlmProviderMock.mockResolvedValue(undefined);
    setSessionLlmModelMock.mockResolvedValue(undefined);
  });

  it('renders the restored session header with right-aligned provider and model selectors', async () => {
    render(<AgentSessionHeader sessionId="session-header" onShowToast={onShowToast} />);

    expect(screen.getByTestId('agent-session-header')).toBeInTheDocument();

    await waitFor(() => {
      expect(screen.getByRole('button', { name: 'LLM provider' })).toHaveTextContent('OpenAI');
      expect(screen.getByRole('button', { name: 'LLM model' })).toHaveTextContent('GPT-4o');
    });
  });

  it('syncs session model changes from the header controls', async () => {
    render(<AgentSessionHeader sessionId="session-header" onShowToast={onShowToast} />);

    const modelTrigger = await screen.findByRole('button', { name: 'LLM model' });

    fireEvent.click(modelTrigger);
    fireEvent.click(await screen.findByRole('menuitemradio', { name: 'GPT-4.1 Mini' }));

    await waitFor(() => {
      expect(setSessionLlmModelMock).toHaveBeenLastCalledWith('session-header', 'gpt-4.1-mini');
    });

    expect(onShowToast).toHaveBeenCalledWith('Model updated', 'success');
  });

  it('syncs session provider changes from the header controls', async () => {
    render(<AgentSessionHeader sessionId="session-header" onShowToast={onShowToast} />);

    const providerTrigger = await screen.findByRole('button', { name: 'LLM provider' });

    fireEvent.click(providerTrigger);
    fireEvent.click(await screen.findByRole('menuitemradio', { name: 'Anthropic' }));

    await waitFor(() => {
      expect(setSessionLlmProviderMock).toHaveBeenCalledWith('session-header', 'anthropic');
      expect(setSessionLlmModelMock).toHaveBeenCalledWith('session-header', 'claude-sonnet-4-5');
    });
  });

  it('renders the header options as attached dropdown menus and closes them on outside click', async () => {
    render(<AgentSessionHeader sessionId="session-header" onShowToast={onShowToast} />);

    const providerTrigger = await screen.findByRole('button', { name: 'LLM provider' });
    fireEvent.click(providerTrigger);

    expect(await screen.findByRole('menu', { name: 'LLM provider menu' })).toBeInTheDocument();

    fireEvent.mouseDown(document.body);

    await waitFor(() => {
      expect(screen.queryByRole('menu', { name: 'LLM provider menu' })).not.toBeInTheDocument();
    });

    fireEvent.click(providerTrigger);
    expect(await screen.findByRole('menu', { name: 'LLM provider menu' })).toBeInTheDocument();

    fireEvent.keyDown(document, { key: 'Escape' });

    await waitFor(() => {
      expect(screen.queryByRole('menu', { name: 'LLM provider menu' })).not.toBeInTheDocument();
    });
  });
});
