import { test, expect } from '@playwright/test';

import {
  clearOnboardingComplete,
  installTauriIpcMock,
  setOnboardingComplete,
} from '../utils/tauriIpcMock';

test.describe('@smoke Gestura App', () => {
  test.beforeEach(async ({ page }) => {
    // Surface browser runtime errors in the test runner output.
    // This keeps failures actionable when the React app fails to mount.
    page.on('pageerror', (err) => {
      // eslint-disable-next-line no-console
      console.error('[e2e][pageerror]', err);
    });

    page.on('console', (msg) => {
      const type = msg.type();
      if (type === 'error' || type === 'warning') {
        // eslint-disable-next-line no-console
        console.error(`[e2e][console:${type}] ${msg.text()}`);
      }
    });

    await installTauriIpcMock(page);
    // Default: skip onboarding so the sidebar remains interactable.
    await setOnboardingComplete(page);
  });

  test('@smoke boots to shell (Voice default)', async ({ page }) => {
    await page.goto('/');

    await expect(page.locator('h1')).toContainText('Gestura');
    await expect(page.locator('h2:has-text("Voice Processing")')).toBeVisible();

    await expect(page.locator('button:has-text("Voice")')).toBeVisible();
    await expect(page.locator('button:has-text("Ring")')).toBeVisible();
    await expect(page.locator('button:has-text("Settings")')).toBeVisible();
  });

  test('@smoke can switch panels', async ({ page }) => {
    await page.goto('/');

    await page.click('button:has-text("Ring")');
    await expect(page.locator('h2:has-text("Haptic Harmony Ring")')).toBeVisible();

    await page.click('button:has-text("Settings")');
    await expect(page.locator('h2:has-text("Settings")')).toBeVisible();

    await page.click('button:has-text("Voice")');
    await expect(page.locator('h2:has-text("Voice Processing")')).toBeVisible();
  });

  test('@smoke help opens and closes', async ({ page }) => {
    await page.goto('/');

    await page.click('button[title="Help (F1)"]');
    await expect(page.locator('.help-system-overlay')).toBeVisible();

    await page.keyboard.press('Escape');
    await expect(page.locator('.help-system-overlay')).toHaveCount(0);
  });

  test('@smoke F1 shortcut opens help (Chromium)', async ({ page, browserName }) => {
    test.skip(browserName !== 'chromium', 'F1 shortcut dispatch is not reliable outside Chromium in Playwright.');

    await page.goto('/');

    // Avoid `page.keyboard.press('F1')` (may be reserved by the host OS/browser).
    // Dispatch directly to validate our app-level shortcut handler.
    await page.evaluate(() => {
      window.dispatchEvent(new KeyboardEvent('keydown', { key: 'F1', bubbles: true }));
    });

    await expect(page.locator('.help-system-overlay')).toBeVisible();
    await expect(page.locator('h1:has-text("Gestura Help")')).toBeVisible();
  });

  test('@smoke shows onboarding when flag is missing', async ({ page }) => {
    await clearOnboardingComplete(page);
    await page.goto('/');

    await expect(page.locator('.onboarding-wizard')).toBeVisible();
    // The onboarding wizard starts on the "Configure" step (copy may evolve),
    // so assert on stable structural elements rather than specific heading text.
    await expect(page.locator('.onboarding-wizard .step-title')).toHaveText('Configure');
    await expect(page.locator('.onboarding-wizard button:has-text("Get Started")')).toBeVisible();
  });

  test('@smoke onboarding window html renders and advances', async ({ page }) => {
    await page.goto('/onboarding.html');

    await expect(page.locator('#stepName')).toHaveText('Configure');
    await expect(page.locator('#stepContent')).toContainText('Configure your agent');

    await page.click('#nextBtn');

    await expect(page.locator('#stepName')).toHaveText('Permissions');
    await expect(page.locator('#stepContent')).toContainText('System Permissions');
  });

  test('@smoke onboarding window grok provider shows API key input', async ({ page }) => {
    await page.goto('/onboarding.html');

    await expect(page.locator('#stepName')).toHaveText('Configure');

    // Welcome -> Permissions
    await page.click('#nextBtn');
    await expect(page.locator('#stepName')).toHaveText('Permissions');

    // Permissions -> Voice
    await page.click('#nextBtn');
    await expect(page.locator('#stepName')).toHaveText('Voice Setup');

    // Voice -> Voice Model
    await page.click('#nextBtn');
    await expect(page.locator('#stepName')).toHaveText('Voice Model');

    // Voice Model -> AI Provider
    await page.click('#nextBtn');
    await expect(page.locator('#stepName')).toHaveText('AI Provider');

    // Wait for the step's async hydration (loadLLMConfig) to complete.
    // The mocked default config sets llm.primary=openai, which should overwrite
    // the template default (ollama). If we select too early, the in-flight
    // hydration can race and revert the provider.
    await expect(page.locator('#llmProvider')).toHaveValue('openai');

    await page.selectOption('#llmProvider', 'grok');
    // Ensure change listeners fire reliably across browsers.
    await page.dispatchEvent('#llmProvider', 'change');

    await expect(page.locator('#llmConfig label[for="apiKey"]')).toContainText('Grok');
    await expect(page.locator('#apiKey')).toBeVisible();
    await expect(page.locator('#apiKey')).toHaveAttribute('placeholder', /xai-/);
  });

  test('@smoke chat explorer toggles and expands a directory', async ({ page }) => {
    await page.goto('/chat.html?session_id=e2e-session');

    const explorerPanel = page.locator('#explorerPanel');
    const explorerOverlay = page.locator('#explorerPanelOverlay');
    const explorerTree = page.locator('#explorerTree');

    await expect(explorerPanel).not.toHaveClass(/\bopen\b/);

    // Open via quick-access button.
    await page.click('#quickExplorerBtn');
    await expect(explorerPanel).toHaveClass(/\bopen\b/);
    await expect(explorerOverlay).toHaveClass(/\bvisible\b/);

    // Tree should populate from mocked IPC.
    await expect(explorerTree.locator('.explorer-row')).toHaveCount(3);
    await expect(explorerTree.locator('.explorer-row', { hasText: 'src' })).toBeVisible();

    // Expand src and expect children to render.
    await explorerTree.locator('.explorer-row', { hasText: 'src' }).click();
    await expect(explorerTree.locator('.explorer-row', { hasText: 'main.rs' })).toBeVisible();

    // Git badge should render for a modified file.
    const mainRow = explorerTree.locator('.explorer-row', { hasText: 'main.rs' });
    await expect(mainRow.locator('.git-badge.modified')).toHaveCount(1);

    // Toggle closed via hotkey (Ctrl/Cmd + B) using a deterministic dispatched event.
    await page.evaluate(() => {
      document.dispatchEvent(
        new KeyboardEvent('keydown', {
          key: 'b',
          ctrlKey: true,
          bubbles: true,
        })
      );
    });
    await expect(explorerPanel).not.toHaveClass(/\bopen\b/);

    // Toggle open again.
    await page.evaluate(() => {
      document.dispatchEvent(
        new KeyboardEvent('keydown', {
          key: 'b',
          ctrlKey: true,
          bubbles: true,
        })
      );
    });
    await expect(explorerPanel).toHaveClass(/\bopen\b/);
  });
});

