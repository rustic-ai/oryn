// State
const connections = new Map(); // TabID -> { socket: WebSocket, url: string, status: string }
let defaultUrl = "ws://127.0.0.1:9001";

// Load saved default URL and Auto-Connect setting
chrome.storage.local.get(['websocketUrl', 'autoConnect'], (result) => {
    if (result.websocketUrl) {
        defaultUrl = result.websocketUrl;
    }
    // Note: autoConnect might be set by config loading below
});

// --- Config Loading ---

async function loadConfig() {
    try {
        const response = await fetch('config.json');
        if (response.ok) {
            const config = await response.json();

            // Update storage if config file is present
            if (config.websocketUrl) {
                defaultUrl = config.websocketUrl;
                chrome.storage.local.set({ websocketUrl: defaultUrl });
            }

            if (config.autoConnect !== undefined) {
                chrome.storage.local.set({ autoConnect: config.autoConnect });
                remoteLog(`[System] Config loaded: autoConnect=${config.autoConnect}`);
            }
        }
    } catch (e) {
        // Expected in production (file missing)
        remoteLog("[System] No config.json found, skipping.");
    }
}

// Ensure config is loaded before other init or alongside
loadConfig();

// --- State Helpers ---

function getConnection(tabId) {
    return connections.get(tabId) || { socket: null, url: null, status: 'disconnected' };
}

function updateConnectionState(tabId, updates) {
    const current = getConnection(tabId);
    const newState = { ...current, ...updates };

    // If we have a socket but status is disconnected, we might be cleaning up,
    // but usually status drives the UI.

    if (updates.socket !== undefined) {
        // If updating socket, ensure we store it
        newState.socket = updates.socket;
    }

    connections.set(tabId, newState);

    // Update UI if this is the active tab
    chrome.tabs.query({ active: true, currentWindow: true }, (tabs) => {
        if (tabs.length > 0 && tabs[0].id === tabId) {
            updateBadge(newState.status);
            // Notify popup
            chrome.runtime.sendMessage({
                type: 'STATUS_CHANGE',
                tabId: tabId,
                status: newState.status,
                url: newState.url
            }).catch(() => { });
        }
    });
}

function updateBadge(status) {
    let iconPath = "icons/icon-disconnected.svg";
    let badgeText = "";
    let badgeColor = "#999";

    if (status === 'connected_local') {
        iconPath = "icons/icon-local.svg";
        badgeText = "L";
        badgeColor = "#2196F3";
    } else if (status === 'connected_remote') {
        iconPath = "icons/icon-remote.svg";
        badgeText = "R";
        badgeColor = "#4CAF50";
    }

    chrome.action.setIcon({ path: iconPath });
    chrome.action.setBadgeText({ text: badgeText });
    chrome.action.setBadgeBackgroundColor({ color: badgeColor });
}

function remoteLog(msg, tabId = null) {
    console.log(msg);
    const prefix = tabId ? `[Tab ${tabId}] ` : `[System] `;
    chrome.runtime.sendMessage({
        type: 'log',
        message: prefix + msg,
        tabId: tabId
    }).catch(() => { });
}

// --- Connection Logic ---

function connect(tabId, url) {
    const existing = getConnection(tabId);
    if (existing.socket && (existing.socket.readyState === WebSocket.OPEN || existing.socket.readyState === WebSocket.CONNECTING)) {
        if (existing.url === url) return; // Already connected
        existing.socket.close();
    }

    if (!url) url = defaultUrl;

    remoteLog(`Connecting to ${url}`, tabId);
    updateConnectionState(tabId, { url: url, status: 'disconnected', socket: null });

    let socket;
    try {
        socket = new WebSocket(url);
    } catch (e) {
        remoteLog(`Invalid URL: ${url}`, tabId);
        return;
    }

    // Update state with new socket
    updateConnectionState(tabId, { socket: socket });

    socket.onopen = () => {
        remoteLog(`Connected`, tabId);
        const isLocal = url.includes("127.0.0.1") || url.includes("localhost");
        updateConnectionState(tabId, {
            status: isLocal ? 'connected_local' : 'connected_remote'
        });
    };

    socket.onmessage = (event) => {
        // remoteLog(`Received: ${event.data}`, tabId);
        try {
            const command = JSON.parse(event.data);

            // Handle navigation commands at the background level
            if (command.action === "navigate" && command.url) {
                handleNavigate(tabId, command.url);
                return;
            }
            if (command.action === "back") {
                handleBack(tabId);
                return;
            }

            // Forward to specific tab
            sendCommandToTab(tabId, command);

        } catch (e) {
            remoteLog(`Info: ${event.data}`, tabId);
        }
    };

    socket.onclose = () => {
        remoteLog(`Disconnected`, tabId);
        updateConnectionState(tabId, {
            status: 'disconnected',
            socket: null
        });
    };

    socket.onerror = (e) => {
        remoteLog(`Error: ${e.message}`, tabId);
    };
}

function disconnect(tabId) {
    const conn = getConnection(tabId);
    if (conn.socket) {
        conn.socket.close();
    }
    updateConnectionState(tabId, { status: 'disconnected', socket: null });
}

// --- Command Handlers ---

