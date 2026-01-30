// Background service worker for Oryn-W (WASM edition)
// This handles command processing using the client-side WASM engine

import init, { OrynCore } from './wasm/oryn_core.js';
import { LLMManager } from './llm/llm_manager.js';
import { TrajectoryStore } from './agent/trajectory_store.js';
import { RalphAgent } from './agent/ralph_agent.js';
import { loadSeedTrajectories } from './agent/seed_trajectories.js';

// Global state
let orynCore = null;
let currentScan = null;
let isWasmInitialized = false;
let llmManager = null;
let trajectoryStore = null;
let ralphAgent = null;

// Agent state
let agentState = {
    active: false,
    task: null,
    currentIteration: 0,
    maxIterations: 10,
    history: [],
    startTime: null,
};

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

// Initialize LLM manager
async function initLLM() {
    try {
        console.log('[Oryn-W] Initializing LLM manager...');
        llmManager = new LLMManager();
        await llmManager.initialize();
        console.log('[Oryn-W] LLM manager initialized successfully');
    } catch (error) {
        console.error('[Oryn-W] Failed to initialize LLM manager:', error);
    }
}

// Initialize trajectory store and agent
async function initAgent() {
    try {
        console.log('[Oryn-W] Initializing trajectory store...');
        trajectoryStore = new TrajectoryStore();
        await trajectoryStore.initialize();

        // Check if we need to load seed trajectories
        const stats = await trajectoryStore.getStats();
        if (stats.total === 0) {
            console.log('[Oryn-W] Loading seed trajectories...');
            await loadSeedTrajectories(trajectoryStore);
        } else {
            console.log('[Oryn-W] Trajectory store has', stats.total, 'trajectories');
        }

        console.log('[Oryn-W] Trajectory store initialized successfully');
    } catch (error) {
        console.error('[Oryn-W] Failed to initialize trajectory store:', error);
    }
}

// Create Ralph agent instance
function createRalphAgent(config = {}) {
    if (!llmManager || !trajectoryStore) {
        throw new Error('LLM manager and trajectory store must be initialized first');
    }

    return new RalphAgent(llmManager, trajectoryStore, config);
}

