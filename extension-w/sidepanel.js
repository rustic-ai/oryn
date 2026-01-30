// Sidepanel for Oryn-W extension - Chat/Command Interface

const chatContainer = document.getElementById('chat-container');
const emptyState = document.getElementById('empty-state');
const commandInput = document.getElementById('command-input');
const btnExecute = document.getElementById('btn-execute');
const btnClear = document.getElementById('btn-clear');
const wasmBadge = document.getElementById('wasm-badge');
const scanBadge = document.getElementById('scan-badge');
const llmBadge = document.getElementById('llm-badge');

// Agent mode elements
const oilModeBtn = document.getElementById('oil-mode-btn');
const agentModeBtn = document.getElementById('agent-mode-btn');
const oilInputSection = document.getElementById('oil-input-section');
const agentInputSection = document.getElementById('agent-input-section');
const agentTask = document.getElementById('agent-task');
const executeAgent = document.getElementById('execute-agent');
const agentStatus = document.getElementById('agent-status');
const btnConfigLLM = document.getElementById('btn-config-llm');

// Current mode
let currentMode = 'oil'; // 'oil' or 'agent'

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
    emptyState.style.display = messages.length > 0 ? 'none' : 'flex';

    // Clear existing messages (except empty state)
    chatContainer.querySelectorAll('.message').forEach(msg => msg.remove());

    // Add all messages and scroll to bottom
    messages.forEach(msg => chatContainer.appendChild(createMessageElement(msg)));
    chatContainer.scrollTop = chatContainer.scrollHeight;
}

// Clear all messages
function clearMessages() {
    messages.length = 0;
    renderMessages();
}

// Format result for display - matches oryn-h CLI output
async function formatResult(response) {
    // If there's a message, use it
    if (response.message) {
        return response.message;
    }

    // If there's result data, format it
    if (response.result) {
        const result = response.result;

        // Scan result - format as canonical OIL text output using WASM formatter
        if (result.page && result.elements && result.stats) {
            return await formatScanResult(result);
        }

        // String result (from execute, url, title, etc.)
        if (typeof result === 'string') {
            return result;
        }

        // Object result (extraction values, etc.)
        if (typeof result === 'object') {
            return `Value: ${JSON.stringify(result)}`;
        }
    }

    return 'Command executed successfully';
}

// Format scan result using WASM formatter for uniform output
async function formatScanResult(scan) {
    const fallback = (reason) =>
        `@ ${scan.page.url} "${scan.page.title}"\n(Formatting unavailable: ${reason})`;

    try {
        const response = await chrome.runtime.sendMessage({
            type: 'format_scan',
            scan: scan
        });

        if (response.success) {
            return response.formatted;
        }

        console.error('[Sidepanel] Format scan failed:', response.error);
        return fallback(response.error);
    } catch (error) {
        console.error('[Sidepanel] Failed to format scan:', error);
        return fallback(error.message);
    }
}


// Toggle between OIL and Agent mode
function switchMode(mode) {
    currentMode = mode;

    if (mode === 'oil') {
        oilModeBtn.classList.add('active');
        agentModeBtn.classList.remove('active');
        oilInputSection.style.display = 'flex';
        agentInputSection.style.display = 'none';
    } else {
        oilModeBtn.classList.remove('active');
        agentModeBtn.classList.add('active');
        oilInputSection.style.display = 'none';
        agentInputSection.style.display = 'block';
    }
}

