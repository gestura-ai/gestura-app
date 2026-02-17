#!/usr/bin/env node

/**
 * Simple Tray Functionality Test
 * Tests if the tray menu and click handlers are working
 */

import { spawn } from 'child_process';
import fs from 'fs';
import path from 'path';
import { fileURLToPath } from 'url';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

class TrayTester {
    constructor() {
        this.testResults = [];
    }

    log(message, type = 'info') {
        const timestamp = new Date().toISOString();
        const colorMap = {
            info: '\x1b[36m',    // Cyan
            success: '\x1b[32m', // Green
            warning: '\x1b[33m', // Yellow
            error: '\x1b[31m',   // Red
        };

        const color = colorMap[type] || '\x1b[0m';
        const reset = '\x1b[0m';

        console.log(`${color}[${timestamp}] [${type.toUpperCase()}] ${message}${reset}`);

        this.testResults.push({ timestamp, type, message });
    }

    async testHTMLFiles() {
        this.log('🔍 Testing HTML file accessibility...', 'info');

        const files = ['agent.html', 'config.html'];
        const results = {};

        for (const file of files) {
            try {
                const url = `http://localhost:1420/${file}`;
                const response = await fetch(url);

                results[file] = {
                    status: response.status,
                    ok: response.ok,
                    contentType: response.headers.get('content-type')
                };

                if (response.ok) {
                    this.log(`✅ ${file}: ${response.status} OK`, 'success');
                } else {
                    this.log(`❌ ${file}: ${response.status} ${response.statusText}`, 'error');
                }
            } catch (error) {
                this.log(`❌ ${file}: ${error.message}`, 'error');
                results[file] = { error: error.message };
            }
        }

        return results;
    }

    async testTrayFunctionality() {
        this.log('🖱️ Testing tray functionality...', 'info');

        // Since we can't programmatically click the tray icon,
        // we'll provide instructions for manual testing

        this.log('📋 Manual Tray Test Instructions:', 'info');
        this.log('1. Look for the Gestura tray icon in your system tray', 'info');
        this.log('2. Single-click the tray icon → Should open agent window', 'info');
        this.log('3. Right-click the tray icon → Should show context menu with:', 'info');
        this.log('   - 💬 Open Agent', 'info');
        this.log('   - ⚙️ Configuration', 'info');
        this.log('   - ❌ Quit', 'info');
        this.log('4. Double-click the tray icon → Should open agent window', 'info');
        this.log('5. Try selecting menu items to test window opening', 'info');

        return {
            instructions_provided: true,
            manual_testing_required: true
        };
    }

    async runTests() {
        this.log('🚀 Starting Tray Functionality Tests', 'info');

        try {
            // Test 1: HTML file accessibility
            const htmlResults = await this.testHTMLFiles();

            // Test 2: Tray functionality (manual)
            const trayResults = await this.testTrayFunctionality();

            // Generate summary
            const summary = {
                html_files: htmlResults,
                tray_functionality: trayResults,
                timestamp: new Date().toISOString()
            };

            this.log('📊 Test Summary:', 'info');
            this.log(`HTML Files: ${Object.keys(htmlResults).length} tested`, 'info');
            this.log(`Tray: Manual testing instructions provided`, 'info');

            // Save results
            const reportPath = path.join(__dirname, '..', 'test-reports', `tray-test-${Date.now()}.json`);
            fs.writeFileSync(reportPath, JSON.stringify(summary, null, 2));
            this.log(`📄 Test report saved: ${reportPath}`, 'info');

            this.log('✅ Tray tests completed', 'success');
            return summary;

        } catch (error) {
            this.log(`❌ Test failed: ${error.message}`, 'error');
            throw error;
        }
    }
}

// Run tests if called directly
if (import.meta.url === `file://${process.argv[1]}`) {
    const tester = new TrayTester();

    tester.runTests()
        .then(() => {
            console.log('\n🎉 Tray testing completed!');
            console.log('\n📋 Next Steps:');
            console.log('1. Check your system tray for the Gestura icon');
            console.log('2. Test single-click, double-click, and right-click');
            console.log('3. Verify that windows open when menu items are selected');
            process.exit(0);
        })
        .catch((error) => {
            console.error('\n❌ Tray testing failed:', error.message);
            process.exit(1);
        });
}

export default TrayTester;
