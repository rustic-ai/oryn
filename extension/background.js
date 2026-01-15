const WEBSOCKET_URL = "ws://localhost:9001";
let socket = null;

function connect() {
    console.log("Connecting to Lemmascope Server at " + WEBSOCKET_URL);
    socket = new WebSocket(WEBSOCKET_URL);

    socket.onopen = () => {
        console.log("Connected to Lemmascope Server");
    };

    socket.onmessage = (event) => {
        console.log("Received command:", event.data);
        try {
            const command = JSON.parse(event.data);
            // Create a navigation function for 'execute' type commands or just forward everything?
            // Our backend sends ScannerRequest.
            // We forward to active tab.

            chrome.tabs.query({ active: true, currentWindow: true }, (tabs) => {
                if (tabs.length === 0) {
                    console.warn("No active tab to send command to");
                    // Could send error back?
                    return;
                }
                const activeTab = tabs[0];
                console.log("Sending to tab:", activeTab.id);

                // Ensure content script is ready? We assume it is via manifest injection.
                chrome.tabs.sendMessage(activeTab.id, command, (response) => {
                    if (chrome.runtime.lastError) {
                        console.error("Error sending to tab:", chrome.runtime.lastError.message);
                        // Try to report error to server?
                        if (socket && socket.readyState === WebSocket.OPEN) {
                            socket.send(JSON.stringify({
                                status: "error",
                                code: "extension_error",
                                message: chrome.runtime.lastError.message
                            }));
                        }
                    } else {
                        console.log("Tab responded immediately (allocating async response via port?)", response);
                        // If content script returns response directly, we send it.
                        // But content script usually needs to work async.
                        // Better pattern: Content script posts a runtime message back to background when done.
                    }
                });
            });

        } catch (e) {
            console.error("Failed to parse command:", e);
        }
    };

    socket.onclose = () => {
        console.log("Disconnected. Reconnecting in 3s...");
        setTimeout(connect, 3000);
    };

    socket.onerror = (e) => {
        console.error("WebSocket error:", e);
        socket.close();
    };
}

// Listen for messages from content script
chrome.runtime.onMessage.addListener((message, sender, sendResponse) => {
    console.log("Received message from content script:", message);
    if (socket && socket.readyState === WebSocket.OPEN) {
        // Forward to server
        socket.send(JSON.stringify(message));
    } else {
        console.warn("Socket not open, cannot forward response");
    }
    // sendResponse(true); // Keep channel open?
});

connect();
