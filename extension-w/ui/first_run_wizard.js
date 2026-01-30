// First Run Wizard Logic

let currentStep = 1;
let hwProfile = null;
let selectedAdapter = null;

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

// Initialize wizard
async function init() {
    console.log('[Wizard] Initializing...');

    // Start hardware detection immediately
    await runHardwareDetection();

    // Setup navigation
    document.getElementById('btn-next').addEventListener('click', nextStep);
    document.getElementById('btn-back').addEventListener('click', prevStep);
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
                    <span style="font-size: 13px;">${hwProfile.chromeAI.status === 'ready' ? 'Ready to use' : (hwProfile.chromeAI.status === 'downloadable' || hwProfile.chromeAI.status === 'downloading') ? 'Available after download' : 'Not available'}</span>
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
        showStep(3);
        await completeSetup();
    } else if (currentStep === 3) {
        await finishWizard();
    }
}

function prevStep() {
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
    `;

    div.addEventListener('click', () => {
        document.querySelectorAll('.adapter-option').forEach(opt => {
            opt.classList.remove('selected');
        });
        div.classList.add('selected');
        selectedAdapter = adapter;
    });

    return div;
}

async function completeSetup() {
    const summaryDiv = document.getElementById('config-summary');

    const icon = ADAPTER_ICONS[selectedAdapter.id] || '🤖';

    summaryDiv.innerHTML = `
        <div style="display: flex; align-items: center; gap: 12px; font-size: 18px;">
            <span style="font-size: 32px;">${icon}</span>
            <div>
                <strong>${selectedAdapter.displayName}</strong><br>
                <span style="font-size: 14px; color: #666;">Selected as your default AI model</span>
            </div>
        </div>
    `;

    // Configure the adapter in background
    try {
        let defaultModel = null;
        if (selectedAdapter.id === 'chrome-ai') {
            defaultModel = 'gemini-nano';
        } else if (selectedAdapter.id === 'webllm') {
            defaultModel = 'Phi-3-mini-4k-instruct-q4f16_1';
        } else if (selectedAdapter.id === 'wllama') {
            defaultModel = 'tinyllama';
        }

        await chrome.runtime.sendMessage({
            type: 'llm_set_adapter',
            adapter: selectedAdapter.id,
            model: defaultModel,
            apiKey: null
        });

        console.log('[Wizard] Adapter configured successfully');
    } catch (error) {
        console.error('[Wizard] Failed to configure adapter:', error);
    }
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
