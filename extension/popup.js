document.addEventListener('DOMContentLoaded', async () => {
    const urlInput = document.getElementById('ws-url');
    const connectBtn = document.getElementById('btn-connect');
    const disconnectBtn = document.getElementById('btn-disconnect');
    const statusBadge = document.getElementById('status-badge');

    // Get Current Tab ID
    const [tab] = await chrome.tabs.query({ active: true, currentWindow: true });
    const currentTabId = tab ? tab.id : null;

    if (!currentTabId) {
        statusBadge.textContent = "Error: No Tab";
        return;
    }

    // Load saved global settings (for default URL)
    const storage = await chrome.storage.local.get(['websocketUrl']);
    if (storage.websocketUrl) {
        urlInput.value = storage.websocketUrl;
    }

    // Subscribe to status changes from background
    chrome.runtime.onMessage.addListener((message) => {
        if (message.type === 'STATUS_CHANGE' && message.tabId === currentTabId) {
            updateUI(message.status);
            if (message.url) urlInput.value = message.url;
        }
    });

    // Request current status for THIS tab
    chrome.runtime.sendMessage({ type: 'GET_STATUS', tabId: currentTabId }, (response) => {
        if (response) {
            updateUI(response.status || 'disconnected');
            if (response.url) urlInput.value = response.url;
        }
    });

    connectBtn.addEventListener('click', () => {
        const url = urlInput.value.trim();
        if (!url) return;

        chrome.storage.local.set({ websocketUrl: url }); // Save as preference
        statusBadge.textContent = 'Connecting...';

        chrome.runtime.sendMessage({
            type: 'CONNECT',
            url: url,
            tabId: currentTabId
        });
    });

    disconnectBtn.addEventListener('click', () => {
        chrome.runtime.sendMessage({
            type: 'DISCONNECT',
            tabId: currentTabId
        });
    });

    function updateUI(status) {
        statusBadge.className = 'status-badge';
        if (status === 'connected_local') {
            statusBadge.textContent = 'Local';
            statusBadge.classList.add('status-connected-local');
            connectBtn.classList.add('hidden');
            disconnectBtn.classList.remove('hidden');
            urlInput.disabled = true;
        } else if (status === 'connected_remote') {
            statusBadge.textContent = 'Remote';
            statusBadge.classList.add('status-connected-remote');
            connectBtn.classList.add('hidden');
            disconnectBtn.classList.remove('hidden');
            urlInput.disabled = true;
        } else {
            statusBadge.textContent = 'Disconnected';
            statusBadge.classList.add('status-disconnected');
            connectBtn.classList.remove('hidden');
            disconnectBtn.classList.add('hidden');
            urlInput.disabled = false;
        }
    }
});
