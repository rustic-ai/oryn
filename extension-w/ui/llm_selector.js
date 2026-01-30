// LLM Selector UI Controller
// Handles adapter selection, configuration, and testing

// Adapter metadata with icons and descriptions
const ADAPTER_META = {
    'chrome-ai': {
        icon: '⚡',
        name: 'Chrome AI (Gemini Nano)',
        description: 'Built-in browser AI. Fast, private, and free. Best for quick responses.',
        type: 'local',
        requiresApi: false,
        color: '#4285f4',
        specs: [
            { label: 'Speed', value: '300-1000ms', highlight: true },
            { label: 'Privacy', value: 'Private', highlight: true },
            { label: 'Cost', value: 'Free', highlight: true },
            { label: 'Quality', value: 'Basic' }
        ]
    },
    'webllm': {
        icon: '🚀',
        name: 'WebLLM',
        description: 'GPU-accelerated local LLMs. High quality responses with near-native performance.',
        type: 'local',
        requiresApi: false,
        color: '#764ba2',
        specs: [
            { label: 'Speed', value: '500-2000ms', highlight: true },
            { label: 'Privacy', value: 'Private', highlight: true },
            { label: 'Cost', value: 'Free', highlight: true },
            { label: 'Quality', value: 'High', highlight: true },
            { label: 'WebGPU', value: 'Required' }
        ],
        models: [
            { id: 'Llama-3-8B-Instruct-q4f16_1', name: 'Llama 3 8B (4.5GB)', size: '4.5GB', quality: 'Best' },
            { id: 'Phi-3-mini-4k-instruct-q4f16_1', name: 'Phi-3 Mini (2.2GB)', size: '2.2GB', quality: 'Good' },
            { id: 'Gemma-2B-it-q4f16_1', name: 'Gemma 2B (1.5GB)', size: '1.5GB', quality: 'Fast' }
        ]
    },
    'wllama': {
        icon: '🦙',
        name: 'llama.cpp (wllama)',
        description: 'CPU-based WASM inference. Works everywhere without GPU, good for smaller models.',
        type: 'local',
        requiresApi: false,
        color: '#10a37f',
        specs: [
            { label: 'Speed', value: '2-10s' },
            { label: 'Privacy', value: 'Private', highlight: true },
            { label: 'Cost', value: 'Free', highlight: true },
            { label: 'Quality', value: 'Medium' },
            { label: 'GPU', value: 'Not Required', highlight: true }
        ],
        models: [
            { id: 'tinyllama', name: 'TinyLlama 1.1B (669MB)', size: '669MB', quality: 'Fast' },
            { id: 'phi2', name: 'Phi-2 (1.6GB)', size: '1.6GB', quality: 'Good' }
        ]
    },
    'onnx': {
        icon: '⚙️',
        name: 'ONNX Runtime Web',
        description: 'Flexible runtime for custom ONNX models. Supports WebGPU and quantized models.',
        type: 'local',
        requiresApi: false,
        color: '#5e5ce6',
        specs: [
            { label: 'Speed', value: '1-5s' },
            { label: 'Privacy', value: 'Private', highlight: true },
            { label: 'Cost', value: 'Free', highlight: true },
            { label: 'Quality', value: 'Varies' },
            { label: 'Custom', value: 'Models', highlight: true }
        ]
    },
    'openai': {
        icon: '🤖',
        name: 'OpenAI',
        description: 'GPT-4 and GPT-3.5 models. Most capable but requires API key and costs money.',
        type: 'remote',
        requiresApi: true,
        color: '#10a37f',
        specs: [
            { label: 'Speed', value: '500-2000ms', highlight: true },
            { label: 'Privacy', value: 'Shared' },
            { label: 'Cost', value: 'Paid' },
            { label: 'Quality', value: 'Excellent', highlight: true }
        ],
        models: [
            { id: 'gpt-4', name: 'GPT-4', quality: 'Best', cost: '$$$$' },
            { id: 'gpt-4-turbo', name: 'GPT-4 Turbo', quality: 'Best', cost: '$$$' },
            { id: 'gpt-3.5-turbo', name: 'GPT-3.5 Turbo', quality: 'Good', cost: '$' }
        ],
        apiKeyLabel: 'OpenAI API Key',
        apiKeyPlaceholder: 'sk-...',
        apiKeyHint: 'Get your API key from https://platform.openai.com/api-keys'
    },
    'claude': {
        icon: '🧠',
        name: 'Anthropic Claude',
        description: 'Claude 3.5 Sonnet and Haiku. Excellent quality with strong reasoning capabilities.',
        type: 'remote',
        requiresApi: true,
        color: '#d97757',
        specs: [
            { label: 'Speed', value: '500-2000ms', highlight: true },
            { label: 'Privacy', value: 'Shared' },
            { label: 'Cost', value: 'Paid' },
            { label: 'Quality', value: 'Excellent', highlight: true }
        ],
        models: [
            { id: 'claude-3-5-sonnet-20241022', name: 'Claude 3.5 Sonnet', quality: 'Best', cost: '$$$' },
            { id: 'claude-3-haiku-20240307', name: 'Claude 3 Haiku', quality: 'Good', cost: '$' }
        ],
        apiKeyLabel: 'Anthropic API Key',
        apiKeyPlaceholder: 'sk-ant-...',
        apiKeyHint: 'Get your API key from https://console.anthropic.com/'
    },
    'gemini': {
        icon: '✨',
        name: 'Google Gemini',
        description: 'Gemini Pro and Flash. Google\'s multimodal AI with competitive pricing.',
        type: 'remote',
        requiresApi: true,
        color: '#4285f4',
        specs: [
            { label: 'Speed', value: '400-1500ms', highlight: true },
            { label: 'Privacy', value: 'Shared' },
            { label: 'Cost', value: 'Paid' },
            { label: 'Quality', value: 'Excellent', highlight: true }
        ],
        models: [
            { id: 'gemini-pro', name: 'Gemini Pro', quality: 'Best', cost: '$$' },
            { id: 'gemini-flash', name: 'Gemini Flash', quality: 'Good', cost: '$' }
        ],
        apiKeyLabel: 'Google AI API Key',
        apiKeyPlaceholder: 'AI...',
        apiKeyHint: 'Get your API key from https://makersuite.google.com/app/apikey'
    }
};

