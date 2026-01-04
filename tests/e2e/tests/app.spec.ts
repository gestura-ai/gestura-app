import { test, expect } from '@playwright/test';

test.describe('Gestura App E2E Tests', () => {
  test('should load the main application', async ({ page }) => {
    await page.goto('/');
    
    // Check if the main title is present
    await expect(page.locator('h1')).toContainText('Gestura');
    
    // Check if navigation buttons are present
    await expect(page.locator('button:has-text("Voice")')).toBeVisible();
    await expect(page.locator('button:has-text("Ring")')).toBeVisible();
    await expect(page.locator('button:has-text("Settings")')).toBeVisible();
  });

  test('should navigate between panels', async ({ page }) => {
    await page.goto('/');
    
    // Start on voice panel
    await expect(page.locator('h2:has-text("Voice Processing")')).toBeVisible();
    
    // Navigate to ring panel
    await page.click('button:has-text("Ring")');
    await expect(page.locator('h2:has-text("Haptic Harmony Ring")')).toBeVisible();
    
    // Navigate to settings panel
    await page.click('button:has-text("Settings")');
    await expect(page.locator('h2:has-text("Settings")')).toBeVisible();
    
    // Navigate back to voice panel
    await page.click('button:has-text("Voice")');
    await expect(page.locator('h2:has-text("Voice Processing")')).toBeVisible();
  });

  test('should test voice engine functionality', async ({ page }) => {
    await page.goto('/');
    
    // Ensure we're on the voice panel
    await page.click('button:has-text("Voice")');
    
    // Test voice engine selection
    const providerSelect = page.locator('select').first();
    await providerSelect.selectOption('local');
    await expect(providerSelect).toHaveValue('local');
    
    // Test voice engine test button
    const testButton = page.locator('button:has-text("Test Engine")');
    await expect(testButton).toBeVisible();
    await testButton.click();
    
    // Should show testing state
    await expect(page.locator('button:has-text("Testing...")')).toBeVisible({ timeout: 1000 });
  });

  test('should handle ring management', async ({ page }) => {
    await page.goto('/');
    
    // Navigate to ring panel
    await page.click('button:has-text("Ring")');
    
    // Test ring scanning
    const scanButton = page.locator('button:has-text("Scan for Rings")');
    await expect(scanButton).toBeVisible();
    await scanButton.click();
    
    // Should show scanning state
    await expect(page.locator('button:has-text("Scanning...")')).toBeVisible({ timeout: 1000 });
    
    // Wait for scan to complete
    await expect(scanButton).toBeVisible({ timeout: 10000 });
  });

  test('should manage settings', async ({ page }) => {
    await page.goto('/');
    
    // Navigate to settings panel
    await page.click('button:has-text("Settings")');
    
    // Test theme mode selection
    const themeSelect = page.locator('select').first();
    await themeSelect.selectOption('dark');
    await expect(themeSelect).toHaveValue('dark');
    
    // Test accent color selection
    const accentSelect = page.locator('select').nth(1);
    await accentSelect.selectOption('emerald');
    await expect(accentSelect).toHaveValue('emerald');
    
    // Test hotkey input
    const hotkeyInput = page.locator('input[placeholder*="Ctrl+Space"]');
    await hotkeyInput.fill('Ctrl+Alt+G');
    await expect(hotkeyInput).toHaveValue('Ctrl+Alt+G');
  });

  test('should display system status', async ({ page }) => {
    await page.goto('/');
    
    // Check status indicators in header
    const statusBar = page.locator('.status-indicator').first();
    await expect(statusBar).toBeVisible();
    
    // Should show agent count
    await expect(page.locator('text=agent')).toBeVisible();
  });

  test('should handle theme switching', async ({ page }) => {
    await page.goto('/');
    
    // Navigate to settings
    await page.click('button:has-text("Settings")');
    
    // Switch to dark theme
    const themeSelect = page.locator('select').first();
    await themeSelect.selectOption('dark');
    
    // Check if dark theme is applied
    const html = page.locator('html');
    await expect(html).toHaveAttribute('data-theme', 'dark');
    
    // Switch to light theme
    await themeSelect.selectOption('light');
    await expect(html).toHaveAttribute('data-theme', 'light');
  });

  test('should handle voice transcription', async ({ page }) => {
    await page.goto('/');
    
    // Ensure we're on voice panel
    await page.click('button:has-text("Voice")');
    
    // Set up a test input path
    const inputPath = page.locator('input[placeholder*="/path/to/test.wav"]');
    await inputPath.fill('/tmp/test.wav');
    
    // Try to run transcription (will fail without real file, but should handle gracefully)
    const runButton = page.locator('button:has-text("Run Transcription")');
    if (await runButton.isEnabled()) {
      await runButton.click();
      
      // Should show processing state
      await expect(page.locator('button:has-text("Processing...")')).toBeVisible({ timeout: 1000 });
    }
  });

  test('should handle haptic feedback testing', async ({ page }) => {
    await page.goto('/');
    
    // Navigate to ring panel
    await page.click('button:has-text("Ring")');
    
    // First scan for rings
    await page.click('button:has-text("Scan for Rings")');
    
    // Wait for scan to complete and check if rings are found
    await page.waitForTimeout(2000);
    
    // If rings are available, test haptic feedback
    const clickButton = page.locator('button:has-text("Click")');
    if (await clickButton.isVisible()) {
      await clickButton.click();
      // Haptic feedback is fire-and-forget, so just verify the button works
    }
  });

  test('should persist configuration changes', async ({ page }) => {
    await page.goto('/');
    
    // Navigate to settings
    await page.click('button:has-text("Settings")');
    
    // Change theme and accent
    await page.locator('select').first().selectOption('dark');
    await page.locator('select').nth(1).selectOption('purple');
    
    // Reload page
    await page.reload();
    
    // Navigate back to settings
    await page.click('button:has-text("Settings")');
    
    // Verify settings persisted
    await expect(page.locator('select').first()).toHaveValue('dark');
    await expect(page.locator('select').nth(1)).toHaveValue('purple');
  });

  test('should handle error states gracefully', async ({ page }) => {
    await page.goto('/');
    
    // Test voice panel with invalid configuration
    await page.click('button:has-text("Voice")');
    
    // Set invalid model path
    const modelPath = page.locator('input[placeholder*="/path/to/whisper/model.bin"]');
    if (await modelPath.isVisible()) {
      await modelPath.fill('/invalid/path/model.bin');
      
      // Try to test - should handle error gracefully
      await page.click('button:has-text("Test Engine")');
      
      // Should not crash the app
      await expect(page.locator('h1:has-text("Gestura")')).toBeVisible();
    }
  });
});
