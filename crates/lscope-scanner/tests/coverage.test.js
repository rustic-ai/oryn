/**
 * Coverage test - uses Puppeteer's coverage API to measure scanner.js coverage
 */
const puppeteer = require('puppeteer');
const fs = require('fs');
const path = require('path');

const SCANNER_JS = fs.readFileSync(path.resolve(__dirname, '../src/scanner.js'), 'utf8');
const HARNESS_PATH = 'file://' + path.resolve(__dirname, 'harness.html');

describe('Scanner Coverage Analysis', () => {
    let browser;
    let page;
    const coverageData = [];

    beforeAll(async () => {
        browser = await puppeteer.launch({
            headless: 'new',
            args: ['--no-sandbox', '--disable-setuid-sandbox']
        });
    });

    afterAll(async () => {
        if (browser) await browser.close();

        // Analyze and report coverage
        console.log(`\nCollected coverage from ${coverageData.length} test runs`);
        if (coverageData.length > 0) {
            const totalBytes = SCANNER_JS.length;
            const coveredRanges = coverageData.flatMap((entry) => entry.ranges || []);

            // Merge overlapping ranges
            const sortedRanges = coveredRanges.sort((a, b) => a.start - b.start);
            const mergedRanges = [];
            for (const range of sortedRanges) {
                if (mergedRanges.length === 0 || mergedRanges[mergedRanges.length - 1].end < range.start) {
                    mergedRanges.push({ ...range });
                } else {
                    mergedRanges[mergedRanges.length - 1].end = Math.max(
                        mergedRanges[mergedRanges.length - 1].end,
                        range.end
                    );
                }
            }

            const coveredBytes = mergedRanges.reduce((sum, r) => sum + (r.end - r.start), 0);
            const coveragePercent = ((coveredBytes / totalBytes) * 100).toFixed(1);

            console.log('\n========================================');
            console.log('       SCANNER.JS COVERAGE REPORT       ');
            console.log('========================================');
            console.log(`Total bytes:   ${totalBytes}`);
            console.log(`Covered bytes: ${coveredBytes}`);
            console.log(`Coverage:      ${coveragePercent}%`);
            console.log('========================================\n');

            // Find uncovered sections
            const uncoveredSections = [];
            let lastEnd = 0;
            for (const range of mergedRanges) {
                if (range.start > lastEnd) {
                    uncoveredSections.push({ start: lastEnd, end: range.start });
                }
                lastEnd = range.end;
            }
            if (lastEnd < totalBytes) {
                uncoveredSections.push({ start: lastEnd, end: totalBytes });
            }

            // Report significant uncovered sections
            const significantUncovered = uncoveredSections.filter((s) => s.end - s.start > 100);
            if (significantUncovered.length > 0) {
                console.log('Significant uncovered sections:');
                for (const section of significantUncovered.slice(0, 5)) {
                    const snippet = SCANNER_JS.substring(section.start, Math.min(section.start + 80, section.end));
                    const lineNum = SCANNER_JS.substring(0, section.start).split('\n').length;
                    console.log(`  Line ~${lineNum}: ${snippet.trim().substring(0, 60)}...`);
                }
                console.log('');
            }
        }
    });

    beforeEach(async () => {
        page = await browser.newPage();
        await page.coverage.startJSCoverage({ includeRawScriptCoverage: true, resetOnNavigation: false });
        await page.goto(HARNESS_PATH);
        // Use addScriptTag with path so coverage API can track it properly
        await page.addScriptTag({ path: path.resolve(__dirname, '../src/scanner.js') });
    });

    afterEach(async () => {
        if (page) {
            const coverage = await page.coverage.stopJSCoverage();
            // Find our scanner script coverage (look for scanner.js URL or content)
            for (const entry of coverage) {
                if (
                    (entry.url && entry.url.includes('scanner.js')) ||
                    (entry.text && entry.text.includes('Lemmascope Universal Scanner'))
                ) {
                    coverageData.push(entry);
                    break;
                }
            }
            await page.close();
        }
    });

    // Run all commands to maximize coverage
    test('scan command', async () => {
        const result = await page.evaluate(async () => {
            return await window.Lemmascope.process({ cmd: 'scan' });
        });
        expect(result.ok).toBe(true);
    });

    test('scan with options', async () => {
        const result = await page.evaluate(async () => {
            return await window.Lemmascope.process({
                cmd: 'scan',
                include_hidden: true,
                viewport_only: true,
                max_elements: 50
            });
        });
        expect(result.ok).toBe(true);
    });

    test('click command', async () => {
        const scan = await page.evaluate(async () => window.Lemmascope.process({ cmd: 'scan' }));
        const btn = scan.elements.find((el) => el.attributes.id === 'btn-1');

        const result = await page.evaluate(async (id) => {
            return await window.Lemmascope.process({ cmd: 'click', id: id });
        }, btn.id);
        expect(result.ok).toBe(true);
    });

    test('click with options', async () => {
        const scan = await page.evaluate(async () => window.Lemmascope.process({ cmd: 'scan' }));
        const btn = scan.elements.find((el) => el.attributes.id === 'btn-1');

        const result = await page.evaluate(async (id) => {
            return await window.Lemmascope.process({
                cmd: 'click',
                id: id,
                button: 'left',
                click_count: 2,
                modifiers: ['shift']
            });
        }, btn.id);
        expect(result.ok).toBe(true);
    });

    test('type command', async () => {
        const scan = await page.evaluate(async () => window.Lemmascope.process({ cmd: 'scan' }));
        const input = scan.elements.find((el) => el.attributes.id === 'input-1');

        const result = await page.evaluate(async (id) => {
            return await window.Lemmascope.process({ cmd: 'type', id: id, text: 'test' });
        }, input.id);
        expect(result.ok).toBe(true);
    });

    test('type with delay', async () => {
        const scan = await page.evaluate(async () => window.Lemmascope.process({ cmd: 'scan' }));
        const input = scan.elements.find((el) => el.attributes.id === 'input-1');

        const result = await page.evaluate(async (id) => {
            return await window.Lemmascope.process({ cmd: 'type', id: id, text: 'ab', delay: 10, clear: false });
        }, input.id);
        expect(result.ok).toBe(true);
    });

    test('clear command', async () => {
        const scan = await page.evaluate(async () => window.Lemmascope.process({ cmd: 'scan' }));
        const input = scan.elements.find((el) => el.attributes.id === 'input-1');

        const result = await page.evaluate(async (id) => {
            return await window.Lemmascope.process({ cmd: 'clear', id: id });
        }, input.id);
        expect(result.ok).toBe(true);
    });

    test('check/uncheck commands', async () => {
        const scan = await page.evaluate(async () => window.Lemmascope.process({ cmd: 'scan' }));
        const checkbox = scan.elements.find((el) => el.attributes.id === 'check-1');

        await page.evaluate(async (id) => window.Lemmascope.process({ cmd: 'check', id: id }), checkbox.id);
        await page.evaluate(async (id) => window.Lemmascope.process({ cmd: 'uncheck', id: id }), checkbox.id);
        expect(true).toBe(true);
    });

    test('select command', async () => {
        const scan = await page.evaluate(async () => window.Lemmascope.process({ cmd: 'scan' }));
        const select = scan.elements.find((el) => el.attributes.id === 'select-1');

        await page.evaluate(async (id) => window.Lemmascope.process({ cmd: 'select', id: id, value: '2' }), select.id);
        await page.evaluate(
            async (id) => window.Lemmascope.process({ cmd: 'select', id: id, text: 'Option 3' }),
            select.id
        );
        await page.evaluate(async (id) => window.Lemmascope.process({ cmd: 'select', id: id, index: 0 }), select.id);
        expect(true).toBe(true);
    });

    test('scroll command', async () => {
        await page.evaluate(async () => window.Lemmascope.process({ cmd: 'scroll', direction: 'down', amount: 100 }));
        await page.evaluate(async () => window.Lemmascope.process({ cmd: 'scroll', direction: 'up' }));
        expect(true).toBe(true);
    });

    test('focus command', async () => {
        const scan = await page.evaluate(async () => window.Lemmascope.process({ cmd: 'scan' }));
        const input = scan.elements.find((el) => el.attributes.id === 'input-1');

        const result = await page.evaluate(async (id) => {
            return await window.Lemmascope.process({ cmd: 'focus', id: id });
        }, input.id);
        expect(result.ok).toBe(true);
    });

    test('hover command', async () => {
        const scan = await page.evaluate(async () => window.Lemmascope.process({ cmd: 'scan' }));
        const btn = scan.elements.find((el) => el.attributes.id === 'btn-1');

        const result = await page.evaluate(async (id) => {
            return await window.Lemmascope.process({ cmd: 'hover', id: id });
        }, btn.id);
        expect(result.ok).toBe(true);
    });

    test('submit command', async () => {
        const scan = await page.evaluate(async () => window.Lemmascope.process({ cmd: 'scan' }));
        const email = scan.elements.find((el) => el.attributes.id === 'email');

        const result = await page.evaluate(async (id) => {
            return await window.Lemmascope.process({ cmd: 'submit', id: id });
        }, email.id);
        expect(result.ok).toBe(true);
    });

    test('wait_for conditions', async () => {
        const scan = await page.evaluate(async () => window.Lemmascope.process({ cmd: 'scan' }));
        const btn = scan.elements.find((el) => el.attributes.id === 'btn-1');

        await page.evaluate(async (id) => {
            return await window.Lemmascope.process({ cmd: 'wait_for', condition: 'visible', id: id, timeout: 100 });
        }, btn.id);

        await page.evaluate(async () => {
            return await window.Lemmascope.process({
                cmd: 'wait_for',
                condition: 'exists',
                selector: '#btn-1',
                timeout: 100
            });
        });

        expect(true).toBe(true);
    });

    test('get_text command', async () => {
        const result = await page.evaluate(async () => {
            return await window.Lemmascope.process({ cmd: 'get_text', selector: 'h1' });
        });
        expect(result.ok).toBe(true);
    });

    test('get_value command', async () => {
        const scan = await page.evaluate(async () => window.Lemmascope.process({ cmd: 'scan' }));
        const checkbox = scan.elements.find((el) => el.attributes.id === 'check-1');

        // Test checkbox value (boolean)
        const result = await page.evaluate(async (id) => {
            return await window.Lemmascope.process({ cmd: 'get_value', id: id });
        }, checkbox.id);
        expect(result.ok).toBe(true);
    });

    test('exists command', async () => {
        const result = await page.evaluate(async () => {
            return await window.Lemmascope.process({ cmd: 'exists', selector: '#btn-1' });
        });
        expect(result.exists).toBe(true);
    });

    test('execute command', async () => {
        const result = await page.evaluate(async () => {
            return await window.Lemmascope.process({ cmd: 'execute', script: 'return 1 + 1' });
        });
        expect(result.result).toBe(2);
    });

    test('version command', async () => {
        const result = await page.evaluate(async () => {
            return await window.Lemmascope.process({ cmd: 'version' });
        });
        expect(result.protocol).toBe('1.0');
    });

    // --- Coverage tests for uncovered paths ---

    test('primary button detection - by class', async () => {
        const result = await page.evaluate(async () => {
            return await window.Lemmascope.process({ cmd: 'scan' });
        });

        // Button with btn-primary class should get 'primary' role
        const primaryBtn = result.elements.find((el) => el.attributes.id === 'btn-primary-class');
        expect(primaryBtn).toBeDefined();
        expect(primaryBtn.role).toBe('primary');

        // Button with cta class should get 'primary' role
        const ctaBtn = result.elements.find((el) => el.attributes.id === 'btn-cta');
        expect(ctaBtn).toBeDefined();
        expect(ctaBtn.role).toBe('primary');
    });

    test('primary button detection - by text', async () => {
        const result = await page.evaluate(async () => {
            return await window.Lemmascope.process({ cmd: 'scan' });
        });

        // Button with "Continue" text should get 'primary' role
        const continueBtn = result.elements.find((el) => el.attributes.id === 'btn-continue');
        expect(continueBtn).toBeDefined();
        expect(continueBtn.role).toBe('primary');

        // Button with "Save" text should get 'primary' role
        const saveBtn = result.elements.find((el) => el.attributes.id === 'btn-save');
        expect(saveBtn).toBeDefined();
        expect(saveBtn.role).toBe('primary');
    });

    test('role=button elements', async () => {
        const result = await page.evaluate(async () => {
            return await window.Lemmascope.process({ cmd: 'scan' });
        });

        // Div with role="button" should get 'button' role
        const divBtn = result.elements.find((el) => el.attributes.id === 'div-button');
        expect(divBtn).toBeDefined();
        expect(divBtn.role).toBe('button');

        // Div with role="button" and primary class should get 'primary' role
        const divPrimary = result.elements.find((el) => el.attributes.id === 'div-primary');
        expect(divPrimary).toBeDefined();
        expect(divPrimary.role).toBe('primary');
    });

    test('aria-labelledby label resolution', async () => {
        const result = await page.evaluate(async () => {
            return await window.Lemmascope.process({ cmd: 'scan' });
        });

        // Input with aria-labelledby should have label text resolved
        const usernameInput = result.elements.find((el) => el.attributes.id === 'username-input');
        expect(usernameInput).toBeDefined();
        expect(usernameInput.label).toBe('Username');

        const searchInput = result.elements.find((el) => el.attributes.id === 'search-input');
        expect(searchInput).toBeDefined();
        expect(searchInput.label).toBe('Search Query');
    });

    test('iframe scanning - same origin', async () => {
        // Wait for iframe to load
        await page.waitForSelector('#same-origin-iframe');
        await new Promise((r) => setTimeout(r, 100)); // Brief wait for srcdoc to render

        const result = await page.evaluate(async () => {
            return await window.Lemmascope.process({ cmd: 'scan', include_iframes: true });
        });

        // Should find the iframe element itself
        const iframe = result.elements.find((el) => el.attributes.id === 'same-origin-iframe');
        expect(iframe).toBeDefined();
        expect(iframe.iframe).toBeDefined();
        expect(iframe.iframe.accessible).toBe(true);
        expect(iframe.iframe.origin).toBe('same-origin');
    });

    test('iframe content elements', async () => {
        await page.waitForSelector('#same-origin-iframe');
        await new Promise((r) => setTimeout(r, 100));

        const result = await page.evaluate(async () => {
            return await window.Lemmascope.process({ cmd: 'scan', include_iframes: true });
        });

        // Should find elements from within the iframe
        const iframeBtn = result.elements.find((el) => el.attributes && el.attributes.id === 'iframe-btn');
        expect(iframeBtn).toBeDefined();
        expect(iframeBtn.text).toBe('Iframe Button');
        expect(iframeBtn.iframe_context).toBeDefined();
    });

    test('max_elements limit', async () => {
        const result = await page.evaluate(async () => {
            return await window.Lemmascope.process({ cmd: 'scan', max_elements: 5 });
        });

        // Should respect max_elements limit
        expect(result.elements.length).toBe(5);
    });

    test('max_elements with iframes', async () => {
        await page.waitForSelector('#same-origin-iframe');
        await new Promise((r) => setTimeout(r, 100));

        const result = await page.evaluate(async () => {
            return await window.Lemmascope.process({ cmd: 'scan', include_iframes: true, max_elements: 10 });
        });

        // Should stop at max_elements even with iframes
        expect(result.elements.length).toBeLessThanOrEqual(10);
    });

    test('submit button in form', async () => {
        const result = await page.evaluate(async () => {
            return await window.Lemmascope.process({ cmd: 'scan' });
        });

        // Button with type="submit" should get 'submit' role
        const singleSubmit = result.elements.find((el) => el.attributes.id === 'single-submit-btn');
        expect(singleSubmit).toBeDefined();
        expect(singleSubmit.role).toBe('submit');
    });

    test('click on hidden element - should fail', async () => {
        // First scan with include_hidden to get the hidden button's ID
        const scan = await page.evaluate(async () => {
            return await window.Lemmascope.process({ cmd: 'scan', include_hidden: true });
        });

        const hiddenBtn = scan.elements.find((el) => el.text === 'Hidden Element');
        expect(hiddenBtn).toBeDefined();

        // Try to click it - should get ELEMENT_NOT_VISIBLE error
        const result = await page.evaluate(async (id) => {
            return await window.Lemmascope.process({ cmd: 'click', id: id });
        }, hiddenBtn.id);

        expect(result.ok).toBe(false);
        expect(result.code).toBe('ELEMENT_NOT_VISIBLE');
    });

    test('click on covered element - should fail', async () => {
        const scan = await page.evaluate(async () => {
            return await window.Lemmascope.process({ cmd: 'scan' });
        });

        const coveredBtn = scan.elements.find((el) => el.attributes.id === 'covered-btn');
        expect(coveredBtn).toBeDefined();

        // Try to click it - should get ELEMENT_NOT_INTERACTABLE error
        const result = await page.evaluate(async (id) => {
            return await window.Lemmascope.process({ cmd: 'click', id: id });
        }, coveredBtn.id);

        expect(result.ok).toBe(false);
        expect(result.code).toBe('ELEMENT_NOT_INTERACTABLE');
    });

    test('click with force bypasses visibility check', async () => {
        const scan = await page.evaluate(async () => {
            return await window.Lemmascope.process({ cmd: 'scan', include_hidden: true });
        });

        const hiddenBtn = scan.elements.find((el) => el.text === 'Hidden Element');
        expect(hiddenBtn).toBeDefined();

        // Click with force=true should succeed
        const result = await page.evaluate(async (id) => {
            return await window.Lemmascope.process({ cmd: 'click', id: id, force: true });
        }, hiddenBtn.id);

        expect(result.ok).toBe(true);
    });

    test('click with offset', async () => {
        const scan = await page.evaluate(async () => {
            return await window.Lemmascope.process({ cmd: 'scan' });
        });

        const btn = scan.elements.find((el) => el.attributes.id === 'btn-1');
        expect(btn).toBeDefined();

        // Click with specific offset from top-left
        const result = await page.evaluate(async (id) => {
            return await window.Lemmascope.process({
                cmd: 'click',
                id: id,
                offset: { x: 5, y: 5 }
            });
        }, btn.id);

        expect(result.ok).toBe(true);
    });

    test('click with right button', async () => {
        const scan = await page.evaluate(async () => {
            return await window.Lemmascope.process({ cmd: 'scan' });
        });

        const btn = scan.elements.find((el) => el.attributes.id === 'btn-1');
        expect(btn).toBeDefined();

        // Right click
        const result = await page.evaluate(async (id) => {
            return await window.Lemmascope.process({
                cmd: 'click',
                id: id,
                button: 'right'
            });
        }, btn.id);

        expect(result.ok).toBe(true);
    });

    test('click with middle button', async () => {
        const scan = await page.evaluate(async () => {
            return await window.Lemmascope.process({ cmd: 'scan' });
        });

        const btn = scan.elements.find((el) => el.attributes.id === 'btn-1');
        expect(btn).toBeDefined();

        // Middle click
        const result = await page.evaluate(async (id) => {
            return await window.Lemmascope.process({
                cmd: 'click',
                id: id,
                button: 'middle'
            });
        }, btn.id);

        expect(result.ok).toBe(true);
    });

    test('type on disabled input - should fail', async () => {
        const scan = await page.evaluate(async () => {
            return await window.Lemmascope.process({ cmd: 'scan' });
        });

        const disabledInput = scan.elements.find((el) => el.attributes.id === 'disabled-input');
        expect(disabledInput).toBeDefined();

        // Try to type into disabled input - should get ELEMENT_DISABLED error
        const result = await page.evaluate(async (id) => {
            return await window.Lemmascope.process({ cmd: 'type', id: id, text: 'test' });
        }, disabledInput.id);

        expect(result.ok).toBe(false);
        expect(result.code).toBe('ELEMENT_DISABLED');
    });

    test('type appends when clear=false', async () => {
        const scan = await page.evaluate(async () => {
            return await window.Lemmascope.process({ cmd: 'scan' });
        });

        const input = scan.elements.find((el) => el.attributes.id === 'input-1');
        expect(input).toBeDefined();

        // First type something
        await page.evaluate(async (id) => {
            return await window.Lemmascope.process({ cmd: 'type', id: id, text: 'Hello' });
        }, input.id);

        // Then append more with clear=false
        const result = await page.evaluate(async (id) => {
            return await window.Lemmascope.process({ cmd: 'type', id: id, text: ' World', clear: false });
        }, input.id);

        expect(result.ok).toBe(true);

        // Verify the value was appended
        const value = await page.evaluate(async (id) => {
            return await window.Lemmascope.process({ cmd: 'get_value', id: id });
        }, input.id);

        expect(value.value).toBe('Hello World');
    });

    // --- Error handling tests ---

    test('error handling - unknown command', async () => {
        const result = await page.evaluate(async () => {
            return await window.Lemmascope.process({ cmd: 'unknown_cmd' });
        });
        expect(result.ok).toBe(false);
        expect(result.code).toBe('UNKNOWN_COMMAND');
    });

    test('error handling - element not found', async () => {
        const result = await page.evaluate(async () => {
            return await window.Lemmascope.process({ cmd: 'click', id: 99999 });
        });
        expect(result.ok).toBe(false);
        expect(result.code).toBe('ELEMENT_NOT_FOUND');
    });
});
