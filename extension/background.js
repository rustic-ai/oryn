const WEBSOCKET_URL = "ws://127.0.0.1:9001";
let socket = null;

function remoteLog(msg) {
    console.log(msg);
    fetch("http://127.0.0.1:3000/log?msg=" + encodeURIComponent(msg)).catch(e => console.error("FETCH ERROR: " + e.message));
}

function connect() {
    if (socket && (socket.readyState === WebSocket.OPEN || socket.readyState === WebSocket.CONNECTING)) {
        return;
    }
    remoteLog("LEMMASCOPE: Attempting connection to " + WEBSOCKET_URL);
    socket = new WebSocket(WEBSOCKET_URL);

    socket.onopen = () => {
        remoteLog("Connected to Oryn Server");
    };

    socket.onmessage = (event) => {
        remoteLog("Received command: " + event.data);
        try {
            const command = JSON.parse(event.data);

            // Handle navigation commands at the background level (not content script)
            if (command.action === "navigate" && command.url) {
                handleNavigate(command.url);
                return;
            }
            if (command.action === "back") {
                handleBack();
                return;
            }

            // Forward other commands to content script
            chrome.tabs.query({ active: true }, (tabs) => {
                let activeTab = tabs[0];
                if (!activeTab) {
                    chrome.tabs.query({}, (allTabs) => {
                        if (allTabs.length > 0) {
                            sendCommandToTab(allTabs[0], command);
                        } else {
                            remoteLog("No tabs found at all");
                        }
                    });
                } else {
                    sendCommandToTab(activeTab, command);
                }
            });
        } catch (e) {
            remoteLog("Failed to parse command: " + e.message);
        }
    };

    function handleNavigate(url) {
        remoteLog("Navigating to: " + url);
        chrome.tabs.query({ active: true }, (tabs) => {
            let tab = tabs[0];
            if (!tab) {
                chrome.tabs.query({}, (allTabs) => {
                    if (allTabs.length > 0) {
                        doNavigate(allTabs[0].id, url);
                    } else {
                        sendResponse({ status: "error", code: "NO_TAB", message: "No tabs found" });
                    }
                });
            } else {
                doNavigate(tab.id, url);
            }
        });
    }

    function doNavigate(tabId, url) {
        chrome.tabs.update(tabId, { url: url }, (tab) => {
            if (chrome.runtime.lastError) {
                sendResponse({ status: "error", code: "NAVIGATE_ERROR", message: chrome.runtime.lastError.message });
                return;
            }
            // Wait for the page to load
            chrome.tabs.onUpdated.addListener(function listener(updatedTabId, changeInfo) {
                if (updatedTabId === tabId && changeInfo.status === "complete") {
                    chrome.tabs.onUpdated.removeListener(listener);
                    remoteLog("Navigation complete to: " + url);
                    sendResponse({ status: "ok", url: url });
                }
            });
        });
    }

    function handleBack() {
        remoteLog("Going back in history");
        chrome.tabs.query({ active: true }, (tabs) => {
            let tab = tabs[0];
            if (!tab) {
                chrome.tabs.query({}, (allTabs) => {
                    if (allTabs.length > 0) {
                        doGoBack(allTabs[0].id);
                    } else {
                        sendResponse({ status: "error", code: "NO_TAB", message: "No tabs found" });
                    }
                });
            } else {
                doGoBack(tab.id);
            }
        });
    }

    function doGoBack(tabId) {
        chrome.tabs.goBack(tabId, () => {
            if (chrome.runtime.lastError) {
                sendResponse({ status: "error", code: "BACK_ERROR", message: chrome.runtime.lastError.message });
                return;
            }
            // Wait for the page to load
            chrome.tabs.onUpdated.addListener(function listener(updatedTabId, changeInfo) {
                if (updatedTabId === tabId && changeInfo.status === "complete") {
                    chrome.tabs.onUpdated.removeListener(listener);
                    remoteLog("Back navigation complete");
                    sendResponse({ status: "ok" });
                }
            });
        });
    }

    function sendResponse(response) {
        if (socket && socket.readyState === WebSocket.OPEN) {
            socket.send(JSON.stringify(response));
        }
    }

    function sendCommandToTab(tab, command) {
        remoteLog("Sending to tab: " + tab.id + " " + tab.url);
        chrome.tabs.sendMessage(tab.id, command, (response) => {
            if (chrome.runtime.lastError) {
                remoteLog("Error sending to tab: " + chrome.runtime.lastError.message);
                if (socket && socket.readyState === WebSocket.OPEN) {
                    socket.send(JSON.stringify({
                        status: "error",
                        code: "extension_error",
                        message: chrome.runtime.lastError.message
                    }));
                }
            } else if (response) {
                remoteLog("Received response from tab: " + JSON.stringify(response));
                if (socket && socket.readyState === WebSocket.OPEN) {
                    socket.send(JSON.stringify(response));
                }
            }
        });
    }

    socket.onclose = () => {
        remoteLog("Disconnected. Reconnecting in 1s...");
        setTimeout(connect, 1000);
    };

    socket.onerror = (e) => {
        remoteLog("WebSocket error: " + e.message);
    };
}

chrome.runtime.onMessage.addListener((message, sender, sendResponse) => {
    remoteLog("Received message from content script: " + JSON.stringify(message));
    if (!socket || socket.readyState !== WebSocket.OPEN) {
        remoteLog("Re-initializing connection due to message...");
        connect();
    }
    if (socket && socket.readyState === WebSocket.OPEN) {
        if (message.type !== "ping") {
            socket.send(JSON.stringify(message));
        }
    }
});

chrome.runtime.onStartup.addListener(() => {
    remoteLog("OnStartup triggered");
    connect();
});

chrome.runtime.onInstalled.addListener(() => {
    remoteLog("OnInstalled triggered");
    connect();
});

chrome.tabs.onActivated.addListener(connect);
chrome.tabs.onUpdated.addListener(connect);

remoteLog("BACKGROUND SCRIPT LOADED");
connect();
setInterval(connect, 10000);
