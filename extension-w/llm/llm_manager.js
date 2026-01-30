/**
 * LLM Manager
 *
 * Manages LLM adapters, handles auto-detection, adapter switching,
 * and provides a unified interface for LLM operations.
 */

import { ChromeAIAdapter } from './chrome_ai_adapter.js';
import { OpenAIAdapter } from './openai_adapter.js';
import { ClaudeAdapter } from './claude_adapter.js';
import { GeminiAdapter } from './gemini_adapter.js';
import { WebLLMAdapter } from './webllm_adapter.js';
import { WllamaAdapter } from './wllama_adapter.js';
import { HardwareDetector } from './hardware_detector.js';

export class LLMManager {
    constructor() {
        this.adapters = new Map();
        this.activeAdapter = null;
        this.availableAdapters = [];
    }

    /**
     * Initialize the manager and detect available adapters
     */
    async initialize() {
        console.log('[LLM Manager] Initializing...');

        // Register all adapters
        this.registerAdapter('chrome-ai', ChromeAIAdapter);
        this.registerAdapter('webllm', WebLLMAdapter);
        this.registerAdapter('wllama', WllamaAdapter);
        this.registerAdapter('openai', OpenAIAdapter);
        this.registerAdapter('claude', ClaudeAdapter);
        this.registerAdapter('gemini', GeminiAdapter);

        // Detect available adapters
        await this.detectAdapters();

        // Try to load saved configuration
        await this.loadConfig();

        console.log('[LLM Manager] Initialized with', this.availableAdapters.length, 'available adapters');
    }

    /**
     * Register an adapter class
     */
    registerAdapter(name, AdapterClass) {
        this.adapters.set(name, AdapterClass);
    }

    /**
     * Detect which adapters are available
     */
    async detectAdapters() {
        this.availableAdapters = [];

        for (const [name, AdapterClass] of this.adapters.entries()) {
            console.log('[LLM Manager] Checking availability for:', name);
            const available = await AdapterClass.isAvailable();
            console.log('[LLM Manager]', name, 'available:', available);

            if (available) {
                this.availableAdapters.push({
                    id: name,
                    name: name,
                    displayName: AdapterClass.getDisplayName(),
                    description: AdapterClass.getDescription(),
                    requiresApiKey: !name.includes('chrome'),
                });
                console.log('[LLM Manager] Detected available adapter:', name);
            } else {
                console.log('[LLM Manager] Adapter not available:', name);
            }
        }

        console.log('[LLM Manager] Total available adapters:', this.availableAdapters.length);
        return this.availableAdapters;
    }

    /**
     * Set the active adapter
     */
    async setActiveAdapter(name, model = null, config = {}) {
        if (!this.adapters.has(name)) {
            throw new Error(`Unknown adapter: ${name}`);
        }

        console.log('[LLM Manager] Setting active adapter:', name);

        const AdapterClass = this.adapters.get(name);
        const adapter = new AdapterClass();

        // Initialize the adapter
        await adapter.initialize(model, config);

        // Set as active
        this.activeAdapter = adapter;

        // Save configuration
        await this.saveConfig(name, model, config);

        console.log('[LLM Manager] Active adapter set:', name);

        return adapter;
    }

    /**
     * Get the current active adapter
     */
    getActiveAdapter() {
        return this.activeAdapter;
    }

    /**
     * Send a prompt using the active adapter
     */
    async prompt(messages, options = {}) {
        if (!this.activeAdapter) {
            throw new Error('No active LLM adapter. Please configure an LLM first.');
        }

        return await this.activeAdapter.prompt(messages, options);
    }

    /**
     * Stream a response using the active adapter
     */
    async stream(messages, options = {}, onChunk = null) {
        if (!this.activeAdapter) {
            throw new Error('No active LLM adapter. Please configure an LLM first.');
        }

        return await this.activeAdapter.stream(messages, options, onChunk);
    }

    /**
     * Get available adapters
     */
    getAvailableAdapters() {
        console.log('[LLM Manager] getAvailableAdapters called, returning:', this.availableAdapters);
        console.log('[LLM Manager] Adapters count:', this.availableAdapters.length);
        return this.availableAdapters;
    }

