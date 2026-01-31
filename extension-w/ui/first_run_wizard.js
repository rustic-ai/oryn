// First Run Wizard Logic

let currentStep = 1;
let hwProfile = null;
let selectedAdapter = null;
let selectedModel = null;
let isDownloading = false;
let downloadPollInterval = null;

const ADAPTER_ICONS = {
    'chrome-ai': '⚡',
    'webllm': '🚀',
    'wllama': '🦙'
};

const ADAPTER_DESCRIPTIONS = {
    'chrome-ai': 'Built into Chrome. Instant and private.',
    'webllm': 'High quality local AI. Requires download (1.5-4.5GB).',
    'wllama': 'Universal CPU-based AI. Works everywhere.'
};

const ADAPTER_BADGES = {
    'chrome-ai': ['free', 'fast', 'local'],
    'webllm': ['free', 'local'],
    'wllama': ['free', 'local']
};

const ADAPTER_MODELS = {
    'webllm': [
        { id: 'Phi-3-mini-4k-instruct-q4f16_1-MLC-1k', size: '2.2GB', description: 'Balanced (recommended)', default: true },
        { id: 'Llama-3-8B-Instruct-q4f16_1-MLC-1k', size: '4.5GB', description: 'Best quality' },
        { id: 'gemma-2b-it-q4f16_1-MLC-1k', size: '1.5GB', description: 'Smallest, fastest' }
    ],
    'wllama': [
        { id: 'tinyllama', size: '669MB', description: 'Lightweight (recommended)', default: true },
        { id: 'phi2', size: '1.6GB', description: 'Phi-2 2.7B' },
        { id: 'gemma-2b', size: '1.6GB', description: 'Gemma 2B' }
    ]
};

// Initialize wizard
async function init() {
    console.log('[Wizard] Initializing...');

    // Start hardware detection immediately
    await runHardwareDetection();

    // Setup navigation
    document.getElementById('btn-next').addEventListener('click', nextStep);
    document.getElementById('btn-back').addEventListener('click', prevStep);
}

function chromeAIStatusText(status) {
    if (status === 'ready') return 'Ready to use';
    if (status === 'downloadable' || status === 'downloading') return 'Available after download';
    return 'Not available';
}

async function runHardwareDetection() {
    const resultsDiv = document.getElementById('hw-results');
    const recDiv = document.getElementById('recommendations');

    try {
        // Call hardware detector via background
        const response = await chrome.runtime.sendMessage({ type: 'detect_hardware' });
        hwProfile = response.profile;

        console.log('[Wizard] Hardware profile:', hwProfile);

        // Display results
        resultsDiv.innerHTML = `
            <div class="hw-item ${hwProfile.chromeAI.available ? 'available' : 'unavailable'}">
                <span style="font-size: 24px;">${hwProfile.chromeAI.available ? '✓' : '✗'}</span>
                <div>
                    <strong>Chrome AI</strong><br>
                    <span style="font-size: 13px;">${chromeAIStatusText(hwProfile.chromeAI.status)}</span>
                </div>
            </div>
            <div class="hw-item ${hwProfile.webgpu.available ? 'available' : 'unavailable'}">
                <span style="font-size: 24px;">${hwProfile.webgpu.available ? '✓' : '✗'}</span>
                <div>
                    <strong>WebGPU</strong><br>
                    <span style="font-size: 13px;">${hwProfile.webgpu.available ? 'GPU acceleration available' : 'Not available'}</span>
                </div>
            </div>
            <div class="hw-item">
                <span style="font-size: 24px;">💾</span>
                <div>
                    <strong>System Memory</strong><br>
                    <span style="font-size: 13px;">~${hwProfile.ram.estimated || '?'}GB RAM ${hwProfile.ram.sufficient ? '(Good)' : '(Limited)'}</span>
                </div>
            </div>
        `;

        // Show recommendations
        if (hwProfile.recommendations && hwProfile.recommendations.length > 0) {
            recDiv.style.display = 'block';
            recDiv.innerHTML = `
                <h3>💡 Recommendations</h3>
                <ul>
                    ${hwProfile.recommendations.map(r => `<li>${r}</li>`).join('')}
                </ul>
            `;
        }
    } catch (error) {
        console.error('[Wizard] Hardware detection failed:', error);
        resultsDiv.innerHTML = `
            <div class="hw-item unavailable">
                <span style="font-size: 24px;">✗</span>
                <div>
                    <strong>Detection Failed</strong><br>
                    <span style="font-size: 13px;">Error: ${error.message}</span>
                </div>
            </div>
        `;
    }
}

