// Sidepanel for Oryn-W extension - Chat/Command Interface

const chatContainer = document.getElementById('chat-container');
const emptyState = document.getElementById('empty-state');
const commandInput = document.getElementById('command-input');
const btnExecute = document.getElementById('btn-execute');
const btnClear = document.getElementById('btn-clear');
const wasmBadge = document.getElementById('wasm-badge');
const scanBadge = document.getElementById('scan-badge');

// Message storage
const messages = [];
const MAX_MESSAGES = 100;

// Add message to chat
function addMessage(text, type = 'system') {
    const timestamp = new Date().toLocaleTimeString();

    messages.push({ text, type, timestamp });

    // Trim messages if too many
    if (messages.length > MAX_MESSAGES) {
        messages.shift();
    }

    renderMessages();
}

// Create a message element
function createMessageElement(msg) {
    const message = document.createElement('div');
    message.className = `message message-${msg.type}`;

    const text = document.createElement('div');
    text.textContent = msg.text;

    const time = document.createElement('div');
    time.className = 'message-time';
    time.textContent = msg.timestamp;

    message.appendChild(text);
    message.appendChild(time);

    return message;
}

// Render all messages
function renderMessages() {
    // Hide empty state if there are messages
    if (messages.length > 0) {
        emptyState.style.display = 'none';
    } else {
        emptyState.style.display = 'flex';
    }

    // Clear existing messages (except empty state)
    const existingMessages = chatContainer.querySelectorAll('.message');
    existingMessages.forEach(msg => msg.remove());

    // Add all messages
    messages.forEach(msg => {
        chatContainer.appendChild(createMessageElement(msg));
    });

    // Scroll to bottom
    chatContainer.scrollTop = chatContainer.scrollHeight;
}

// Clear all messages
function clearMessages() {
    messages.length = 0;
    renderMessages();
}

// Format result for display - matches oryn-h CLI output
function formatResult(response) {
    // If there's a message, use it
    if (response.message) {
        return response.message;
    }

    // If there's result data, format it
    if (response.result) {
        const result = response.result;

        // Scan result - format as canonical OIL text output
        if (result.page && result.elements && result.stats) {
            return formatScanResult(result);
        }

        // String result (from execute, url, title, etc.)
        if (typeof result === 'string') {
            return result;
        }

        // Extraction result
        if (result.values || result.value !== undefined) {
            return `Value: ${JSON.stringify(result)}`;
        }

        // Object result - show as "Value: {json}"
        if (typeof result === 'object') {
            return `Value: ${JSON.stringify(result)}`;
        }
    }

    return 'Command executed successfully';
}

// Format scan result as canonical OIL text output
function formatScanResult(scan) {
    let output = `@ ${scan.page.url} "${scan.page.title}"\n`;

    for (const el of scan.elements) {
        // Format type/role
        const typeStr = el.role ? `${el.type}/${el.role}` : el.type;

        // Get label (text or label attribute)
        const label = el.text || el.label || '';

        // Build state flags
        const flags = [];
        if (el.state.checked) flags.push('checked');
        if (el.state.selected) flags.push('selected');
        if (el.state.disabled) flags.push('disabled');
        if (el.state.readonly) flags.push('readonly');
        if (el.state.required) flags.push('required');
        if (el.state.primary) flags.push('primary');

        const flagsStr = flags.length > 0 ? ` {${flags.join(', ')}}` : '';

        output += `[${el.id}] ${typeStr} "${label}"${flagsStr}\n`;
    }

    // Add patterns if detected
    if (scan.patterns) {
        const detected = [];
        if (scan.patterns.login) detected.push('Login Form');
        if (scan.patterns.search) detected.push('Search Box');
        if (scan.patterns.pagination) detected.push('Pagination');
        if (scan.patterns.modal) detected.push('Modal');
        if (scan.patterns.cookie_banner) detected.push('Cookie Banner');

        if (detected.length > 0) {
            output += '\nPatterns:';
            for (const p of detected) {
                output += `\n- ${p}`;
            }
        }
    }

    return output;
}


// Update status badges
async function updateStatus() {
    try {
        const response = await chrome.runtime.sendMessage({ type: 'get_status' });

        // Update WASM badge
        if (response.wasmInitialized) {
            wasmBadge.textContent = 'WASM: Ready';
            wasmBadge.className = 'status-badge ready';
            commandInput.disabled = false;
            btnExecute.disabled = false;
        } else {
            wasmBadge.textContent = 'WASM: Error';
            wasmBadge.className = 'status-badge error';
            commandInput.disabled = true;
            btnExecute.disabled = true;
        }

        // Update scan badge
        if (response.hasScan) {
            scanBadge.textContent = 'Scan: Loaded';
            scanBadge.className = 'status-badge ready';
        } else {
            scanBadge.textContent = 'Scan: Not loaded';
            scanBadge.className = 'status-badge idle';
        }
    } catch (error) {
        console.error('Failed to update status:', error);
        wasmBadge.textContent = 'WASM: Error';
        wasmBadge.className = 'status-badge error';
        commandInput.disabled = true;
        btnExecute.disabled = true;
    }
}

// Execute command
async function executeCommand() {
    const command = commandInput.value.trim();
    if (!command) {
        return;
    }

    // Disable input during execution
    commandInput.disabled = true;
    btnExecute.disabled = true;
    btnExecute.textContent = 'Executing...';

    // Add user message
    addMessage(command, 'user');

    // Clear input
    commandInput.value = '';

    try {
        // Get active tab
        const tabs = await chrome.tabs.query({ active: true, currentWindow: true });
        if (!tabs[0]) {
            addMessage('No active tab found', 'error');
            return;
        }

        // Send command to background
        const response = await chrome.runtime.sendMessage({
            type: 'execute_oil',
            oil: command,
            tabId: tabs[0].id,
        });

        // Display result
        if (response.error) {
            addMessage(`Error: ${response.error}`, 'error');
        } else if (response.success) {
            // Format result based on type
            const formatted = formatResult(response);
            addMessage(formatted, 'success');
        } else {
            addMessage('Unexpected response from background script', 'error');
        }

        // Update scan status
        await updateStatus();
    } catch (error) {
        console.error('Command execution error:', error);
        addMessage(`Error: ${error.message}`, 'error');
    } finally {
        // Re-enable input
        commandInput.disabled = false;
        btnExecute.disabled = false;
        btnExecute.textContent = 'Execute';
        commandInput.focus();
    }
}

// Auto-resize textarea
commandInput.addEventListener('input', () => {
    commandInput.style.height = 'auto';
    commandInput.style.height = Math.min(commandInput.scrollHeight, 120) + 'px';
});

// Handle Enter key (Shift+Enter for newline, Enter to execute)
commandInput.addEventListener('keydown', (e) => {
    if (e.key === 'Enter' && !e.shiftKey) {
        e.preventDefault();
        executeCommand();
    }
});

// Event listeners
btnExecute.addEventListener('click', executeCommand);
btnClear.addEventListener('click', clearMessages);

// Listen for messages from background script
chrome.runtime.onMessage.addListener((request, sender, sendResponse) => {
    if (request.type === 'log') {
        addMessage(request.message, request.level || 'system');
    }
    sendResponse({ ok: true });
});

// Initialize
console.log('[Oryn-W] Sidepanel initialized');
updateStatus();

// Update status periodically
setInterval(updateStatus, 2000);

// Focus input on load
commandInput.focus();
