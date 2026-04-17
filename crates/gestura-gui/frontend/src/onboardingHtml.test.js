import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';

const onboardingHtml = readFileSync(resolve(process.cwd(), 'public/onboarding.html'), 'utf8');

describe('public onboarding workflow', () => {
  it('offers simple-first STT and LLM setup paths with advanced options still available', () => {
    expect(onboardingHtml).toContain('Choose how Gestura should hear you');
    expect(onboardingHtml).toContain('name="sttSetupMode" value="simple"');
    expect(onboardingHtml).toContain('Use Local Whisper');
    expect(onboardingHtml).toContain('name="sttSetupMode" value="advanced"');
    expect(onboardingHtml).toContain('id="sttProvider"');
    expect(onboardingHtml).toContain('id="sttValidationStatus"');

    expect(onboardingHtml).toContain('Choose how Gestura should think');
    expect(onboardingHtml).toContain('name="llmSetupMode" value="simple"');
    expect(onboardingHtml).toContain('Use Ollama on this computer');
    expect(onboardingHtml).toContain('name="llmSetupMode" value="advanced"');
    expect(onboardingHtml).toContain('id="llmProvider" class="onb-input onb-input--compact"');
    expect(onboardingHtml).toContain('id="llmValidationStatus"');
    expect(onboardingHtml).toContain('id="ollamaQuickSetupInstructions"');
    expect(onboardingHtml).toContain('https://ollama.com/download');
    expect(onboardingHtml).toContain('Ollama quickstart guide');
    expect(onboardingHtml).toContain('OLLAMA_AGENTIC_MODEL_MARKERS');
    expect(onboardingHtml).toContain('isAgenticOllamaModelName');
    expect(onboardingHtml).not.toContain('id="ollamaModel"');
    expect(onboardingHtml).toContain('id="cloudModelSelect"');
    expect(onboardingHtml).toContain('id="cloudModelSelectContainer"');
  });

  it('keeps tools and knowledge simple-first while preserving advanced flows', () => {
    expect(onboardingHtml).toContain('name="defaultToolsMode" value="suggested"');
    expect(onboardingHtml).toContain('Suggested tools');
    expect(onboardingHtml).toContain('name="defaultToolsMode" value="advanced"');
    expect(onboardingHtml).toContain('id="defaultToolsAdvancedPanel"');

    expect(onboardingHtml).toContain('Knowledge is optional');
    expect(onboardingHtml).toContain('name="defaultKnowledgeMode" value="skip"');
    expect(onboardingHtml).toContain('name="defaultKnowledgeMode" value="link-source"');
    expect(onboardingHtml).toContain('name="defaultKnowledgeMode" value="specialty"');
    expect(onboardingHtml).toContain('id="knowledgeAdvancedPanel"');
    expect(onboardingHtml).toContain('id="knowledgeLinkForm"');
  });
});