async function nextStep() {
    if (currentStep === 1) {
        showStep(2);
        await populateAdapterOptions();
    } else if (currentStep === 2) {
        if (!selectedAdapter) {
            alert('Please select an AI model');
            return;
        }

        // Save config and move to Step 3
        await saveConfigAndProceed();
    } else if (currentStep === 3) {
        await finishWizard();
    }
}

function prevStep() {
    if (isDownloading) {
        return; // Prevent going back during download
    }
    if (currentStep > 1) {
        showStep(currentStep - 1);
    }
}

function showStep(step) {
    // Hide all pages
    document.querySelectorAll('.wizard-page').forEach(page => {
        page.classList.remove('active');
    });

    // Show target page
    document.getElementById(`step-${step}`).classList.add('active');

    // Update step indicators
    document.querySelectorAll('.wizard-step').forEach(s => {
        s.classList.remove('active');
        const stepNum = parseInt(s.dataset.step);
        if (stepNum < step) {
            s.classList.add('completed');
        }
    });
    document.querySelector(`[data-step="${step}"]`).classList.add('active');

    currentStep = step;

    // Update button visibility
    const backBtn = document.getElementById('btn-back');
    const nextBtn = document.getElementById('btn-next');

    backBtn.style.display = step > 1 ? 'inline-block' : 'none';
    nextBtn.textContent = step === 3 ? 'Open Oryn' : 'Next';
}

async function populateAdapterOptions() {
    const container = document.getElementById('adapter-options');

    try {
        // Get available adapters
        const response = await chrome.runtime.sendMessage({ type: 'llm_get_adapters' });
        const adapters = response.adapters || [];

        if (adapters.length === 0) {
            container.innerHTML = '<p>No adapters available. Please check your setup.</p>';
            return;
        }

        // Filter and sort by recommendation (local adapters only)
        const recommended = adapters.filter(a =>
            a.id === 'chrome-ai' || a.id === 'webllm' || a.id === 'wllama'
        );

        container.innerHTML = '';

        recommended.forEach(adapter => {
            const option = createAdapterOption(adapter);
            container.appendChild(option);
        });

    } catch (error) {
        console.error('[Wizard] Failed to load adapters:', error);
        container.innerHTML = '<p>Error loading adapters.</p>';
    }
}

function createAdapterOption(adapter) {
    const div = document.createElement('div');
    div.className = 'adapter-option';
    div.dataset.adapterId = adapter.id;

    const icon = ADAPTER_ICONS[adapter.id] || '🤖';
    const description = ADAPTER_DESCRIPTIONS[adapter.id] || adapter.description;
    const badges = ADAPTER_BADGES[adapter.id] || [];
    const hasModels = ADAPTER_MODELS[adapter.id] && ADAPTER_MODELS[adapter.id].length > 0;

    let modelSelectorHtml = '';
    if (hasModels) {
        const models = ADAPTER_MODELS[adapter.id];

        modelSelectorHtml = `
            <div class="adapter-option-body" style="display: none;">
                <div class="model-selector">
                    <label>Select Model:</label>
                    <select class="model-dropdown">
                        ${models.map(model => `
                            <option value="${model.id}" ${model.default ? 'selected' : ''}>
                                ${model.id} (${model.size}) - ${model.description}
                            </option>
                        `).join('')}
                    </select>
                </div>
            </div>
        `;
    }

    div.innerHTML = `
        <div class="adapter-option-header">
            <div class="adapter-option-icon">${icon}</div>
            <div class="adapter-option-info">
                <h4>${adapter.displayName}</h4>
                <p>${description}</p>
            </div>
        </div>
        <div class="adapter-option-badges">
            ${badges.map(b =>
                `<span class="badge ${b}">${b.toUpperCase()}</span>`
            ).join('')}
        </div>
        ${modelSelectorHtml}
    `;

    div.addEventListener('click', (e) => {
        // Don't collapse if clicking inside the model dropdown
        if (e.target.classList.contains('model-dropdown')) {
            return;
        }

        document.querySelectorAll('.adapter-option').forEach(opt => {
            opt.classList.remove('selected');
            const body = opt.querySelector('.adapter-option-body');
            if (body) body.style.display = 'none';
        });

        div.classList.add('selected');
        selectedAdapter = adapter;

        // Show model selector if available
        const body = div.querySelector('.adapter-option-body');
        if (body) {
            body.style.display = 'block';
            const dropdown = div.querySelector('.model-dropdown');
            if (dropdown) {
                selectedModel = dropdown.value;
            }
        } else {
            // For adapters without model selection (like chrome-ai)
            selectedModel = null;
        }
    });

    // Handle model selection change
    const dropdown = div.querySelector('.model-dropdown');
    if (dropdown) {
        dropdown.addEventListener('change', (e) => {
            selectedModel = e.target.value;
            console.log('[Wizard] Model selected:', selectedModel);
        });

        // Set initial model
        if (hasModels) {
            const defaultModel = ADAPTER_MODELS[adapter.id].find(m => m.default) || ADAPTER_MODELS[adapter.id][0];
            selectedModel = defaultModel.id;
        }
    }

    return div;
}

