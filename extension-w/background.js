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
        console.log('[Oryn-W] Service worker location:', self.location.href);

        await init();
        console.log('[Oryn-W] WASM init() completed');

        orynCore = new OrynCore();
        console.log('[Oryn-W] OrynCore instance created');

        isWasmInitialized = true;

        // Expose to global scope for tests
        self.orynCore = orynCore;
        self.isWasmInitialized = isWasmInitialized;
        self.OrynCoreClass = OrynCore;

        console.log('[Oryn-W] WASM initialized successfully');
        console.log('[Oryn-W] Version:', OrynCore.getVersion());
    } catch (e) {
        console.error('[Oryn-W] Failed to initialize WASM:');
        console.error('[Oryn-W] Error name:', e.name);
        console.error('[Oryn-W] Error message:', e.message);
        console.error('[Oryn-W] Error stack:', e.stack);
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
                // Use tabId from request (sent by popup) or from sender (sent by content script)
                const tabId = request.tabId || sender.tab?.id;
                const result = await executeOilCommand(request.oil, tabId);
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
            console.log('[Oryn-W] Sample elements:', scan.elements.slice(0, 5).map(e => ({
                id: e.id,
                type: e.type,
                text: e.text?.substring(0, 30),
                label: e.label?.substring(0, 30)
            })));
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
        console.log('[Oryn-W] WASM returned JSON:', resultJson);

        const result = JSON.parse(resultJson);
        console.log('[Oryn-W] Parsed result:', result);

        // Execute the action
        if (result.Resolved) {
            console.log('[Oryn-W] Executing resolved action');
            const action = result.Resolved;
            const execResult = await executeAction(tabId, action);
            console.log('[Oryn-W] Action execution result:', execResult);
            return execResult;
        } else {
            console.error('[Oryn-W] Unexpected result format:', result);
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
    console.log('[Oryn-W] Action type:', action.action);

    try {
        // Action enum is #[serde(untagged)], so we get the flat structure
        // Determine action category by the action field
        const actionType = action.action;

        // Browser actions: navigate, back, forward, refresh, screenshot, pdf, tab, frame, dialog, press
        const browserActions = ['navigate', 'back', 'forward', 'refresh', 'screenshot', 'pdf', 'tab', 'frame', 'dialog', 'press'];

        // Session actions: cookie, storage, headers, proxy
        const sessionActions = ['cookie', 'storage', 'headers', 'proxy'];

        // Meta actions: pack, intent, learn, config
        const metaActions = ['pack', 'intent', 'learn', 'config'];

        if (browserActions.includes(actionType)) {
            console.log('[Oryn-W] Detected Browser action');
            return await executeBrowserAction(tabId, action);
        } else if (sessionActions.includes(actionType)) {
            console.log('[Oryn-W] Detected Session action');
            return await executeSessionAction(tabId, action);
        } else if (metaActions.includes(actionType)) {
            console.log('[Oryn-W] Detected Meta action');
            return { error: 'Meta actions not supported in extension mode' };
        } else {
            // Scanner action - send directly to content script
            console.log('[Oryn-W] Detected Scanner action');
            const response = await chrome.tabs.sendMessage(tabId, action);

            if (response.error) {
                return { error: response.error };
            }

            // If this is a scan action, update the WASM context
            if (actionType === 'scan' && response.elements) {
                console.log('[Oryn-W] Updating WASM scan context with', response.elements.length, 'elements');
                handleScanComplete(response);
            }

            return { success: true, result: response };
        }
    } catch (error) {
        console.error('[Oryn-W] Action execution error:', error);
        return { error: error.message };
    }
}

// Execute browser action (navigate, back, forward, etc.)
async function executeBrowserAction(tabId, action) {
    console.log('[Oryn-W] Executing browser action:', action);

    switch (action.action) {
        case 'navigate':
            console.log('[Oryn-W] Navigating to:', action.url);
            await chrome.tabs.update(tabId, { url: action.url });
            return { success: true, message: `Navigated to ${action.url}` };

        case 'back':
            await chrome.tabs.goBack(tabId);
            return { success: true, message: 'Navigated back' };

        case 'forward':
            await chrome.tabs.goForward(tabId);
            return { success: true, message: 'Navigated forward' };

        case 'refresh':
            await chrome.tabs.reload(tabId);
            return { success: true, message: 'Page refreshed' };

        case 'screenshot':
            const dataUrl = await chrome.tabs.captureVisibleTab();
            return { success: true, data: dataUrl };

        default:
            return { error: `Unsupported browser action: ${action.action}` };
    }
}

// Execute session action (cookies, etc.)
async function executeSessionAction(tabId, action) {
    console.log('[Oryn-W] Executing session action:', action);

    const tab = await chrome.tabs.get(tabId);
    const tabUrl = tab.url;

    // SessionAction uses action field, but CookieRequest also has action field
    // So we get a flat structure with action being the cookie operation
    if (action.action) {
        // This is a cookie action
        switch (action.action) {
            case 'list': {
                const cookies = await chrome.cookies.getAll({ url: tabUrl });
                return { success: true, cookies };
            }
            case 'get': {
                const cookie = await chrome.cookies.get({
                    url: tabUrl,
                    name: action.name,
                });
                return { success: true, cookie };
            }
            case 'set': {
                await chrome.cookies.set({
                    url: tabUrl,
                    name: action.name,
                    value: action.value,
                });
                return { success: true, message: 'Cookie set' };
            }
            case 'delete': {
                await chrome.cookies.remove({
                    url: tabUrl,
                    name: action.name,
                });
                return { success: true, message: 'Cookie deleted' };
            }
            default:
                return { error: `Unsupported cookie action: ${action.action}` };
        }
    }

    return { error: 'Unsupported session action' };
}

// Open sidepanel when extension icon is clicked
chrome.action.onClicked.addListener((tab) => {
    chrome.sidePanel.open({ tabId: tab.id });
});

console.log('[Oryn-W] Background service worker loaded');