let currentConfig = {
    selectedAdapter: null,
    selectedModel: null,
    apiKeys: {},
    agentSettings: {
        maxIterations: 10,
        temperature: 0.7,
        retrievalCount: 3
    }
};

let availableAdapters = [];

// Initialize UI
document.addEventListener('DOMContentLoaded', async () => {
    await loadConfiguration();
    await detectAdapters();
    await renderAdapters();
    setupEventListeners();
});

// Load saved configuration
async function loadConfiguration() {
    try {
        const result = await chrome.storage.sync.get(['llmConfig']);
        if (result.llmConfig) {
            currentConfig = { ...currentConfig, ...result.llmConfig };

            // Update sliders
            document.getElementById('maxIterations').value = currentConfig.agentSettings.maxIterations;
            document.getElementById('temperature').value = currentConfig.agentSettings.temperature * 100;
            document.getElementById('retrievalCount').value = currentConfig.agentSettings.retrievalCount;
            updateSliderValues();
        }
    } catch (error) {
        console.error('Failed to load configuration:', error);
    }
}

// Detect available adapters from background script
async function detectAdapters() {
    try {
        console.log('[LLM Selector] Requesting adapters from background...');
        const response = await chrome.runtime.sendMessage({ type: 'llm_get_adapters' });
        console.log('[LLM Selector] Response from background:', response);
        console.log('[LLM Selector] response.adapters:', response.adapters);
        console.log('[LLM Selector] typeof response:', typeof response);
        console.log('[LLM Selector] Object.keys(response):', Object.keys(response));

        availableAdapters = response.adapters || [];
        console.log('[LLM Selector] Available adapters:', availableAdapters);

        if (availableAdapters.length === 0) {
            console.warn('[LLM Selector] No adapters detected! Check background console.');
            console.warn('[LLM Selector] Full response object:', JSON.stringify(response));
        }
    } catch (error) {
        console.error('[LLM Selector] Failed to detect adapters:', error);
        availableAdapters = [];
    }
}

