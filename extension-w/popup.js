// Popup UI for Oryn-W extension

const wasmBadge = document.getElementById('wasm-badge');
const scanBadge = document.getElementById('scan-badge');
const btnOpenSidepanel = document.getElementById('btn-open-sidepanel');

// Update status badges
async function checkStatus() {
    try {
        const response = await chrome.runtime.sendMessage({ type: 'get_status' });

        // Update WASM badge
        if (response.wasmInitialized) {
            wasmBadge.textContent = 'Ready';
            wasmBadge.className = 'status-badge ready';
        } else {
            wasmBadge.textContent = 'Error';
            wasmBadge.className = 'status-badge error';
        }

        // Update scan badge
        if (response.hasScan) {
            scanBadge.textContent = 'Loaded';
            scanBadge.className = 'status-badge ready';
        } else {
            scanBadge.textContent = 'Not loaded';
            scanBadge.className = 'status-badge idle';
        }
    } catch (error) {
        console.error('Failed to check status:', error);
        wasmBadge.textContent = 'Error';
        wasmBadge.className = 'status-badge error';
    }
}

// Open sidepanel
async function openSidepanel() {
    try {
        const tabs = await chrome.tabs.query({ active: true, currentWindow: true });
        if (tabs[0]) {
            await chrome.sidePanel.open({ tabId: tabs[0].id });
            // Close popup after opening sidepanel
            window.close();
        }
    } catch (error) {
        console.error('Failed to open sidepanel:', error);
    }
}

// Event listeners
btnOpenSidepanel.addEventListener('click', openSidepanel);

// Initialize
checkStatus();

// Re-check status every 2 seconds
setInterval(checkStatus, 2000);
