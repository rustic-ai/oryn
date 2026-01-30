/**
 * wllama Adapter
 *
 * Uses wllama for CPU-based local LLM inference via WebAssembly.
 * Provides universal browser support without requiring WebGPU.
 *
 * Features:
 * - CPU-based inference (works everywhere)
 * - GGUF model support from HuggingFace
 * - Inference: 2-10s per response
 * - Local and private
 *
 * See: https://github.com/ngxson/wllama
 */

import { LLMAdapter } from './llm_adapter.js';

// Dynamically import wllama from CDN
let Wllama = null;

async function loadWllama() {
    if (!Wllama) {
        try {
            const module = await import('https://esm.sh/@wllama/wllama@1.6.0');
            Wllama = module.Wllama;
            console.log('[wllama] Library loaded from CDN');
        } catch (error) {
            console.error('[wllama] Failed to load library:', error);
            throw new Error('Failed to load wllama library from CDN');
        }
    }
    return Wllama;
}

// GGUF model registry with verified HuggingFace URLs
const GGUF_MODELS = {
    'tinyllama': {
        url: 'https://huggingface.co/TheBloke/TinyLlama-1.1B-Chat-v1.0-GGUF/resolve/main/tinyllama-1.1b-chat-v1.0.Q4_K_M.gguf',
        size: '669MB',
        description: 'TinyLlama 1.1B - Fast and lightweight (recommended)',
        contextLength: 2048,
    },
    'phi2': {
        url: 'https://huggingface.co/TheBloke/phi-2-GGUF/resolve/main/phi-2.Q4_K_M.gguf',
        size: '1.6GB',
        description: 'Phi-2 2.7B - Better quality, slower',
        contextLength: 2048,
    },
    'gemma-2b': {
        url: 'https://huggingface.co/lmstudio-ai/gemma-2b-it-GGUF/resolve/main/gemma-2b-it-q4_k_m.gguf',
        size: '1.6GB',
        description: 'Gemma 2B - Google model, balanced',
        contextLength: 8192,
    },
};

export class WllamaAdapter extends LLMAdapter {
    constructor() {
        super('wllama');
        this.wllama = null;
        this.downloadProgress = 0;
        this.isLoading = false;
        this.modelConfig = null;
    }

    async initialize(modelId = 'tinyllama', config = {}) {
        try {
            console.log('[wllama] Initializing with model:', modelId);

            // Load wllama library
            const WllamaClass = await loadWllama();

            // Validate model
            if (!GGUF_MODELS[modelId]) {
                throw new Error(`Unknown model: ${modelId}. Available: ${Object.keys(GGUF_MODELS).join(', ')}`);
            }

            this.modelConfig = GGUF_MODELS[modelId];
            this.model = modelId;
            this.isLoading = true;
            this.downloadProgress = 0;

            // Create wllama instance with multi-threading
            console.log('[wllama] Creating instance...');
            this.wllama = new WllamaClass({
                useMultiThread: true,
                nThreads: config.nThreads || 4,
            });

            // Load model with progress callback
            console.log('[wllama] Loading model from:', this.modelConfig.url);
            await this.wllama.loadModelFromUrl(this.modelConfig.url, {
                progressCallback: (progress) => {
                    this.downloadProgress = progress.loaded / progress.total * 100;
                    console.log(`[wllama] Download progress: ${this.downloadProgress.toFixed(1)}%`);
                },
            });

            this.isLoading = false;
            this.initialized = true;
            this.error = null;

            console.log('[wllama] Initialized successfully');
            console.log('[wllama] Model info:', this.modelConfig);

        } catch (error) {
            console.error('[wllama] Initialization failed:', error);
            this.error = error.message;
            this.initialized = false;
            this.isLoading = false;
            throw error;
        }
    }