// Render adapter cards
async function renderAdapters() {
    const gridLocal = document.getElementById('adapterGridLocal');
    const gridRemote = document.getElementById('adapterGridRemote');
    const hwSummary = document.getElementById('hardwareSummary');

    gridLocal.innerHTML = '';
    gridRemote.innerHTML = '';

    // Render hardware summary
    await renderHardwareSummary(hwSummary);

    // Separate adapters by category
    const localCards = [];
    const remoteCards = [];

    // Create all adapter cards (async)
    for (const adapter of availableAdapters) {
        const adapterId = adapter.id;
        const meta = ADAPTER_META[adapterId];

        if (!meta) {
            console.warn('[LLM Selector] No metadata for adapter:', adapterId);
            continue;
        }

        const isSelected = currentConfig.selectedAdapter === adapterId;
        const card = await createAdapterCard(adapterId, meta, true, isSelected);

        // Categorize by type
        if (meta.type === 'local') {
            localCards.push(card);
        } else if (meta.type === 'remote') {
            remoteCards.push(card);
        }
    }

    // Append to appropriate grids
    localCards.forEach(card => gridLocal.appendChild(card));
    remoteCards.forEach(card => gridRemote.appendChild(card));

    // Show empty states if needed
    if (localCards.length === 0) {
        gridLocal.innerHTML = '<div class="empty-message">ℹ️ No local models available on this device. Check hardware requirements.</div>';
    }
    if (remoteCards.length === 0) {
        gridRemote.innerHTML = '<div class="empty-message">ℹ️ Remote APIs are always available with API keys.</div>';
    }
}

// Render hardware summary
async function renderHardwareSummary(container) {
    try {
        // Get cached hardware profile from background
        const response = await chrome.runtime.sendMessage({ type: 'get_hardware_profile' });
        const hw = response.profile;

        if (!hw) {
            container.style.display = 'none';
            return;
        }

        container.style.display = 'block';
        container.innerHTML = `
            <strong>Your Device:</strong>
            <div class="hw-item ${hw.chromeAI.available ? 'available' : 'unavailable'}">
                ${hw.chromeAI.available ? '✓' : '✗'} Chrome AI
            </div>
            <div class="hw-item ${hw.webgpu.available ? 'available' : 'unavailable'}">
                ${hw.webgpu.available ? '✓' : '✗'} WebGPU
            </div>
            <div class="hw-item">
                💾 ~${hw.ram.estimated || '?'}GB RAM
            </div>
            ${hw.recommendations && hw.recommendations.length > 0 ? `
                <div style="margin-top: 8px;">
                    <strong>💡 Recommendation:</strong> ${hw.recommendations[0]}
                </div>
            ` : ''}
        `;
    } catch (error) {
        console.error('[LLM Selector] Failed to render hardware summary:', error);
        container.style.display = 'none';
    }
}

// Get compatibility warning for adapter
async function getCompatibilityWarning(adapterId) {
    try {
        const response = await chrome.runtime.sendMessage({
            type: 'check_adapter_compatibility',
            adapter: adapterId
        });
        return response.warning || null;
    } catch (error) {
        console.error('[LLM Selector] Compatibility check failed:', error);
        return null;
    }
}

// Create adapter card element
async function createAdapterCard(adapterId, meta, isAvailable, isSelected) {
    const card = document.createElement('div');
    card.className = `adapter-card ${isSelected ? 'selected' : ''} ${!isAvailable ? 'unavailable' : ''} ${meta.requiresApi ? 'requires-api' : ''}`;
    card.dataset.adapterId = adapterId;

    // Adapter header
    const header = document.createElement('div');
    header.className = 'adapter-header';

    const nameDiv = document.createElement('div');
    nameDiv.className = 'adapter-name';
    nameDiv.innerHTML = `
        <div class="adapter-icon" style="background: ${meta.color}20; color: ${meta.color}">
            ${meta.icon}
        </div>
        <span>${meta.name}</span>
    `;

    const statusDiv = document.createElement('div');
    statusDiv.className = `adapter-status ${isAvailable ? 'status-ready' : 'status-unavailable'}`;
    statusDiv.innerHTML = `
        <span>${isAvailable ? '✓' : '✗'}</span>
        <span>${isAvailable ? 'Ready' : 'Unavailable'}</span>
    `;

    header.appendChild(nameDiv);
    header.appendChild(statusDiv);

    // Description
    const description = document.createElement('div');
    description.className = 'adapter-description';
    description.textContent = meta.description;

    // Specs
    const specsDiv = document.createElement('div');
    specsDiv.className = 'adapter-specs';
    meta.specs.forEach(spec => {
        const badge = document.createElement('span');
        badge.className = `spec-badge ${spec.highlight ? 'highlight' : ''}`;
        badge.innerHTML = `<strong>${spec.label}:</strong> ${spec.value}`;
        specsDiv.appendChild(badge);
    });

    // Model selector (for adapters with multiple models)
    const modelSelector = createModelSelector(adapterId, meta);

    // API key input (for remote adapters)
    const apiKeySection = createApiKeySection(adapterId, meta);

    // Assemble card
    card.appendChild(header);
    card.appendChild(description);
    card.appendChild(specsDiv);

    // Add hardware compatibility warning
    const warning = await getCompatibilityWarning(adapterId);
    if (warning) {
        const warningDiv = document.createElement('div');
        warningDiv.className = 'adapter-warning';
        warningDiv.innerHTML = `⚠️ ${warning}`;
        card.appendChild(warningDiv);
    }

    if (modelSelector) card.appendChild(modelSelector);
    if (apiKeySection) card.appendChild(apiKeySection);

    // Click handler
    if (isAvailable) {
        card.addEventListener('click', (e) => {
            // Don't trigger if clicking on input fields
            if (e.target.tagName === 'INPUT' || e.target.tagName === 'SELECT') {
                return;
            }
            selectAdapter(adapterId);
        });
    }

    return card;
}

