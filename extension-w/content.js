// Checks if Oryn scanner is loaded
function isScannerLoaded() {
    return window.Oryn && typeof window.Oryn.process === 'function';
}

// Log message to console and send to background
function remoteLog(msg) {
    const prefixedMsg = "[CONTENT] " + msg;
    console.log(prefixedMsg);
    chrome.runtime.sendMessage({ type: "log", message: prefixedMsg });
}

// Handle incoming messages
chrome.runtime.onMessage.addListener((request, sender, sendResponse) => {
    if (!isScannerLoaded()) {
        console.error("Oryn Scanner not loaded on page.");
        sendResponse({ ok: false, error: "Scanner not loaded", code: "SCANNER_MISSING" });
        return true;
    }

    // Route message to Oryn.process
    (async () => {
        try {
            // Compatibility: Map action to cmd if needed
            if (request.action && !request.cmd) {
                request.cmd = request.action;
            }

            const result = await window.Oryn.process(request);
            sendResponse(result || { status: "ok" });
        } catch (e) {
            console.error("Execution error:", e);
            sendResponse({
                status: "error",
                message: e.msg || e.message || "Unknown error",
                code: e.code || "EXECUTION_ERROR",
                details: e
            });
        }
    })();

    return true; // Keep channel open for async sendResponse
});

// Initialize
remoteLog("Oryn Content Script Initialized on " + window.location.href);
chrome.runtime.sendMessage({ type: "ping" });
