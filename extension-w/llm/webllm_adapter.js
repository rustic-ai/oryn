/**
 * WebLLM Adapter
 *
 * Uses WebLLM (MLC-AI) for GPU-accelerated local LLM inference via WebGPU.
 * Provides fast, private inference without cloud APIs.
 *
 * Features:
 * - GPU acceleration via WebGPU (Chrome 113+)
 * - Models: Llama-3, Phi-3, Gemma (quantized)
 * - Fast inference: 500-2000ms per response
 * - Local and private
 *
 * See: https://github.com/mlc-ai/web-llm
 */

import { LLMAdapter } from './llm_adapter.js';

// Dynamically import WebLLM from CDN
let webllm = null;

async function loadWebLLM() {
    if (!webllm) {
        try {
            webllm = await import('https://esm.sh/@mlc-ai/web-llm@0.2.59');
            console.log('[WebLLM] Library loaded from CDN');
        } catch (error) {
            console.error('[WebLLM] Failed to load library:', error);
            throw new Error('Failed to load WebLLM library from CDN');
        }
    }
    return webllm;
}

// Model registry with sizes and descriptions
const WEBLLM_MODELS = {
    'Phi-3-mini-4k-instruct-q4f16_1': {
        size: '2.2GB',
        description: 'Phi-3 Mini - Balanced quality and speed (recommended)',
        contextLength: 4096,
    },
    'Llama-3-8B-Instruct-q4f16_1': {
        size: '4.5GB',
        description: 'Llama-3 8B - Best quality, slower download',
        contextLength: 8192,
    },
    'Gemma-2B-it-q4f16_1': {
        size: '1.5GB',
        description: 'Gemma 2B - Smallest and fastest',
        contextLength: 8192,
    },
};

export class WebLLMAdapter extends LLMAdapter {
    constructor() {
        super('webllm');
        this.engine = null;
        this.downloadProgress = 0;
        this.isLoading = false;
    }

    async initialize(model = 'Phi-3-mini-4k-instruct-q4f16_1', config = {}) {
        try {
            console.log('[WebLLM] Initializing with model:', model);

            // Load WebLLM library
            const lib = await loadWebLLM();

            // Validate model
            if (!WEBLLM_MODELS[model]) {
                throw new Error(`Unknown model: ${model}. Available: ${Object.keys(WEBLLM_MODELS).join(', ')}`);
            }

            this.model = model;
            this.isLoading = true;
            this.downloadProgress = 0;

            // Create progress callback
            const initProgressCallback = (report) => {
                if (report.progress !== undefined) {
                    this.downloadProgress = report.progress * 100;
                    console.log(`[WebLLM] Download progress: ${this.downloadProgress.toFixed(1)}%`);
                }
                if (report.text) {
                    console.log(`[WebLLM] ${report.text}`);
                }
            };

            // Create MLC engine
            console.log('[WebLLM] Creating engine...');
            this.engine = await lib.CreateMLCEngine(model, {
                initProgressCallback: initProgressCallback,
            });

            this.isLoading = false;
            this.initialized = true;
            this.error = null;

            console.log('[WebLLM] Initialized successfully');
            console.log('[WebLLM] Model info:', WEBLLM_MODELS[model]);

        } catch (error) {
            console.error('[WebLLM] Initialization failed:', error);
            this.error = error.message;
            this.initialized = false;
            this.isLoading = false;
            throw error;
        }
    }

    async prompt(messages, options = {}) {
        if (!this.initialized || !this.engine) {
            throw new Error('WebLLM adapter not initialized');
        }

        try {
            console.log('[WebLLM] Sending prompt with', messages.length, 'messages');

            // WebLLM uses OpenAI-compatible API
            const response = await this.engine.chat.completions.create({
                messages: messages,
                temperature: options.temperature || 0.7,
                max_tokens: options.max_tokens || 512,
                stream: false,
            });

            const content = response.choices[0].message.content;
            console.log('[WebLLM] Received response:', content.substring(0, 100) + '...');

            return content;
        } catch (error) {
            console.error('[WebLLM] Prompt failed:', error);
            throw error;
        }
    }

    async stream(messages, options = {}, onChunk = null) {
        if (!this.initialized || !this.engine) {
            throw new Error('WebLLM adapter not initialized');
        }

        try {
            console.log('[WebLLM] Streaming prompt with', messages.length, 'messages');

            // WebLLM returns an async iterator for streaming
            const stream = await this.engine.chat.completions.create({
                messages: messages,
                temperature: options.temperature || 0.7,
                max_tokens: options.max_tokens || 512,
                stream: true,
            });

            let fullResponse = '';

            for await (const chunk of stream) {
                const content = chunk.choices[0]?.delta?.content;
                if (content) {
                    fullResponse += content;
                    if (onChunk) {
                        onChunk(content);
                    }
                }
            }

            console.log('[WebLLM] Stream complete');
            return fullResponse;
        } catch (error) {
            console.error('[WebLLM] Streaming failed:', error);
            throw error;
        }
    }

    getCapabilities() {
        const modelInfo = WEBLLM_MODELS[this.model] || {};
        return {
            name: this.name,
            type: 'local',
            maxTokens: modelInfo.contextLength || 4096,
            streaming: true,
            local: true,
            requiresWebGPU: true,
            models: Object.keys(WEBLLM_MODELS),
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
     * Destroy the engine and free resources
     */
    async destroy() {
        if (this.engine) {
            try {
                // WebLLM doesn't have explicit destroy, just unload
                this.engine = null;
                this.initialized = false;
                console.log('[WebLLM] Engine destroyed');
            } catch (error) {
                console.error('[WebLLM] Destroy failed:', error);
            }
        }
    }

    /**
     * Get model information
     */
    static getModelInfo(modelId) {
        return WEBLLM_MODELS[modelId] || null;
    }

    /**
     * Get all available models
     */
    static getAvailableModels() {
        return Object.entries(WEBLLM_MODELS).map(([id, info]) => ({
            id,
            ...info,
        }));
    }

    static async isAvailable() {
        try {
            console.log('[WebLLM] Checking availability...');

            // Check if running in a service worker or extension context
            if (typeof self === 'undefined' && typeof window === 'undefined') {
                console.log('[WebLLM] Not available: no global context');
                return false;
            }

            // Check for WebGPU support
            const nav = typeof self !== 'undefined' ? self.navigator : window.navigator;
            if (!nav.gpu) {
                console.log('[WebLLM] Not available: WebGPU not supported');
                console.log('[WebLLM] Hint: Requires Chrome 113+ with WebGPU enabled');
                return false;
            }

            // Try to request a WebGPU adapter
            const adapter = await nav.gpu.requestAdapter();
            if (!adapter) {
                console.log('[WebLLM] Not available: WebGPU adapter not available');
                return false;
            }

            console.log('[WebLLM] Available with WebGPU support');
            return true;
        } catch (error) {
            console.error('[WebLLM] Availability check failed:', error);
            return false;
        }
    }

    static getDisplayName() {
        return 'WebLLM (GPU-Accelerated)';
    }

    static getDescription() {
        return 'Local LLM inference using WebGPU. Fast, private, and runs entirely in your browser. Requires Chrome 113+ with WebGPU support.';
    }
}