// Create model selector
function createModelSelector(adapterId, meta) {
    if (!meta.models || meta.models.length === 0) {
        return null;
    }

    const container = document.createElement('div');
    container.className = 'model-selector';

    const label = document.createElement('div');
    label.className = 'model-label';
    label.textContent = 'Select Model:';

    const select = document.createElement('select');
    select.className = 'model-select';
    select.id = `model-select-${adapterId}`;

    meta.models.forEach((model, index) => {
        const option = document.createElement('option');
        option.value = model.id;
        option.textContent = model.name;
        if (currentConfig.selectedModel === model.id) {
            option.selected = true;
        } else if (index === 0 && !currentConfig.selectedModel) {
            option.selected = true;
        }
        select.appendChild(option);
    });

    const info = document.createElement('div');
    info.className = 'model-info';
    info.id = `model-info-${adapterId}`;
    updateModelInfo(adapterId, select.value);

    select.addEventListener('change', (e) => {
        currentConfig.selectedModel = e.target.value;
        updateModelInfo(adapterId, e.target.value);
    });

    container.appendChild(label);
    container.appendChild(select);
    container.appendChild(info);

    return container;
}

// Update model info display
function updateModelInfo(adapterId, modelId) {
    const meta = ADAPTER_META[adapterId];
    const model = meta.models?.find(m => m.id === modelId);
    if (!model) return;

    const infoDiv = document.getElementById(`model-info-${adapterId}`);
    if (!infoDiv) return;

    let infoText = '';
    if (model.size) infoText += `Size: ${model.size} • `;
    if (model.quality) infoText += `Quality: ${model.quality} • `;
    if (model.cost) infoText += `Cost: ${model.cost}`;

    infoDiv.textContent = infoText.replace(/ • $/, '');
}

// Create API key input section
function createApiKeySection(adapterId, meta) {
    if (!meta.requiresApi) {
        return null;
    }

    const container = document.createElement('div');
    container.className = 'api-key-section';

    const inputGroup = document.createElement('div');
    inputGroup.className = 'input-group';

    const label = document.createElement('label');
    label.className = 'input-label';
    label.textContent = meta.apiKeyLabel || 'API Key';

    const input = document.createElement('input');
    input.type = 'password';
    input.className = 'input-field';
    input.id = `api-key-${adapterId}`;
    input.placeholder = meta.apiKeyPlaceholder || 'Enter API key';
    input.value = currentConfig.apiKeys[adapterId] || '';

    const hint = document.createElement('div');
    hint.className = 'input-hint';
    hint.textContent = meta.apiKeyHint || '';

    input.addEventListener('input', (e) => {
        currentConfig.apiKeys[adapterId] = e.target.value;
    });

    inputGroup.appendChild(label);
    inputGroup.appendChild(input);
    inputGroup.appendChild(hint);
    container.appendChild(inputGroup);

    return container;
}

// Select an adapter
function selectAdapter(adapterId) {
    currentConfig.selectedAdapter = adapterId;

    // Update UI
    document.querySelectorAll('.adapter-card').forEach(card => {
        card.classList.remove('selected');
    });

    const selectedCard = document.querySelector(`[data-adapter-id="${adapterId}"]`);
    if (selectedCard) {
        selectedCard.classList.add('selected');
    }

    // Update selected model if applicable
    const meta = ADAPTER_META[adapterId];
    if (meta.models && meta.models.length > 0) {
        const select = document.getElementById(`model-select-${adapterId}`);
        if (select) {
            currentConfig.selectedModel = select.value;
        }
    } else {
        currentConfig.selectedModel = null;
    }

    console.log('Selected adapter:', adapterId, 'Model:', currentConfig.selectedModel);
}