    /**
     * Get status of the active adapter
     */
    getStatus() {
        if (!this.activeAdapter) {
            return {
                ready: false,
                adapter: null,
                error: 'No adapter configured',
            };
        }

        // Get detailed status from adapter
        const adapterStatus = this.activeAdapter.getStatus();

        return {
            ready: this.activeAdapter.initialized,
            adapter: this.activeAdapter.name,
            current: this.activeAdapter.name,
            model: this.activeAdapter.model,
            error: this.activeAdapter.error,
            capabilities: this.activeAdapter.getCapabilities(),
            // Include download progress for local adapters
            downloadProgress: adapterStatus.downloadProgress,
            isLoading: adapterStatus.isLoading,
        };
    }

    /**
     * Load saved configuration from chrome.storage
     */
    async loadConfig() {
        try {
            const result = await chrome.storage.sync.get(['llmConfig']);

            // Check if first run (no config exists)
            if (!result.llmConfig || !result.llmConfig.selectedAdapter) {
                console.log('[LLM Manager] First run detected, auto-configuring...');
                await this.autoConfigureAdapter();
                return;
            }

            const config = result.llmConfig;
            console.log('[LLM Manager] Loading saved configuration:', config.selectedAdapter);

            // Try to initialize the saved adapter
            if (config.selectedAdapter && config.selectedModel) {
                try {
                    await this.setActiveAdapter(
                        config.selectedAdapter,
                        config.selectedModel,
                        config.apiKeys || {}
                    );
                    console.log('[LLM Manager] Restored previous adapter:', config.selectedAdapter);
                } catch (error) {
                    console.error('[LLM Manager] Failed to restore adapter:', error);
                    // If restore fails, try auto-config as fallback
                    await this.autoConfigureAdapter();
                }
            }
        } catch (error) {
            console.error('[LLM Manager] Failed to load config:', error);
        }
    }

    /**
     * Auto-configure adapter based on hardware capabilities
     */
    async autoConfigureAdapter() {
        try {
            // Detect hardware capabilities
            const hwProfile = await HardwareDetector.detect();
            console.log('[LLM Manager] Hardware profile:', hwProfile);

            // Get recommended adapter based on hardware and availability
            const recommended = HardwareDetector.getRecommendedAdapter(
                this.availableAdapters,
                hwProfile
            );

            if (!recommended) {
                console.log('[LLM Manager] No adapters available for auto-config');
                return;
            }

            console.log('[LLM Manager] Auto-configuring with:', recommended.id);

            // Determine default model for adapter
            let defaultModel = null;
            if (recommended.id === 'chrome-ai') {
                defaultModel = 'gemini-nano';
            } else if (recommended.id === 'webllm') {
                defaultModel = 'Phi-3-mini-4k-instruct-q4f16_1'; // Balanced 2.2GB
            } else if (recommended.id === 'wllama') {
                defaultModel = 'tinyllama'; // Smallest 669MB
            }

            // Initialize adapter
            await this.setActiveAdapter(recommended.id, defaultModel, {});

            console.log('[LLM Manager] Auto-configuration complete');
        } catch (error) {
            console.error('[LLM Manager] Auto-configuration failed:', error);
            // Non-fatal - user can configure manually
        }
    }

    /**
     * Save configuration to chrome.storage
     */
    async saveConfig(adapter, model, apiKeys = {}) {
        try {
            const config = {
                selectedAdapter: adapter,
                selectedModel: model,
                apiKeys: {
                    openai: apiKeys.apiKey && adapter === 'openai' ? apiKeys.apiKey : null,
                    claude: apiKeys.apiKey && adapter === 'claude' ? apiKeys.apiKey : null,
                    gemini: apiKeys.apiKey && adapter === 'gemini' ? apiKeys.apiKey : null,
                },
            };

            await chrome.storage.sync.set({ llmConfig: config });
            console.log('[LLM Manager] Configuration saved');
        } catch (error) {
            console.error('[LLM Manager] Failed to save config:', error);
        }
    }

    /**
     * Clear the active adapter and configuration
     */
    async clear() {
        this.activeAdapter = null;
        await chrome.storage.sync.remove(['llmConfig']);
        console.log('[LLM Manager] Configuration cleared');
    }

    /**
     * Get a specific adapter instance (for testing or advanced usage)
     */
    getAdapter(name) {
        if (!this.adapters.has(name)) {
            return null;
        }

        const AdapterClass = this.adapters.get(name);
        return new AdapterClass();
    }
}
