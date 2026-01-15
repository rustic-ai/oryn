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
    try {
        const result = window.Lemmascope.process(request.action, request);

        // Ensure result is an object/protocol response
        sendResponse(result || { ok: true });

    } catch (e) {
        console.error("Execution error:", e);
        sendResponse({
            ok: false,
            error: e.msg || e.message || "Unknown error",
            code: e.code || "EXECUTION_ERROR",
            details: e
        });
    }

    return true; // Indicate async response potentially
});

console.log("Lemmascope Content Script Initialized");