function handleNavigate(tabId, url) {
    remoteLog(`Navigating to ${url}`, tabId);
    chrome.tabs.update(tabId, { url: url }, (tab) => {
        if (chrome.runtime.lastError) {
            sendResponseToSocket(tabId, { status: "error", code: "NAVIGATE_ERROR", message: chrome.runtime.lastError.message });
            return;
        }
        chrome.tabs.onUpdated.addListener(function listener(updatedTabId, changeInfo) {
            if (updatedTabId === tabId && changeInfo.status === "complete") {
                chrome.tabs.onUpdated.removeListener(listener);
                remoteLog(`Navigation complete`, tabId);
                sendResponseToSocket(tabId, { status: "ok", url: url });
            }
        });
    });
}

function handleBack(tabId) {
    remoteLog("Going back", tabId);
    chrome.tabs.goBack(tabId, () => {
        if (chrome.runtime.lastError) {
            sendResponseToSocket(tabId, { status: "error", code: "BACK_ERROR", message: chrome.runtime.lastError.message });
            return;
        }
        chrome.tabs.onUpdated.addListener(function listener(updatedTabId, changeInfo) {
            if (updatedTabId === tabId && changeInfo.status === "complete") {
                chrome.tabs.onUpdated.removeListener(listener);
                remoteLog("Back complete", tabId);
                sendResponseToSocket(tabId, { status: "ok" });
            }
        });
    });
}

function sendResponseToSocket(tabId, response) {
    const conn = getConnection(tabId);
    if (conn.socket && conn.socket.readyState === WebSocket.OPEN) {
        conn.socket.send(JSON.stringify(response));
    }
}

function sendCommandToTab(tabId, command) {
    chrome.tabs.sendMessage(tabId, command, (response) => {
        if (chrome.runtime.lastError) {
            // Often usually means content script not ready or page loading
            // remoteLog("Tab msg error: " + chrome.runtime.lastError.message, tabId);
            // We could report this back to server
        } else if (response) {
            sendResponseToSocket(tabId, response);
        }
    });
}


// --- Listeners ---

// 1. Popup / global messages
chrome.runtime.onMessage.addListener((message, sender, sendResponse) => {
    // Determine Target Tab ID
    let targetTabId = null;

    if (message.type === 'CONNECT' || message.type === 'DISCONNECT' || message.type === 'GET_STATUS') {
        // These come from popup usually, so we need the 'active' tab ID passed or implied?
        // Popup should pass tabId. If not, we have to guess context, but Popup is separate window.
        if (message.tabId) {
            targetTabId = message.tabId;
        } else if (sender.tab) {
            targetTabId = sender.tab.id;
        }
    } else {
        // Content script message
        if (sender.tab) {
            targetTabId = sender.tab.id;
        }
    }

    if (!targetTabId && message.type !== 'log') {
        // If no valid tabId, we can't do much for connection logic unless we default to active tab?
        // Popup.js will be updated to send tabId.
        return;
    }

    if (message.type === 'CONNECT') {
        connect(targetTabId, message.url);
        // Save as default for future convenience?
        chrome.storage.local.set({ websocketUrl: message.url });
        defaultUrl = message.url;
        return;
    }

    if (message.type === 'DISCONNECT') {
        disconnect(targetTabId);
        return;
    }

    if (message.type === 'GET_STATUS') {
        const conn = getConnection(targetTabId);
        sendResponse({ status: conn.status, url: conn.url || defaultUrl });
        return;
    }

    if (message.type === 'log') {
        // Broadcast
        remoteLog("[CONTENT] " + message.message, targetTabId);
        return;
    }

    // Forward content script responses/events to socket
    if (sender.tab) {
        const conn = getConnection(targetTabId);
        if (conn.socket && conn.socket.readyState === WebSocket.OPEN) {
            if (message.type !== "ping") {
                conn.socket.send(JSON.stringify(message));
            }
        }
    }
});

// 2. Tab Lifecycle
chrome.tabs.onRemoved.addListener((tabId) => {
    if (connections.has(tabId)) {
        remoteLog(`Tab closed, cleaning up connection`, tabId);
        disconnect(tabId);
        connections.delete(tabId);
    }
});

// 3. Tab Switching (Update Badge)
chrome.tabs.onActivated.addListener((activeInfo) => {
    const conn = getConnection(activeInfo.tabId);
    updateBadge(conn.status);

    // Notify Sidepanel
    chrome.runtime.sendMessage({
        type: 'TAB_SWITCHED',
        tabId: activeInfo.tabId
    }).catch(() => { });
});

chrome.tabs.onUpdated.addListener((tabId, changeInfo, tab) => {
    if (tab.active) {
        const conn = getConnection(tabId);
        updateBadge(conn.status);
    }

    // Auto-Connect Logic
    if (changeInfo.status === 'complete') {
        chrome.storage.local.get(['autoConnect'], (result) => {
            if (result.autoConnect) {
                const conn = getConnection(tabId);
                if (conn.status === 'disconnected') {
                    remoteLog(`[AutoConnect] Triggered for Tab ${tabId}`, tabId);
                    connect(tabId, defaultUrl);
                }
            }
        });
    }
});

chrome.runtime.onInstalled.addListener(() => {
    chrome.sidePanel.setOptions({
        path: 'sidepanel.html',
        enabled: true
    });
});

remoteLog("BACKGROUND SCRIPT LOADED (Multi-Tab)");
