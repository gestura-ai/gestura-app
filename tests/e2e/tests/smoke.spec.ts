import { test, expect } from '@playwright/test';

import {
  clearOnboardingComplete,
  installTauriIpcMock,
  setOnboardingComplete,
} from '../utils/tauriIpcMock';

test.describe('@smoke Gestura App', () => {
  test.beforeEach(async ({ page }) => {
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
    await expect(page.locator('h2:has-text("Welcome to Gestura")')).toBeVisible();
  });
});

