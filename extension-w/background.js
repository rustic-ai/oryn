// Background service worker for Oryn-W (WASM edition)
// This handles command processing using the client-side WASM engine

import init, { OrynCore } from './wasm/oryn_core.js';

// Global state
let orynCore = null;
let currentScan = null;
let isWasmInitialized = false;

// Initialize WASM module
async function initWasm() {
    try {
        console.log('[Oryn-W] Initializing WASM module...');
        await init();
        orynCore = new OrynCore();
        isWasmInitialized = true;

        // Expose to global scope for tests
        self.orynCore = orynCore;
        self.isWasmInitialized = isWasmInitialized;
        self.OrynCoreClass = OrynCore;

        console.log('[Oryn-W] WASM initialized successfully');
        console.log('[Oryn-W] Version:', OrynCore.getVersion());
    } catch (e) {
        console.error('[Oryn-W] Failed to initialize WASM:', e);
        isWasmInitialized = false;
        self.isWasmInitialized = false;
    }
}

// Initialize on startup
initWasm();

// Message handler
chrome.runtime.onMessage.addListener((request, sender, sendResponse) => {
    // Handle async responses
    (async () => {
        try {
            if (request.type === 'execute_oil') {
                const result = await executeOilCommand(request.oil, sender.tab?.id);
                sendResponse(result);
            } else if (request.type === 'scan_complete') {
                handleScanComplete(request.scan);
                sendResponse({ ok: true });
            } else if (request.type === 'get_status') {
                sendResponse({
                    wasmInitialized: isWasmInitialized,
                    hasScan: currentScan !== null,
                });
            } else {
                sendResponse({ error: 'Unknown message type' });
            }
        } catch (error) {
            console.error('[Oryn-W] Message handler error:', error);
            sendResponse({ error: error.message });
        }
    })();

    return true; // Keep channel open for async response
});

// Handle scan completion
function handleScanComplete(scan) {
    try {
        currentScan = scan;
        if (orynCore) {
            orynCore.updateScan(JSON.stringify(scan));
            console.log('[Oryn-W] Scan updated:', scan.stats);
        }
    } catch (error) {
        console.error('[Oryn-W] Failed to update scan:', error);
    }
}

// Execute OIL command
async function executeOilCommand(oil, tabId) {
    if (!tabId) {
        return { error: 'No active tab' };
    }

    try {
        // Check WASM is initialized
        if (!isWasmInitialized || !orynCore) {
            return { error: 'WASM not initialized' };
        }

        // Get fresh scan if needed
        if (!currentScan) {
            console.log('[Oryn-W] Getting fresh scan...');
            const scan = await scanPage(tabId);
            handleScanComplete(scan);
        }

        // Process command with WASM
        console.log('[Oryn-W] Processing command:', oil);
        const resultJson = orynCore.processCommand(oil);
        const result = JSON.parse(resultJson);

        console.log('[Oryn-W] Command processed:', result);

        // Execute the action
        if (result.Resolved) {
            const action = result.Resolved;
            return await executeAction(tabId, action);
        } else {
            return { error: 'Unexpected result format' };
        }
    } catch (error) {
        console.error('[Oryn-W] Command execution error:', error);
        return { error: error.message || String(error) };
    }
}

// Scan the page
async function scanPage(tabId) {
    console.log('[Oryn-W] Scanning page...');

    try {
        const response = await chrome.tabs.sendMessage(tabId, {
            action: 'scan',
            include_patterns: true,
        });

        if (response.error) {
            throw new Error(response.error);
        }

        return response;
    } catch (error) {
        console.error('[Oryn-W] Scan failed:', error);
        throw error;
    }
}

// Execute an action via scanner
async function executeAction(tabId, action) {
    console.log('[Oryn-W] Executing action:', action);

    try {
        // Map action types to scanner commands
        let scannerCommand;

        if (action.Scanner) {
            scannerCommand = action.Scanner;
        } else if (action.Browser) {
            return await executeBrowserAction(tabId, action.Browser);
        } else if (action.Session) {
            return await executeSessionAction(tabId, action.Session);
        } else {
            return { error: 'Unsupported action type' };
        }

        // Send to scanner
        const response = await chrome.tabs.sendMessage(tabId, scannerCommand);

        if (response.error) {
            return { error: response.error };
        }

        return { success: true, result: response };
    } catch (error) {
        console.error('[Oryn-W] Action execution error:', error);
        return { error: error.message };
    }
}

// Execute browser action (navigate, back, forward, etc.)
async function executeBrowserAction(tabId, browserAction) {
    if (browserAction.Navigate) {
        const url = browserAction.Navigate.url;
        await chrome.tabs.update(tabId, { url });
        return { success: true, message: `Navigated to ${url}` };
    }
    if (browserAction.Back) {
        await chrome.tabs.goBack(tabId);
        return { success: true, message: 'Navigated back' };
    }
    if (browserAction.Forward) {
        await chrome.tabs.goForward(tabId);
        return { success: true, message: 'Navigated forward' };
    }
    if (browserAction.Refresh) {
        await chrome.tabs.reload(tabId);
        return { success: true, message: 'Page refreshed' };
    }
    if (browserAction.Screenshot) {
        const dataUrl = await chrome.tabs.captureVisibleTab();
        return { success: true, data: dataUrl };
    }
    return { error: 'Unsupported browser action' };
}

// Execute session action (cookies, etc.)
async function executeSessionAction(tabId, sessionAction) {
    if (!sessionAction.Cookie) {
        return { error: 'Unsupported session action' };
    }

    const cookieAction = sessionAction.Cookie;
    const tab = await chrome.tabs.get(tabId);
    const tabUrl = tab.url;

    switch (cookieAction.action) {
        case 'list': {
            const cookies = await chrome.cookies.getAll({ url: tabUrl });
            return { success: true, cookies };
        }
        case 'get': {
            const cookie = await chrome.cookies.get({
                url: tabUrl,
                name: cookieAction.name,
            });
            return { success: true, cookie };
        }
        case 'set': {
            await chrome.cookies.set({
                url: tabUrl,
                name: cookieAction.name,
                value: cookieAction.value,
            });
            return { success: true, message: 'Cookie set' };
        }
        case 'delete': {
            await chrome.cookies.remove({
                url: tabUrl,
                name: cookieAction.name,
            });
            return { success: true, message: 'Cookie deleted' };
        }
        default:
            return { error: 'Unsupported cookie action' };
    }
}

console.log('[Oryn-W] Background service worker loaded');