// Setup event listeners
function setupEventListeners() {
    // Sliders
    const maxIterSlider = document.getElementById('maxIterations');
    const tempSlider = document.getElementById('temperature');
    const retrievalSlider = document.getElementById('retrievalCount');

    maxIterSlider.addEventListener('input', updateSliderValues);
    tempSlider.addEventListener('input', updateSliderValues);
    retrievalSlider.addEventListener('input', updateSliderValues);

    // Buttons
    document.getElementById('saveBtn').addEventListener('click', saveConfiguration);
    document.getElementById('resetBtn').addEventListener('click', resetConfiguration);
    document.getElementById('testBtn').addEventListener('click', testLLM);
    document.getElementById('progressCancelBtn').addEventListener('click', closeProgressModal);
}

// Update slider value displays
function updateSliderValues() {
    const maxIter = parseInt(document.getElementById('maxIterations').value);
    const temp = parseInt(document.getElementById('temperature').value);
    const retrieval = parseInt(document.getElementById('retrievalCount').value);

    document.getElementById('maxIterationsValue').textContent = maxIter;
    document.getElementById('temperatureValue').textContent = (temp / 100).toFixed(1);
    document.getElementById('retrievalCountValue').textContent = retrieval;

    currentConfig.agentSettings.maxIterations = maxIter;
    currentConfig.agentSettings.temperature = temp / 100;
    currentConfig.agentSettings.retrievalCount = retrieval;
}

// Save configuration
async function saveConfiguration() {
    try {
        await chrome.storage.sync.set({ llmConfig: currentConfig });

        // Check if this is a local adapter that needs model download
        const needsDownload = ['webllm', 'wllama'].includes(currentConfig.selectedAdapter);

        if (needsDownload) {
            // Show progress modal for local adapters
            showProgressModal(currentConfig.selectedAdapter, currentConfig.selectedModel);
        }

        // Notify background script to update adapter
        if (currentConfig.selectedAdapter) {
            const response = await chrome.runtime.sendMessage({
                type: 'llm_set_adapter',
                adapter: currentConfig.selectedAdapter,
                model: currentConfig.selectedModel,
                apiKey: currentConfig.apiKeys[currentConfig.selectedAdapter],
                settings: currentConfig.agentSettings
            });

            if (needsDownload && response.success) {
                // Start polling for progress
                startProgressPolling();
            } else if (!needsDownload) {
                // For non-downloading adapters, show success immediately
                showStatusMessage('Configuration saved successfully!', 'success');
            }
        }
    } catch (error) {
        console.error('Failed to save configuration:', error);
        showStatusMessage('Failed to save configuration: ' + error.message, 'error');
        closeProgressModal();
    }
}

// Reset configuration
async function resetConfiguration() {
    if (!confirm('Reset all settings to defaults? This will clear API keys.')) {
        return;
    }

    currentConfig = {
        selectedAdapter: 'auto',
        selectedModel: null,
        apiKeys: {},
        agentSettings: {
            maxIterations: 10,
            temperature: 0.7,
            retrievalCount: 3
        }
    };

    await chrome.storage.sync.remove('llmConfig');
    location.reload();
}

// Test LLM
async function testLLM() {
    const prompt = document.getElementById('testPrompt').value.trim();
    if (!prompt) {
        showStatusMessage('Please enter a test prompt', 'error');
        return;
    }

    if (!currentConfig.selectedAdapter) {
        showStatusMessage('Please select an adapter first', 'error');
        return;
    }

    const testBtn = document.getElementById('testBtn');
    const testBtnText = document.getElementById('testBtnText');
    const testSpinner = document.getElementById('testSpinner');
    const testOutput = document.getElementById('testOutput');

    // Show loading state
    testBtn.disabled = true;
    testBtnText.textContent = 'Testing...';
    testSpinner.style.display = 'inline-block';
    testOutput.textContent = '';
    testOutput.classList.remove('visible');

    try {
        const startTime = performance.now();

        const response = await chrome.runtime.sendMessage({
            type: 'llm_prompt',
            messages: [
                { role: 'user', content: prompt }
            ],
            options: {
                temperature: currentConfig.agentSettings.temperature
            }
        });

        const duration = Math.round(performance.now() - startTime);

        if (response.success) {
            testOutput.textContent = `✓ Success (${duration}ms)\n\n${response.response}`;
            testOutput.classList.add('visible');
            showStatusMessage('Test completed successfully!', 'success');
        } else {
            throw new Error(response.error || 'Unknown error');
        }
    } catch (error) {
        console.error('Test failed:', error);
        testOutput.textContent = `✗ Error: ${error.message}`;
        testOutput.classList.add('visible');
        showStatusMessage('Test failed: ' + error.message, 'error');
    } finally {
        // Reset button state
        testBtn.disabled = false;
        testBtnText.textContent = 'Test Prompt';
        testSpinner.style.display = 'none';
    }
}

