// Sidepanel for Oryn-W extension

const logContainer = document.getElementById('log-container');
const btnClear = document.getElementById('btn-clear');
const wasmStatus = document.getElementById('wasm-status');
const scanStatus = document.getElementById('scan-status');

// Log storage
const logs = [];
const MAX_LOGS = 500;

// Add log entry
function addLog(message, type = 'info') {
    const timestamp = new Date().toLocaleTimeString();

    logs.push({ timestamp, message, type });

    // Trim logs if too many
    if (logs.length > MAX_LOGS) {
        logs.shift();
    }

    renderLogs();
}

// Create a log entry element
function createLogEntry(log) {
    const entry = document.createElement('div');
    entry.className = `log-entry log-${log.type}`;

    const time = document.createElement('span');
    time.className = 'log-time';
    time.textContent = log.timestamp;

    const msg = document.createElement('span');
    msg.className = 'log-msg';
    msg.textContent = log.message;

    entry.appendChild(time);
    entry.appendChild(msg);
    return entry;
}

// Render logs
function renderLogs() {
    logContainer.innerHTML = '';
    logs.forEach(log => logContainer.appendChild(createLogEntry(log)));
    logContainer.scrollTop = logContainer.scrollHeight;
}

// Clear logs
function clearLogs() {
    logs.length = 0;
    renderLogs();
}

// Set status element state
function setStatusElement(element, isReady, readyText, notReadyText) {
    element.textContent = isReady ? readyText : notReadyText;
    element.className = isReady ? 'status-value ready' : 'status-value';
}

// Update status
async function updateStatus() {
    try {
        const response = await chrome.runtime.sendMessage({ type: 'get_status' });

        setStatusElement(wasmStatus, response.wasmInitialized, 'Ready', 'Error');
        if (!response.wasmInitialized) {
            wasmStatus.className = 'status-value error';
        }

        setStatusElement(scanStatus, response.hasScan, 'Loaded', 'Not loaded');
    } catch (error) {
        wasmStatus.textContent = 'Error';
        wasmStatus.className = 'status-value error';
        addLog(`Failed to update status: ${error.message}`, 'error');
    }
}

// Listen for console messages from background
chrome.runtime.onMessage.addListener((request, sender, sendResponse) => {
    if (request.type === 'log') {
        addLog(request.message, request.level || 'info');
    }
    sendResponse({ ok: true });
});

// Event listeners
btnClear.addEventListener('click', clearLogs);

// Initialize
addLog('[Oryn-W] Sidepanel initialized', 'info');
updateStatus();

// Update status periodically
setInterval(updateStatus, 2000);

// Intercept console logs (for debugging)
function wrapConsoleMethod(method, type) {
    const original = console[method];
    console[method] = function (...args) {
        original.apply(console, args);
        addLog(args.join(' '), type);
    };
}

wrapConsoleMethod('log', 'info');
wrapConsoleMethod('error', 'error');