// Save config to storage and move to Step 3
async function saveConfigAndProceed() {
    const nextBtn = document.getElementById('btn-next');
    nextBtn.disabled = true;
    nextBtn.textContent = 'Saving...';

    try {
        console.log('[Wizard] Saving config for adapter:', selectedAdapter.id, 'model:', selectedModel);

        // Save config to storage
        await chrome.storage.sync.set({
            llmConfig: {
                selectedAdapter: selectedAdapter.id,
                selectedModel: selectedModel,
                apiKeys: {}
            }
        });

        console.log('[Wizard] Config saved successfully');

        // Show Step 3 with config summary
        await showSetupStep();
        showStep(3);

        // Now trigger download/setup for the adapter
        await triggerAdapterSetup();

    } catch (error) {
        console.error('[Wizard] Failed to save config:', error);
        nextBtn.disabled = false;
        nextBtn.textContent = 'Next';
        alert('Failed to save configuration: ' + error.message);
    }
}

// Display the config summary and set up Step 3 UI
async function showSetupStep() {
    const summaryDiv = document.getElementById('config-summary');
    const icon = ADAPTER_ICONS[selectedAdapter.id] || '🤖';
    const needsDownload = selectedAdapter.id === 'webllm' || selectedAdapter.id === 'wllama';

    // Build model info text
    let modelText = 'Selected as your default AI model';
    if (selectedModel) {
        const modelInfo = ADAPTER_MODELS[selectedAdapter.id]?.find(m => m.id === selectedModel);
        if (modelInfo) {
            modelText = `${selectedModel} (${modelInfo.size})`;
        } else {
            modelText = selectedModel;
        }
    }

    summaryDiv.innerHTML = `
        <div style="display: flex; align-items: center; gap: 12px; font-size: 18px;">
            <span style="font-size: 32px;">${icon}</span>
            <div>
                <strong>${selectedAdapter.displayName}</strong><br>
                <span style="font-size: 14px; color: #666;">${modelText}</span>
            </div>
        </div>
    `;

    // Show/hide download section based on adapter type
    const downloadSection = document.getElementById('download-section');
    const readySection = document.getElementById('ready-section');
    const downloadError = document.getElementById('download-error');

    downloadError.style.display = 'none';

    if (needsDownload) {
        // Show download progress, hide ready section
        downloadSection.style.display = 'block';
        readySection.style.display = 'none';

        const modelInfo = ADAPTER_MODELS[selectedAdapter.id]?.find(m => m.id === selectedModel);
        document.getElementById('download-model-name').textContent =
            `Downloading ${selectedModel}${modelInfo ? ` (${modelInfo.size})` : ''}...`;
        document.getElementById('download-percentage').textContent = '0%';
        document.getElementById('wizard-progress-fill').style.width = '0%';
        document.getElementById('download-status-text').textContent = 'Preparing download...';
    } else {
        // No download needed - show ready section immediately
        downloadSection.style.display = 'none';
        readySection.style.display = 'block';
    }
}

// Trigger adapter setup (offscreen creation + initialization)
async function triggerAdapterSetup() {
    const needsDownload = selectedAdapter.id === 'webllm' || selectedAdapter.id === 'wllama';
    const nextBtn = document.getElementById('btn-next');
    const backBtn = document.getElementById('btn-back');

    if (!needsDownload) {
        // Chrome AI - just configure it directly
        try {
            await chrome.runtime.sendMessage({
                type: 'llm_set_adapter',
                adapter: selectedAdapter.id,
                model: 'gemini-nano',
                apiKey: null
            });
            console.log('[Wizard] Chrome AI configured');
        } catch (error) {
            console.error('[Wizard] Failed to configure Chrome AI:', error);
        }

        // Enable launch button
        nextBtn.disabled = false;
        nextBtn.textContent = 'Open Oryn';
        return;
    }

    // WebLLM/wllama - needs download via offscreen
    isDownloading = true;
    nextBtn.disabled = true;
    nextBtn.textContent = 'Downloading...';
    backBtn.disabled = true;

    try {
        // Tell background to load the config and create offscreen document
        console.log('[Wizard] Notifying background to load config and start download...');
        const loadResponse = await chrome.runtime.sendMessage({
            type: 'llm_reload_config'
        });
        console.log('[Wizard] Background config load response:', loadResponse);

        if (loadResponse?.error) {
            showDownloadError('Failed to start download: ' + loadResponse.error);
            return;
        }

        // Start polling for progress
        startProgressPolling();

    } catch (error) {
        console.error('[Wizard] Failed to trigger download:', error);
        showDownloadError('Failed to start download: ' + error.message);
    }
}