// Initialize on startup
initWasm();
initLLM();
initAgent();

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
            } else if (request.type === 'format_scan') {
                // Format scan using WASM formatter for uniform output
                if (!isWasmInitialized || !orynCore) {
                    sendResponse({ error: 'WASM not initialized' });
                    return;
                }

                try {
                    const scanJson = JSON.stringify(request.scan);
                    const formatted = OrynCore.formatScan(scanJson);
                    sendResponse({ success: true, formatted });
                } catch (error) {
                    console.error('[Oryn-W] Format scan failed:', error);
                    sendResponse({ error: error.message });
                }
            } else if (request.type === 'llm_status') {
                // Get LLM status
                const status = llmManager ? llmManager.getStatus() : { ready: false, error: 'LLM manager not initialized' };
                sendResponse(status);
            } else if (request.type === 'llm_get_adapters') {
                // Get available adapters
                console.log('[Oryn-W] llm_get_adapters request received');
                const adapters = llmManager ? llmManager.getAvailableAdapters() : [];
                console.log('[Oryn-W] Returning adapters:', adapters);
                sendResponse({ adapters });
            } else if (request.type === 'llm_set_adapter') {
                // Set active adapter
                if (!llmManager) {
                    sendResponse({ error: 'LLM manager not initialized' });
                    return;
                }

                try {
                    const config = {
                        apiKey: request.apiKey,
                        temperature: request.temperature,
                    };

                    await llmManager.setActiveAdapter(request.adapter, request.model, config);
                    sendResponse({ success: true, status: llmManager.getStatus() });
                } catch (error) {
                    console.error('[Oryn-W] Failed to set adapter:', error);
                    sendResponse({ error: error.message });
                }
            } else if (request.type === 'llm_prompt') {
                // Send prompt to LLM
                if (!llmManager) {
                    sendResponse({ error: 'LLM manager not initialized' });
                    return;
                }

                try {
                    const response = await llmManager.prompt(request.messages, request.options || {});
                    sendResponse({ success: true, response });
                } catch (error) {
                    console.error('[Oryn-W] LLM prompt failed:', error);
                    sendResponse({ error: error.message });
                }
            } else if (request.type === 'llm_stream') {
                // Stream prompt to LLM (not fully supported yet, falls back to prompt)
                if (!llmManager) {
                    sendResponse({ error: 'LLM manager not initialized' });
                    return;
                }

                try {
                    const response = await llmManager.prompt(request.messages, request.options || {});
                    sendResponse({ success: true, response });
                } catch (error) {
                    console.error('[Oryn-W] LLM stream failed:', error);
                    sendResponse({ error: error.message });
                }
            } else if (request.type === 'execute_agent') {
                // Execute agent with task
                if (!llmManager || !trajectoryStore) {
                    sendResponse({ error: 'Agent not initialized. Ensure LLM is configured.' });
                    return;
                }

                // Check if LLM is ready
                const llmStatus = llmManager.getStatus();
                if (!llmStatus.ready) {
                    sendResponse({ error: 'LLM not configured. Please configure an LLM first.' });
                    return;
                }

                try {
                    agentState.active = true;
                    agentState.task = request.task;
                    agentState.startTime = Date.now();

                    // Get agent config from request or storage
                    const agentConfig = request.config || {
                        maxIterations: 10,
                        temperature: 0.7,
                        retrievalCount: 3,
                    };

                    // Add background.js functions to config
                    agentConfig.scanPage = scanPage;
                    agentConfig.executeOil = executeOilCommand;

                    // Create agent instance
                    ralphAgent = createRalphAgent(agentConfig);

                    console.log('[Oryn-W] Starting agent execution:', request.task);

                    // Execute the task
                    const result = await ralphAgent.execute(request.task, request.tabId);

                    agentState.active = false;
                    agentState.history = result.history;

                    console.log('[Oryn-W] Agent execution completed:', result);

                    sendResponse(result);
                } catch (error) {
                    console.error('[Oryn-W] Agent execution failed:', error);
                    agentState.active = false;
                    sendResponse({ error: error.message, success: false });
                }
            } else if (request.type === 'agent_status') {
                // Get agent status
                const status = {
                    active: agentState.active,
                    task: agentState.task,
                    currentIteration: ralphAgent ? ralphAgent.currentIteration : 0,
                    maxIterations: ralphAgent ? ralphAgent.maxIterations : 10,
                    historyLength: agentState.history.length,
                    llmReady: llmManager ? llmManager.getStatus().ready : false,
                    trajectoryCount: trajectoryStore ? (await trajectoryStore.getStats()).total : 0,
                };
                sendResponse(status);
            } else if (request.type === 'trajectory_get_all') {
                // Get all trajectories
                if (!trajectoryStore) {
                    sendResponse({ error: 'Trajectory store not initialized' });
                    return;
                }

                try {
                    const trajectories = await trajectoryStore.getAll(request.filter || {});
                    sendResponse({ success: true, trajectories });
                } catch (error) {
                    console.error('[Oryn-W] Failed to get trajectories:', error);
                    sendResponse({ error: error.message });
                }
            } else if (request.type === 'trajectory_delete') {
                // Delete a trajectory
                if (!trajectoryStore) {
                    sendResponse({ error: 'Trajectory store not initialized' });
                    return;
                }

                try {
                    await trajectoryStore.delete(request.id);
                    sendResponse({ success: true });
                } catch (error) {
                    console.error('[Oryn-W] Failed to delete trajectory:', error);
                    sendResponse({ error: error.message });
                }
            } else if (request.type === 'trajectory_clear') {
                // Clear all trajectories
                if (!trajectoryStore) {
                    sendResponse({ error: 'Trajectory store not initialized' });
                    return;
                }

                try {
                    await trajectoryStore.clear();
                    sendResponse({ success: true });
                } catch (error) {
                    console.error('[Oryn-W] Failed to clear trajectories:', error);
                    sendResponse({ error: error.message });
                }
            } else if (request.type === 'trajectory_export') {
                // Export trajectories
                if (!trajectoryStore) {
                    sendResponse({ error: 'Trajectory store not initialized' });
                    return;
                }

                try {
                    const json = await trajectoryStore.export();
                    sendResponse({ success: true, data: json });
                } catch (error) {
                    console.error('[Oryn-W] Failed to export trajectories:', error);
                    sendResponse({ error: error.message });
                }
            } else if (request.type === 'trajectory_import') {
                // Import trajectories
                if (!trajectoryStore) {
                    sendResponse({ error: 'Trajectory store not initialized' });
                    return;
                }

                try {
                    const count = await trajectoryStore.import(request.data);
                    sendResponse({ success: true, count });
                } catch (error) {
                    console.error('[Oryn-W] Failed to import trajectories:', error);
                    sendResponse({ error: error.message });
                }
            } else if (request.type === 'trajectory_stats') {
                // Get trajectory statistics
                if (!trajectoryStore) {
                    sendResponse({ error: 'Trajectory store not initialized' });
                    return;
                }

                try {
                    const stats = await trajectoryStore.getStats();
                    sendResponse({ success: true, stats });
                } catch (error) {
                    console.error('[Oryn-W] Failed to get trajectory stats:', error);
                    sendResponse({ error: error.message });
                }
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

// Check if a command requires page scanning
function requiresScan(oil) {
    const cmd = oil.trim().toLowerCase();

    // Commands that DON'T need scanning (navigation and simple queries)
    const noScanCommands = ['goto', 'back', 'forward', 'refresh', 'url', 'title'];

    for (const noScanCmd of noScanCommands) {
        if (cmd === noScanCmd || cmd.startsWith(noScanCmd + ' ')) {
            return false;
        }
    }

    return true;
}

// Create minimal scan for commands that don't need element resolution
async function createMinimalScan(tabId) {
    try {
        const tab = await chrome.tabs.get(tabId);
        return {
            page: {
                url: tab.url || '',
                title: tab.title || '',
                viewport: { width: 0, height: 0 },
                scroll: { x: 0, y: 0, maxX: 0, maxY: 0 },
                ready_state: null
            },
            elements: [],
            stats: { total: 0, scanned: 0, iframes: null },
            patterns: null,
            changes: null,
            available_intents: null,
            full_mode: false,
            settings_applied: null,
            timing: null
        };
    } catch (error) {
        console.error('[Oryn-W] Failed to create minimal scan:', error);
        throw error;
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

        // Determine if we need a full scan or minimal scan
        let scanToUse;
        if (requiresScan(oil)) {
            // Commands like click, type, etc. need element resolution
            if (!currentScan) {
                console.log('[Oryn-W] Getting fresh scan for element resolution...');
                const scan = await scanPage(tabId);
                handleScanComplete(scan);
                scanToUse = currentScan;
            } else {
                scanToUse = currentScan;
            }
        } else {
            // Navigation commands don't need element resolution
            console.log('[Oryn-W] Using minimal scan for navigation command');
            scanToUse = await createMinimalScan(tabId);
        }

        // Update WASM with the scan
        orynCore.updateScan(JSON.stringify(scanToUse));

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

// Check if a page is valid for content scripts
async function isValidPage(tabId) {
    try {
        const tab = await chrome.tabs.get(tabId);
        const url = tab.url;

        // Pages where content scripts cannot run
        const invalidProtocols = ['chrome:', 'chrome-extension:', 'edge:', 'about:', 'data:'];
        const invalidPages = ['chrome.google.com/webstore'];

        // Check protocol
        for (const protocol of invalidProtocols) {
            if (url.startsWith(protocol)) {
                return {
                    valid: false,
                    reason: `Content scripts cannot run on ${protocol} pages`,
                    suggestion: 'Please navigate to a regular website (http:// or https://)'
                };
            }
        }

        // Check specific pages
        for (const page of invalidPages) {
            if (url.includes(page)) {
                return {
                    valid: false,
                    reason: 'Content scripts cannot run on Chrome Web Store pages',
                    suggestion: 'Please navigate to a regular website'
                };
            }
        }

        return { valid: true };
    } catch (error) {
        return {
            valid: false,
            reason: 'Could not access tab information',
            suggestion: 'Please ensure you have an active tab'
        };
    }
}

// Ensure content script is injected
async function ensureContentScript(tabId) {
    try {
        // Try to ping the content script
        await chrome.tabs.sendMessage(tabId, { action: 'ping' });
        return true;
    } catch (error) {
        // Content script not loaded, try to inject it
        console.log('[Oryn-W] Content script not loaded, attempting injection...');

        try {
            await chrome.scripting.executeScript({
                target: { tabId: tabId },
                files: ['suppress_alerts.js', 'scanner.js', 'content.js']
            });

            // Wait a bit for scripts to initialize
            await new Promise(resolve => setTimeout(resolve, 500));

            // Try ping again
            await chrome.tabs.sendMessage(tabId, { action: 'ping' });
            console.log('[Oryn-W] Content script injected successfully');
            return true;
        } catch (injectError) {
            console.error('[Oryn-W] Failed to inject content script:', injectError);
            return false;
        }
    }
}

// Scan the page
async function scanPage(tabId) {
    console.log('[Oryn-W] Scanning page...');

    try {
        // Check if page is valid
        const pageCheck = await isValidPage(tabId);
        if (!pageCheck.valid) {
            throw new Error(`${pageCheck.reason}. ${pageCheck.suggestion}`);
        }

        // Ensure content script is loaded
        const scriptReady = await ensureContentScript(tabId);
        if (!scriptReady) {
            throw new Error('Content script could not be loaded. This page may not allow extensions to run.');
        }

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

        // Provide user-friendly error message
        if (error.message.includes('Receiving end does not exist')) {
            throw new Error('Cannot access this page. Please navigate to a regular website (not chrome:// or extension pages).');
        }

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

        case 'screenshot': {
            const dataUrl = await chrome.tabs.captureVisibleTab();
            return { success: true, data: dataUrl };
        }

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
