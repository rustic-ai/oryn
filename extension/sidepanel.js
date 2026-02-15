const container = document.getElementById('log-container');
const clearBtn = document.getElementById('btn-clear');
const showAllCheckbox = document.getElementById('show-all');
const statusText = document.getElementById('status-text');

let activeTabId = null;
let logHistory = []; // { msg: string, tabId: number|null, timestamp: string }

// Initialize Active Tab
chrome.tabs.query({ active: true, currentWindow: true }, (tabs) => {
    if (tabs.length > 0) {
        updateActiveTab(tabs[0].id);
    }
});

function updateActiveTab(tabId) {
    activeTabId = tabId;
    statusText.textContent = tabId ? `Tab ${tabId}` : 'No Tab';
    renderLogs();
}

function renderLogs() {
    container.innerHTML = '';
    const showAll = showAllCheckbox.checked;

    logHistory.forEach(log => {
        // Filter Logic:
        // Show if "Show All" is checked
        // OR log has NO tabId (System message)
        // OR log.tabId matches activeTabId
        if (showAll || !log.tabId || log.tabId === activeTabId) {
            createLogElement(log);
        }
    });

    // Auto-scroll
    container.scrollTop = container.scrollHeight;
}

function createLogElement(log) {
    const entry = document.createElement('div');
    entry.className = 'log-entry';
    let msg = log.msg;

    if (log.type === 'error' || msg.toLowerCase().includes('error') || msg.toLowerCase().includes('fail')) {
        entry.classList.add('log-error');
    }

    // Highlight Tab ID if present
    const tabMatch = msg.match(/^\[Tab (\d+)\]/);
    if (tabMatch) {
        msg = msg.replace(tabMatch[0], `<span style="color:#2196F3; font-weight:bold">${tabMatch[0]}</span>`);
    } else if (msg.startsWith('[System]')) {
        msg = msg.replace('[System]', `<span style="color:#9C27B0; font-weight:bold">[System]</span>`);
    } else if (msg.startsWith('[CONTENT]')) {
        msg = msg.replace('[CONTENT]', `<span style="color:#FF9800; font-weight:bold">[CONTENT]</span>`);
    }

    const time = document.createElement('span');
    time.className = 'log-time';
    time.textContent = log.timestamp;

    const content = document.createElement('span');
    content.className = 'log-msg';
    content.innerHTML = msg;

    entry.appendChild(time);
    entry.appendChild(content);
    container.appendChild(entry);
}

function addLog(msg, tabId = null, type = 'info') {
    const log = {
        msg: msg,
        tabId: tabId,
        timestamp: new Date().toLocaleTimeString(),
        type: type
    };
    logHistory.push(log);

    // Optimization: If currently visible, just append instead of full re-render
    const showAll = showAllCheckbox.checked;
    if (showAll || !tabId || tabId === activeTabId) {
        createLogElement(log);
        container.scrollTop = container.scrollHeight;
    }
}

clearBtn.addEventListener('click', () => {
    logHistory = [];
    renderLogs();
});

showAllCheckbox.addEventListener('change', () => {
    renderLogs();
});

// Listen for messages
chrome.runtime.onMessage.addListener((message) => {
    if (message.type === 'log') {
        addLog(message.message, message.tabId);
    }
    if (message.type === 'TAB_SWITCHED') {
        updateActiveTab(message.tabId);
    }
});

addLog("[System] Log viewer initialized");
