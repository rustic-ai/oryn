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

// Render logs
function renderLogs() {
    logContainer.innerHTML = '';

    logs.forEach(log => {
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
        logContainer.appendChild(entry);
    });

    // Auto-scroll to bottom
    logContainer.scrollTop = logContainer.scrollHeight;
}

// Clear logs
function clearLogs() {
    logs.length = 0;
    renderLogs();
}

// Update status
async function updateStatus() {
    try {
        const response = await chrome.runtime.sendMessage({ type: 'get_status' });

        if (response.wasmInitialized) {
            wasmStatus.textContent = 'Ready';
            wasmStatus.className = 'status-value ready';
        } else {
            wasmStatus.textContent = 'Error';
            wasmStatus.className = 'status-value error';
        }

        if (response.hasScan) {
            scanStatus.textContent = 'Loaded';
            scanStatus.className = 'status-value ready';
        } else {
            scanStatus.textContent = 'Not loaded';
            scanStatus.className = 'status-value';
        }
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
const originalConsoleLog = console.log;
const originalConsoleError = console.error;

console.log = function (...args) {
    originalConsoleLog.apply(console, args);
    addLog(args.join(' '), 'info');
};

console.error = function (...args) {
    originalConsoleError.apply(console, args);
    addLog(args.join(' '), 'error');
};
