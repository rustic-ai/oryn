// Popup UI for Oryn-W extension

const statusBadge = document.getElementById('status-badge');
const wasmStatus = document.getElementById('wasm-status');
const commandInput = document.getElementById('command-input');
const btnExecute = document.getElementById('btn-execute');
const btnLogs = document.getElementById('btn-logs');
const statusMessage = document.getElementById('status-message');

// Update UI to show error state
function setErrorState(message) {
    statusBadge.textContent = 'Error';
    statusBadge.className = 'status-badge status-error';
    wasmStatus.textContent = message;
}

// Check WASM status on load
async function checkStatus() {
    try {
        const response = await chrome.runtime.sendMessage({ type: 'get_status' });

        if (response.wasmInitialized) {
            statusBadge.textContent = 'Ready';
            statusBadge.className = 'status-badge status-ready';
            wasmStatus.textContent = 'WASM engine ready';
            commandInput.disabled = false;
            btnExecute.disabled = false;
        } else {
            setErrorState('WASM failed to initialize');
        }
    } catch (error) {
        console.error('Failed to check status:', error);
        setErrorState('Connection error');
    }
}

// Execute command
async function executeCommand() {
    const command = commandInput.value.trim();
    if (!command) {
        showStatus('Please enter a command', 'error');
        return;
    }

    btnExecute.disabled = true;
    btnExecute.textContent = 'Executing...';
    hideStatus();

    try {
        // Get active tab
        const tabs = await chrome.tabs.query({ active: true, currentWindow: true });
        if (!tabs[0]) {
            showStatus('No active tab', 'error');
            return;
        }

        // Send command to background with tabId
        const response = await chrome.runtime.sendMessage({
            type: 'execute_oil',
            oil: command,
            tabId: tabs[0].id,
        });

        if (response.error) {
            showStatus(`Error: ${response.error}`, 'error');
        } else if (response.success) {
            showStatus('Command executed successfully', 'success');
            commandInput.value = '';
        } else {
            showStatus('Unexpected response', 'error');
        }
    } catch (error) {
        console.error('Command execution error:', error);
        showStatus(`Error: ${error.message}`, 'error');
    } finally {
        btnExecute.disabled = false;
        btnExecute.textContent = 'Execute';
    }
}

// Show status message
function showStatus(message, type) {
    statusMessage.textContent = message;
    statusMessage.className = type === 'success' ? 'status-message-success' : 'status-message-error';
    statusMessage.classList.remove('hidden');

    // Auto-hide after 3 seconds
    setTimeout(hideStatus, 3000);
}

// Hide status message
function hideStatus() {
    statusMessage.classList.add('hidden');
}

// Open sidepanel
async function openSidepanel() {
    try {
        const tabs = await chrome.tabs.query({ active: true, currentWindow: true });
        if (tabs[0]) {
            await chrome.sidePanel.open({ tabId: tabs[0].id });
        }
    } catch (error) {
        console.error('Failed to open sidepanel:', error);
    }
}

// Event listeners
btnExecute.addEventListener('click', executeCommand);
commandInput.addEventListener('keypress', (e) => {
    if (e.key === 'Enter') {
        executeCommand();
    }
});
btnLogs.addEventListener('click', openSidepanel);

// Initialize
checkStatus();

// Re-check status every 2 seconds
setInterval(checkStatus, 2000);
