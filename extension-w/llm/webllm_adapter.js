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

// Dynamically import WebLLM from local bundle
let webllm = null;

async function loadWebLLM() {
    if (!webllm) {
        try {
            webllm = await import('./vendor/webllm.bundle.js');
            console.log('[WebLLM] Library loaded from local bundle');
        } catch (error) {
            console.error('[WebLLM] Failed to load library:', error);
            throw new Error('Failed to load WebLLM library');
        }
    }
    return webllm;
}

// Curated model catalog with f16/f32 variant pairs.
// When the GPU supports shader-f16, use the f16 variant (smaller, faster).
// Otherwise fall back to the f32 variant which works on all WebGPU devices.
const WEBLLM_MODEL_CATALOG = [
    {
        name: 'Phi 3 Mini',
        provider: 'Microsoft',
        f16: 'Phi-3-mini-4k-instruct-q4f16_1-MLC',
        f32: 'Phi-3-mini-4k-instruct-q4f32_1-MLC',
        sizeF16: '3.6GB',
        sizeF32: '5.5GB',
        contextLength: 4096,
        default: true,
    },
    {
        name: 'Llama 3.1 8B',
        provider: 'Meta',
        f16: 'Llama-3.1-8B-Instruct-q4f16_1-MLC',
        f32: 'Llama-3.1-8B-Instruct-q4f32_1-MLC',
        sizeF16: '5.0GB',
        sizeF32: '6.1GB',
        contextLength: 4096,
    },
    {
        name: 'Gemma 2 2B',
        provider: 'Google',
        f16: 'gemma-2-2b-it-q4f16_1-MLC',
        f32: 'gemma-2-2b-it-q4f32_1-MLC',
        sizeF16: '1.9GB',
        sizeF32: '2.5GB',
        contextLength: 4096,
    },
    {
        name: 'Gemma 2 9B',
        provider: 'Google',
        f16: 'gemma-2-9b-it-q4f16_1-MLC',
        f32: 'gemma-2-9b-it-q4f32_1-MLC',
        sizeF16: '6.4GB',
        sizeF32: '8.4GB',
        contextLength: 8192,
    },
    {
        name: 'Qwen 2.5 1.5B',
        provider: 'Alibaba',
        f16: 'Qwen2.5-1.5B-Instruct-q4f16_1-MLC',
        f32: 'Qwen2.5-1.5B-Instruct-q4f32_1-MLC',
        sizeF16: '1.6GB',
        sizeF32: '1.9GB',
        contextLength: 4096,
    },
    {
        name: 'Qwen 2.5 7B',
        provider: 'Alibaba',
        f16: 'Qwen2.5-7B-Instruct-q4f16_1-MLC',
        f32: 'Qwen2.5-7B-Instruct-q4f32_1-MLC',
        sizeF16: '5.1GB',
        sizeF32: '5.9GB',
        contextLength: 8192,
    },
    {
        name: 'DeepSeek-R1 Qwen 7B',
        provider: 'DeepSeek',
        f16: 'DeepSeek-R1-Distill-Qwen-7B-q4f16_1-MLC',
        f32: 'DeepSeek-R1-Distill-Qwen-7B-q4f32_1-MLC',
        sizeF16: '5.1GB',
        sizeF32: '5.9GB',
        contextLength: 8192,
    },
    {
        name: 'SmolLM2 1.7B',
        provider: 'Hugging Face',
        f16: 'SmolLM2-1.7B-Instruct-q4f16_1-MLC',
        f32: 'SmolLM2-1.7B-Instruct-q4f32_1-MLC',
        sizeF16: '1.8GB',
        sizeF32: '2.7GB',
        contextLength: 4096,
    },
];

// Build a lookup map from model ID to catalog entry for quick access
function _buildModelLookup() {
    const map = {};
    for (const entry of WEBLLM_MODEL_CATALOG) {
        map[entry.f16] = { ...entry, quantization: 'f16', size: entry.sizeF16 };
        map[entry.f32] = { ...entry, quantization: 'f32', size: entry.sizeF32 };
    }
    return map;
}
const WEBLLM_MODEL_LOOKUP = _buildModelLookup();

export class WebLLMAdapter extends LLMAdapter {
    constructor() {
        super('webllm');
        this.engine = null;
        this.downloadProgress = 0;
        this.isLoading = false;
    }

    async initialize(model = null, config = {}) {
        try {
            // Default to the f32 variant of the default model if no model specified
            if (!model) {
                const defaultEntry = WEBLLM_MODEL_CATALOG.find(m => m.default) || WEBLLM_MODEL_CATALOG[0];
                model = defaultEntry.f32;
            }

            console.log('[WebLLM] Initializing with model:', model);

            // Load WebLLM library
            const lib = await loadWebLLM();

            // Validate model is in our catalog
            if (!WEBLLM_MODEL_LOOKUP[model]) {
                console.warn(`[WebLLM] Model ${model} not in curated catalog, proceeding anyway`);
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
            console.log('[WebLLM] Model info:', WEBLLM_MODEL_LOOKUP[model] || model);

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
        const modelInfo = WEBLLM_MODEL_LOOKUP[this.model] || {};
        return {
            name: this.name,
            type: 'local',
            maxTokens: modelInfo.contextLength || 4096,
            streaming: true,
            local: true,
            requiresWebGPU: true,
            models: WEBLLM_MODEL_CATALOG.map(m => m.f16).concat(WEBLLM_MODEL_CATALOG.map(m => m.f32)),
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
     * Get model information by ID (works for both f16 and f32 variants)
     */
    static getModelInfo(modelId) {
        return WEBLLM_MODEL_LOOKUP[modelId] || null;
    }

    /**
     * Get all available models (returns f16 variants by default for backward compat)
     */
    static getAvailableModels() {
        return WEBLLM_MODEL_CATALOG.map(entry => ({
            id: entry.f16,
            size: entry.sizeF16,
            description: `${entry.name} - ${entry.provider}`,
            contextLength: entry.contextLength,
        }));
    }

    /**
     * Get models appropriate for the detected hardware.
     * Uses f16 variants when shader-f16 is supported, f32 otherwise.
     * @param {boolean} shaderF16 - Whether the GPU supports shader-f16
     * @returns {Array<Object>} Models with correct quantization variant
     */
    static getModelsForHardware(shaderF16) {
        return WEBLLM_MODEL_CATALOG.map(entry => ({
            id: shaderF16 ? entry.f16 : entry.f32,
            name: entry.name,
            provider: entry.provider,
            size: shaderF16 ? entry.sizeF16 : entry.sizeF32,
            quantization: shaderF16 ? 'f16' : 'f32',
            contextLength: entry.contextLength,
            default: entry.default || false,
        }));
    }

    /**
     * Get the raw model catalog (for use by wizard/UI)
     */
    static getModelCatalog() {
        return WEBLLM_MODEL_CATALOG;
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