    async prompt(messages, options = {}) {
        if (!this.initialized || !this.wllama) {
            throw new Error('wllama adapter not initialized');
        }

        try {
            console.log('[wllama] Sending prompt with', messages.length, 'messages');

            // Format messages to prompt string (wllama doesn't have chat API)
            const prompt = this._formatMessages(messages);

            console.log('[wllama] Formatted prompt:', prompt.substring(0, 150) + '...');

            // Run completion
            const result = await this.wllama.createCompletion({
                prompt: prompt,
                n_predict: options.max_tokens || 512,
                temperature: options.temperature || 0.7,
                top_k: options.top_k || 40,
                top_p: options.top_p || 0.9,
                stop: ['User:', '\n\n\n', '</s>'],  // Stop tokens to prevent run-on
            });

            const content = result.trim();
            console.log('[wllama] Received response:', content.substring(0, 100) + '...');

            return content;
        } catch (error) {
            console.error('[wllama] Prompt failed:', error);
            throw error;
        }
    }

    async stream(messages, options = {}, onChunk = null) {
        if (!this.initialized || !this.wllama) {
            throw new Error('wllama adapter not initialized');
        }

        try {
            console.log('[wllama] Streaming prompt with', messages.length, 'messages');

            // Format messages to prompt string
            const prompt = this._formatMessages(messages);

            let fullResponse = '';

            // Run completion with streaming
            await this.wllama.createCompletion({
                prompt: prompt,
                n_predict: options.max_tokens || 512,
                temperature: options.temperature || 0.7,
                top_k: options.top_k || 40,
                top_p: options.top_p || 0.9,
                stop: ['User:', '\n\n\n', '</s>'],
                stream: true,
                onNewToken: (token, piece, currentText) => {
                    fullResponse += token;
                    if (onChunk) {
                        onChunk(token);
                    }
                },
            });

            console.log('[wllama] Stream complete');
            return fullResponse.trim();
        } catch (error) {
            console.error('[wllama] Streaming failed:', error);
            throw error;
        }
    }

    /**
     * Format messages array into a single prompt string
     * wllama requires manual prompt formatting (unlike WebLLM)
     * @private
     */
    _formatMessages(messages) {
        let prompt = '';

        for (const msg of messages) {
            if (msg.role === 'system') {
                // System messages go first without prefix
                prompt += `${msg.content}\n\n`;
            } else if (msg.role === 'user') {
                prompt += `User: ${msg.content}\n`;
            } else if (msg.role === 'assistant') {
                prompt += `Assistant: ${msg.content}\n`;
            }
        }

        // Add final assistant prompt to trigger response
        prompt += 'Assistant:';

        return prompt;
    }

    getCapabilities() {
        const modelInfo = this.modelConfig || {};
        return {
            name: this.name,
            type: 'local',
            maxTokens: modelInfo.contextLength || 2048,
            streaming: true,
            local: true,
            requiresWebGPU: false,
            models: Object.keys(GGUF_MODELS),
        };
    }

    getStatus() {
        return {
            ready: this.initialized,
            error: this.error,
            model: this.model || null,
            isLoading: this.isLoading,
            downloadProgress: this.downloadProgress,
        };
    }

    /**
     * Destroy the instance and free resources
     */
    async destroy() {
        if (this.wllama) {
            try {
                await this.wllama.exit();
                this.wllama = null;
                this.initialized = false;
                console.log('[wllama] Instance destroyed');
            } catch (error) {
                console.error('[wllama] Destroy failed:', error);
            }
        }
    }

    /**
     * Get model information
     */
    static getModelInfo(modelId) {
        return GGUF_MODELS[modelId] || null;
    }

    /**
     * Get all available models
     */
    static getAvailableModels() {
        return Object.entries(GGUF_MODELS).map(([id, info]) => ({
            id,
            ...info,
        }));
    }

    static async isAvailable() {
        try {
            console.log('[wllama] Checking availability...');

            // wllama works everywhere WebAssembly is supported
            if (typeof WebAssembly === 'undefined') {
                console.log('[wllama] Not available: WebAssembly not supported');
                return false;
            }

            // Check if we can instantiate WebAssembly (basic test)
            try {
                new WebAssembly.Memory({ initial: 1 });
            } catch (e) {
                console.log('[wllama] Not available: WebAssembly not functional');
                return false;
            }

            console.log('[wllama] Available with WebAssembly support');
            return true;
        } catch (error) {
            console.error('[wllama] Availability check failed:', error);
            return false;
        }
    }

    static getDisplayName() {
        return 'wllama (CPU-based)';
    }

    static getDescription() {
        return 'Local LLM inference using WebAssembly. Works on all browsers without GPU. Slower than WebGPU but more compatible.';
    }
}
