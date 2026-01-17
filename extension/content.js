// Checks if Lemmascope scanner is loaded
function isScannerLoaded() {
    return window.Lemmascope && typeof window.Lemmascope.process === 'function';
}

// Prepare to receive messages
chrome.runtime.onMessage.addListener((request, sender, sendResponse) => {
    // console.log("Content script received request:", request);

    if (!isScannerLoaded()) {
        console.error("Lemmascope Scanner not loaded on page.");
        sendResponse({ ok: false, error: "Scanner not loaded", code: "SCANNER_MISSING" });
        return true;
    }

    // Route message to Lemmascope.process
    (async () => {
        try {
            // Compatibility: Mapping action to cmd if needed
            if (request.action && !request.cmd) {
                request.cmd = request.action;
            }

            // Pass the entire request object to the scanner
            const result = await window.Lemmascope.process(request);

            // Ensure result is an object/protocol response
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

function remoteLog(msg) {
    console.log("[CONTENT] " + msg);
    // Use relative URL to avoid CORS (since we are on localhost:3000)
    fetch("/log?msg=" + encodeURIComponent("[CONTENT] " + msg)).catch(() => { });
}

remoteLog("Lemmascope Content Script Initialized on " + window.location.href);
chrome.runtime.sendMessage({ type: "ping" });
