/**
 * Offscreen Document for LLM Operations
 *
 * This runs in a window context (not service worker) so it can use dynamic imports
 * for WebLLM and wllama libraries.
 */

import { LLMManager } from './llm/llm_manager.js';

console.log('[Offscreen] Starting LLM offscreen document');

// Send status to background (so it can be monitored)
function sendStatus(status, data = {}) {
    chrome.runtime.sendMessage({
        type: 'offscreen_status',
        status,
        data,
        timestamp: Date.now()
    }).catch(err => {
        console.error('[Offscreen] Failed to send status:', err);
    });
}

sendStatus('starting', { message: 'Offscreen document starting' });

// Create LLM manager instance (in window context, so dynamic imports work!)
const llmManager = new LLMManager();

// Initialize the LLM manager
let initializationPromise = (async () => {
    try {
        sendStatus('initializing', { message: 'Starting LLM Manager initialization' });
        await llmManager.initialize();
        console.log('[Offscreen] LLM Manager initialized with', llmManager.availableAdapters.length, 'adapters');
        sendStatus('initialized', {
            message: 'LLM Manager initialized',
            adapterCount: llmManager.availableAdapters.length
        });
    } catch (error) {
        console.error('[Offscreen] Failed to initialize LLM manager:', error);
        sendStatus('error', { message: 'Initialization failed', error: error.message, stack: error.stack });
        throw error;
    }
})();

// Handle messages from background
chrome.runtime.onMessage.addListener((request, sender, sendResponse) => {
    // Only handle messages prefixed with 'offscreen_' - ignore all others
    if (!request.type || !request.type.startsWith('offscreen_')) {
        return false; // Let other listeners handle it
    }

    console.log('[Offscreen] Received message:', request.type);
    sendStatus('message_received', { type: request.type });

    // All handlers are async, so we return true and use sendResponse later
    (async () => {
        try {
            // Wait for initialization to complete
            sendStatus('waiting_init', { message: 'Waiting for initialization' });
            await initializationPromise;
            sendStatus('init_complete', { message: 'Initialization complete, processing message' });

            if (request.type === 'offscreen_llm_set_adapter') {
                // Set active adapter (in window context, so dynamic imports work)
                const { name, model, config } = request;
                console.log('[Offscreen] Setting adapter:', name, model);
                sendStatus('setting_adapter', { name, model });
                await llmManager.setActiveAdapter(name, model, config || {});
                sendStatus('adapter_set', { name, model });
                sendResponse({ success: true });

            } else if (request.type === 'offscreen_llm_prompt') {
                // Send prompt to LLM
                const { messages, options } = request;
                console.log('[Offscreen] Sending prompt to LLM');
                const response = await llmManager.prompt(messages, options || {});
                sendResponse({ success: true, response });

            } else if (request.type === 'offscreen_llm_status') {
                // Get LLM status
                const status = llmManager.getStatus();
                sendResponse({ success: true, status });

            } else if (request.type === 'offscreen_llm_list_adapters') {
                // List available adapters
                const adapters = llmManager.listAdapters();
                sendResponse({ success: true, adapters });

            } else {
                sendResponse({ error: 'Unknown message type: ' + request.type });
            }
        } catch (error) {
            console.error('[Offscreen] Error handling message:', error);
            sendResponse({ error: error.message, stack: error.stack });
        }
    })();

    return true; // Keep channel open for async response
});

console.log('[Offscreen] Ready to handle LLM operations');
sendStatus('ready', { message: 'Offscreen document ready to handle LLM operations' });