// Poll background for download progress
function startProgressPolling() {
    let noProgressCount = 0;
    let lastProgress = 0;
    let pollCount = 0;

    downloadPollInterval = setInterval(async () => {
        pollCount++;

        try {
            const response = await chrome.runtime.sendMessage({ type: 'llm_status' });

            console.log('[Wizard] Status poll #' + pollCount + ':', JSON.stringify(response));

            if (response.error && !response.isLoading) {
                // Real error - download failed
                clearInterval(downloadPollInterval);
                showDownloadError(response.error);
                return;
            }

            // Check for completion
            if (response.ready && !response.isLoading) {
                clearInterval(downloadPollInterval);
                downloadComplete();
                return;
            }

            // Update progress
            const progress = response.downloadProgress || 0;
            const statusText = getProgressMessage(progress, response.isLoading, response.isPending);
            updateDownloadUI(progress, statusText);

            // Detect stuck download
            if (progress === lastProgress && progress > 0) {
                noProgressCount++;
                if (noProgressCount > 15) { // 30 seconds no progress
                    updateDownloadUI(progress, 'Download seems slow. Please be patient...');
                }
            } else {
                noProgressCount = 0;
                lastProgress = progress;
            }

        } catch (error) {
            console.error('[Wizard] Progress poll error:', error);
            clearInterval(downloadPollInterval);
            showDownloadError('Lost connection to background: ' + error.message);
        }
    }, 2000); // Poll every 2 seconds
}

function updateDownloadUI(percentage, statusText) {
    const progressFill = document.getElementById('wizard-progress-fill');
    const progressPercent = document.getElementById('download-percentage');
    const statusTextEl = document.getElementById('download-status-text');

    if (progressFill) progressFill.style.width = `${percentage}%`;
    if (progressPercent) progressPercent.textContent = `${Math.round(percentage)}%`;
    if (statusText && statusTextEl) statusTextEl.textContent = statusText;
}

function getProgressMessage(progress, isLoading, isPending) {
    if (isPending) {
        return 'Setting up offscreen environment...';
    } else if (progress === 0 && isLoading) {
        return 'Initializing model engine...';
    } else if (progress > 0 && progress < 10) {
        return 'Starting model download...';
    } else if (progress >= 10 && progress < 50) {
        return 'Downloading model files...';
    } else if (progress >= 50 && progress < 90) {
        return 'Download in progress...';
    } else if (progress >= 90 && progress < 100) {
        return 'Almost done...';
    } else if (progress === 100) {
        return 'Download complete!';
    }
    return 'Preparing download...';
}

function downloadComplete() {
    console.log('[Wizard] Download completed!');
    isDownloading = false;

    // Update progress to 100%
    updateDownloadUI(100, 'Download complete!');

    // Show ready section
    document.getElementById('ready-section').style.display = 'block';

    // Enable launch button
    const nextBtn = document.getElementById('btn-next');
    const backBtn = document.getElementById('btn-back');
    nextBtn.disabled = false;
    nextBtn.textContent = 'Open Oryn';
    backBtn.disabled = false;
}

function showDownloadError(message) {
    console.error('[Wizard] Download error:', message);
    isDownloading = false;

    const downloadError = document.getElementById('download-error');
    downloadError.style.display = 'block';
    downloadError.innerHTML = `
        <div class="download-error">
            <div class="download-error-title">Download Failed</div>
            <div style="margin-bottom: 12px;">${message}</div>
            <button class="btn btn-primary" onclick="retryDownload()">Retry Download</button>
        </div>
    `;

    // Re-enable back button so user can change adapter
    document.getElementById('btn-back').disabled = false;
    // Keep "Open Oryn" disabled since download failed
    document.getElementById('btn-next').disabled = true;
    document.getElementById('btn-next').textContent = 'Open Oryn';
}

async function retryDownload() {
    // Clear error display
    document.getElementById('download-error').style.display = 'none';

    // Reset progress UI
    updateDownloadUI(0, 'Retrying download...');

    // Retry
    await triggerAdapterSetup();
}

async function finishWizard() {
    // Mark first run as complete
    await chrome.storage.local.set({ oryn_w_first_run_complete: true });

    // Close wizard tab and open sidepanel
    const [tab] = await chrome.tabs.query({ active: true, currentWindow: true });
    chrome.sidePanel.open({ windowId: tab.windowId });

    // Close wizard
    window.close();
}

// Start wizard on load
init();