// Show status message
function showStatusMessage(message, type) {
    const statusMsg = document.getElementById('statusMessage');
    statusMsg.textContent = message;
    statusMsg.className = `status-message ${type} visible`;

    setTimeout(() => {
        statusMsg.classList.remove('visible');
    }, 5000);
}

// Progress Modal Functions
let progressPollInterval = null;

function showProgressModal(adapterId, modelId) {
    const modal = document.getElementById('progressModal');
    const title = document.getElementById('progressTitle');
    const subtitle = document.getElementById('progressSubtitle');
    const status = document.getElementById('progressStatus');

    const meta = ADAPTER_META[adapterId];
    const model = meta?.models?.find(m => m.id === modelId);

    title.textContent = `Downloading ${meta?.name || adapterId} Model`;
    subtitle.textContent = `Model: ${model?.name || modelId}${model?.size ? ' (' + model.size + ')' : ''}`;
    status.textContent = 'Initializing download...';

    document.getElementById('progressPercentage').textContent = '0%';
    document.getElementById('progressSize').textContent = '';
    document.getElementById('progressBarFill').style.width = '0%';

    modal.classList.add('visible');
}

function closeProgressModal() {
    const modal = document.getElementById('progressModal');
    modal.classList.remove('visible');

    if (progressPollInterval) {
        clearInterval(progressPollInterval);
        progressPollInterval = null;
    }
}

async function startProgressPolling() {
    let lastProgress = 0;
    let stuckCount = 0;

    progressPollInterval = setInterval(async () => {
        try {
            const response = await chrome.runtime.sendMessage({ type: 'llm_status' });

            // Check if adapter is ready (download complete)
            if (response.ready && response.adapter) {
                clearInterval(progressPollInterval);
                progressPollInterval = null;

                updateProgressUI(100, 'Download complete!', true);

                setTimeout(() => {
                    closeProgressModal();
                    showStatusMessage('Configuration saved and model loaded successfully!', 'success');
                }, 1500);

                return;
            }

            // Check for errors
            if (response.error) {
                clearInterval(progressPollInterval);
                progressPollInterval = null;

                updateProgressUI(lastProgress, `Error: ${response.error}`, true);
                showStatusMessage('Failed to load model: ' + response.error, 'error');

                return;
            }

            // Update progress if available
            if (response.downloadProgress !== undefined) {
                const progress = Math.round(response.downloadProgress);

                // Check if progress is stuck
                if (progress === lastProgress && progress < 100) {
                    stuckCount++;
                    if (stuckCount > 10) { // Stuck for ~20 seconds
                        updateProgressUI(progress, 'Download may be slow or stalled. Please wait...', false);
                    }
                } else {
                    stuckCount = 0;
                    lastProgress = progress;
                }

                let statusText = 'Downloading model...';
                if (progress > 0 && progress < 100) {
                    statusText = `Downloading model... ${progress}% complete`;
                } else if (progress === 100) {
                    statusText = 'Initializing model...';
                }

                updateProgressUI(progress, statusText, false);
            } else if (response.isLoading) {
                updateProgressUI(lastProgress, 'Loading model...', false);
            }

        } catch (error) {
            console.error('Progress poll failed:', error);
            // Don't stop polling on error, might be transient
        }
    }, 2000); // Poll every 2 seconds
}

function updateProgressUI(percentage, statusText, isComplete) {
    const progressBar = document.getElementById('progressBarFill');
    const progressPercentage = document.getElementById('progressPercentage');
    const progressStatus = document.getElementById('progressStatus');

    progressBar.style.width = `${percentage}%`;
    progressPercentage.textContent = `${percentage}%`;
    progressStatus.textContent = statusText;

    if (isComplete) {
        progressStatus.style.borderLeftColor = '#4ade80';
    }
}
