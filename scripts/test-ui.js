#!/usr/bin/env node

/**
 * UI Testing Script for Gestura Tauri App
 * 
 * This script provides automated testing capabilities for the Tauri UI components.
 * It can be run independently or integrated into CI/CD pipelines.
 */

import { spawn } from 'child_process';
import fs from 'fs';
import path from 'path';
import { fileURLToPath } from 'url';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

class UITester {
    constructor() {
        this.testResults = [];
        this.logFile = path.join(__dirname, '..', 'test-results.json');
    }

    log(message, type = 'info') {
        const timestamp = new Date().toISOString();
        const logEntry = `[${timestamp}] [${type.toUpperCase()}] ${message}`;
        console.log(logEntry);

        this.testResults.push({
            timestamp,
            type,
            message
        });
    }

    async runTest(testName, testFunction) {
        this.log(`Starting test: ${testName}`, 'info');

        try {
            await testFunction();
            this.log(`✅ Test passed: ${testName}`, 'success');
            return true;
        } catch (error) {
            this.log(`❌ Test failed: ${testName} - ${error.message}`, 'error');
            return false;
        }
    }

    async testAppLaunch() {
        return new Promise((resolve, reject) => {
            this.log('Testing app launch...', 'info');

            const appProcess = spawn('just', ['dev'], {
                cwd: path.join(__dirname, '..'),
                stdio: 'pipe'
            });

            let launched = false;
            const timeout = setTimeout(() => {
                if (!launched) {
                    appProcess.kill();
                    reject(new Error('App launch timeout'));
                }
            }, 30000); // 30 second timeout

            appProcess.stdout.on('data', (data) => {
                const output = data.toString();
                if (output.includes('Gestura app initialized')) {
                    launched = true;
                    clearTimeout(timeout);
                    this.log('App launched successfully', 'success');

                    // Keep app running for a bit then kill it
                    setTimeout(() => {
                        appProcess.kill();
                        resolve();
                    }, 5000);
                }
            });

            appProcess.stderr.on('data', (data) => {
                const error = data.toString();
                if (error.includes('error') || error.includes('Error')) {
                    this.log(`App error: ${error}`, 'warning');
                }
            });

            appProcess.on('error', (error) => {
                clearTimeout(timeout);
                reject(error);
            });
        });
    }

    async testBuildProcess() {
        return new Promise((resolve, reject) => {
            this.log('Testing build process...', 'info');

            const buildProcess = spawn('just', ['build'], {
                cwd: path.join(__dirname, '..'),
                stdio: 'pipe'
            });

            let buildOutput = '';

            buildProcess.stdout.on('data', (data) => {
                buildOutput += data.toString();
            });

            buildProcess.stderr.on('data', (data) => {
                buildOutput += data.toString();
            });

            buildProcess.on('close', (code) => {
                if (code === 0) {
                    this.log('Build completed successfully', 'success');

                    // Check for warnings
                    if (buildOutput.includes('warning:')) {
                        const warnings = buildOutput.match(/warning:.*/g) || [];
                        this.log(`Build warnings found: ${warnings.length}`, 'warning');
                        warnings.forEach(warning => {
                            this.log(`Warning: ${warning}`, 'warning');
                        });
                    } else {
                        this.log('Build completed with zero warnings', 'success');
                    }

                    resolve();
                } else {
                    reject(new Error(`Build failed with exit code ${code}`));
                }
            });

            buildProcess.on('error', (error) => {
                reject(error);
            });
        });
    }

    async validateHTMLFiles() {
        const htmlFiles = [
            'src/agent.html',
            'src/config.html',
            'src/permissions.html',
            'src/status.html',
            'src/about.html',
            'src/ui-test.html'
        ];

        for (const file of htmlFiles) {
            const filePath = path.join(__dirname, '..', file);

            if (!fs.existsSync(filePath)) {
                throw new Error(`HTML file not found: ${file}`);
            }

            const content = fs.readFileSync(filePath, 'utf8');

            // Basic HTML validation
            if (!content.includes('<!DOCTYPE html>')) {
                throw new Error(`Invalid HTML structure in ${file}: Missing DOCTYPE`);
            }

            if (!content.includes('<html')) {
                throw new Error(`Invalid HTML structure in ${file}: Missing html tag`);
            }

            if (!content.includes('</html>')) {
                throw new Error(`Invalid HTML structure in ${file}: Missing closing html tag`);
            }

            // Check for Tauri API imports
            if (content.includes('@tauri-apps/api')) {
                if (!content.includes('type="module"')) {
                    this.log(`Warning: ${file} uses Tauri API but script is not type="module"`, 'warning');
                }
            }

            this.log(`✅ HTML validation passed: ${file}`, 'success');
        }
    }

    async generateReport() {
        const report = {
            timestamp: new Date().toISOString(),
            summary: {
                total: this.testResults.length,
                passed: this.testResults.filter(r => r.type === 'success').length,
                failed: this.testResults.filter(r => r.type === 'error').length,
                warnings: this.testResults.filter(r => r.type === 'warning').length
            },
            results: this.testResults
        };

        fs.writeFileSync(this.logFile, JSON.stringify(report, null, 2));
        this.log(`Test report saved to: ${this.logFile}`, 'info');

        return report;
    }

    async runAllTests() {
        this.log('🚀 Starting Gestura UI Test Suite', 'info');

        const tests = [
            ['HTML File Validation', () => this.validateHTMLFiles()],
            ['Build Process Test', () => this.testBuildProcess()],
            ['App Launch Test', () => this.testAppLaunch()]
        ];

        let passedTests = 0;
        let totalTests = tests.length;

        for (const [testName, testFunction] of tests) {
            const passed = await this.runTest(testName, testFunction);
            if (passed) passedTests++;
        }

        this.log(`\n📊 Test Summary: ${passedTests}/${totalTests} tests passed`, 'info');

        const report = await this.generateReport();

        if (passedTests === totalTests) {
            this.log('🎉 All tests passed!', 'success');
            process.exit(0);
        } else {
            this.log('❌ Some tests failed. Check the report for details.', 'error');
            process.exit(1);
        }
    }
}

// CLI interface
if (import.meta.url === `file://${process.argv[1]}`) {
    const tester = new UITester();

    const command = process.argv[2];

    switch (command) {
        case 'build':
            tester.runTest('Build Test', () => tester.testBuildProcess());
            break;
        case 'launch':
            tester.runTest('Launch Test', () => tester.testAppLaunch());
            break;
        case 'html':
            tester.runTest('HTML Validation', () => tester.validateHTMLFiles());
            break;
        case 'all':
        default:
            tester.runAllTests();
            break;
    }
}

export default UITester;