// Display agent iteration
function displayAgentIteration(iteration, thought, command, result) {
    const iterDiv = document.createElement('div');
    iterDiv.className = 'agent-iteration';

    const header = document.createElement('div');
    header.className = 'iteration-header';
    header.textContent = `Step ${iteration}`;

    const thoughtDiv = document.createElement('div');
    thoughtDiv.className = 'thought';
    thoughtDiv.textContent = `💭 ${thought}`;

    const commandDiv = document.createElement('div');
    commandDiv.className = 'command';
    commandDiv.textContent = `→ ${command}`;

    const resultDiv = document.createElement('div');
    resultDiv.className = result ? 'result-success' : 'result-failed';
    resultDiv.textContent = result ? '✓ Success' : '✗ Failed';

    iterDiv.appendChild(header);
    iterDiv.appendChild(thoughtDiv);
    if (command) {
        iterDiv.appendChild(commandDiv);
    }
    iterDiv.appendChild(resultDiv);

    chatContainer.appendChild(iterDiv);
    chatContainer.scrollTop = chatContainer.scrollHeight;
}

// Execute agent task
async function executeAgentTask() {
    const task = agentTask.value.trim();
    if (!task) {
        return;
    }

    // Check if page is valid first
    const pageCheck = await checkCurrentPage();
    if (!pageCheck.valid) {
        addMessage(`Cannot execute on this page: ${pageCheck.reason}`, 'error');
        addMessage(`Suggestion: ${pageCheck.suggestion}`, 'system');
        addMessage('Try navigating to google.com or any regular website', 'system');
        return;
    }

    // Check LLM status first
    const llmStatus = await chrome.runtime.sendMessage({ type: 'llm_status' });
    if (!llmStatus.ready) {
        addMessage('Please configure an LLM first. Click "Configure LLM" button.', 'error');
        return;
    }

    // Disable input during execution
    agentTask.disabled = true;
    executeAgent.disabled = true;
    executeAgent.textContent = 'Agent Running...';
    agentStatus.classList.add('active');
    agentStatus.textContent = 'Agent is thinking and executing...';

    // Clear previous output
    emptyState.style.display = 'none';
    const existingMessages = chatContainer.querySelectorAll('.message, .agent-iteration');
    existingMessages.forEach(msg => msg.remove());

    // Add task message
    const taskDiv = document.createElement('div');
    taskDiv.className = 'message message-user';
    taskDiv.textContent = `Task: ${task}`;
    chatContainer.appendChild(taskDiv);

    try {
        // Get active tab
        const tabs = await chrome.tabs.query({ active: true, currentWindow: true });
        if (!tabs[0]) {
            addMessage('No active tab found', 'error');
            return;
        }

        console.log('[Sidepanel] Starting agent execution:', task);

        // Send agent execution request
        const response = await chrome.runtime.sendMessage({
            type: 'execute_agent',
            task: task,
            tabId: tabs[0].id,
            config: {
                maxIterations: 10,
                temperature: 0.7,
                retrievalCount: 3,
            },
        });

        console.log('[Sidepanel] Agent response:', response);

        // Display results
        if (response.error) {
            addMessage(`Agent Error: ${response.error}`, 'error');
        } else if (response.history) {
            // Display each iteration
            for (const item of response.history) {
                displayAgentIteration(
                    item.iteration,
                    item.thought || 'Thinking...',
                    item.command,
                    item.result
                );
            }

            // Display agent's response if available
            if (response.response) {
                addMessage(response.response, 'success');
            }

            // Display completion status
            if (response.success) {
                addMessage(`✓ Completed in ${response.iterations} steps`, 'system');
            } else {
                addMessage(`⚠ Incomplete after ${response.iterations} steps: ${response.error || 'Max iterations reached'}`, 'error');
            }
        } else {
            addMessage('Unexpected response from agent', 'error');
        }
    } catch (error) {
        console.error('Agent execution error:', error);
        addMessage(`Error: ${error.message}`, 'error');
    } finally {
        // Re-enable input
        agentTask.disabled = false;
        executeAgent.disabled = false;
        executeAgent.textContent = 'Start Agent';
        agentStatus.classList.remove('active');
    }
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

        // Update LLM badge
        const llmStatus = await chrome.runtime.sendMessage({ type: 'llm_status' });
        if (llmStatus.ready) {
            llmBadge.textContent = `LLM: ${llmStatus.adapter}`;
            llmBadge.className = 'status-badge ready';
            executeAgent.disabled = false;
        } else {
            llmBadge.textContent = 'LLM: Not configured';
            llmBadge.className = 'status-badge idle';
            executeAgent.disabled = true;
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

    // Check if page is valid first
    const pageCheck = await checkCurrentPage();
    if (!pageCheck.valid) {
        addMessage(`Cannot execute on this page: ${pageCheck.reason}`, 'error');
        addMessage(`Suggestion: ${pageCheck.suggestion}`, 'system');
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
            // Provide helpful context for common errors
            if (response.error.includes('Content scripts cannot run')) {
                addMessage('⚠️ Invalid Page', 'error');
                addMessage('Navigate to a regular website (http:// or https://) to use automation', 'system');
            } else {
                addMessage(`Error: ${response.error}`, 'error');
            }
        } else if (response.success) {
            // Format result based on type (using WASM formatter for scans)
            const formatted = await formatResult(response);
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

// Mode toggle
oilModeBtn.addEventListener('click', () => switchMode('oil'));
agentModeBtn.addEventListener('click', () => switchMode('agent'));

// Agent execution
executeAgent.addEventListener('click', executeAgentTask);

// Configure LLM
btnConfigLLM.addEventListener('click', () => {
    // Open LLM selector in a new window
    const url = chrome.runtime.getURL('ui/llm_selector.html');
    window.open(url, 'LLM Configuration', 'width=600,height=700');
});

// Listen for messages from background script
chrome.runtime.onMessage.addListener((request, sender, sendResponse) => {
    if (request.type === 'log') {
        addMessage(request.message, request.level || 'system');
        sendResponse({ ok: true });  // Only respond to messages we handle
        return true;
    }
    // Don't respond to messages we don't handle - let other listeners handle them
});

// Check if current page is valid for automation
async function checkCurrentPage() {
    try {
        const tabs = await chrome.tabs.query({ active: true, currentWindow: true });
        if (!tabs[0]) {
            return { valid: false, reason: 'No active tab' };
        }

        const url = tabs[0].url;

        // Check for restricted pages
        const restrictedPrefixes = ['chrome:', 'chrome-extension:', 'edge:', 'about:', 'data:'];
        const matchedPrefix = restrictedPrefixes.find(prefix => url.startsWith(prefix));

        if (matchedPrefix) {
            return {
                valid: false,
                url,
                reason: `Cannot automate ${matchedPrefix} pages`,
                suggestion: 'Navigate to a regular website (e.g., google.com) to use automation'
            };
        }

        return { valid: true, url };
    } catch (error) {
        return { valid: false, reason: 'Could not check page' };
    }
}

// Show page warning if needed
async function showPageWarningIfNeeded() {
    const pageCheck = await checkCurrentPage();

    if (!pageCheck.valid) {
        const warningDiv = document.createElement('div');
        warningDiv.className = 'message message-error';
        warningDiv.style.marginBottom = '12px';
        warningDiv.innerHTML = `
            <strong>⚠️ Invalid Page Detected</strong><br>
            ${pageCheck.reason}<br>
            <br>
            <strong>Current:</strong> ${pageCheck.url || 'Unknown'}<br>
            <br>
            <strong>Suggestion:</strong> ${pageCheck.suggestion || 'Navigate to a regular website'}<br>
            <br>
            <em>Try: google.com, example.com, or any https:// website</em>
        `;

        chatContainer.insertBefore(warningDiv, emptyState);
        emptyState.style.display = 'none';
    }
}

// Initialize
console.log('[Oryn-W] Sidepanel initialized');
updateStatus();
showPageWarningIfNeeded();

// Update status periodically
setInterval(updateStatus, 2000);

// Check page when tab changes
chrome.tabs.onActivated.addListener(() => {
    showPageWarningIfNeeded();
});

chrome.tabs.onUpdated.addListener((tabId, changeInfo, tab) => {
    if (changeInfo.url) {
        showPageWarningIfNeeded();
    }
});

// Focus input on load
commandInput.focus();
