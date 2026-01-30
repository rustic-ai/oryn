/**
 * Base LLM Adapter Interface
 *
 * All LLM adapters must implement this interface to provide a consistent
 * API for interacting with various AI providers (Chrome AI, OpenAI, Claude, Gemini, etc.)
 */

export class LLMAdapter {
    /**
     * @param {string} name - Adapter name (e.g., 'chrome-ai', 'openai', 'claude')
     */
    constructor(name) {
        this.name = name;
        this.initialized = false;
        this.error = null;
    }

    /**
     * Initialize the adapter with configuration
     * @param {string} model - Model name/ID to use
     * @param {Object} config - Configuration object (API keys, endpoints, etc.)
     * @returns {Promise<void>}
     */
    async initialize(model, config = {}) {
        throw new Error('initialize() must be implemented by subclass');
    }

    /**
     * Send a prompt and get a response
     * @param {Array<{role: string, content: string}>} messages - Conversation messages
     * @param {Object} options - Options like temperature, max_tokens, etc.
     * @returns {Promise<string>} - The response text
     */
    async prompt(messages, options = {}) {
        throw new Error('prompt() must be implemented by subclass');
    }

    /**
     * Stream a response (optional - can return null if not supported)
     * @param {Array<{role: string, content: string}>} messages - Conversation messages
     * @param {Object} options - Options like temperature, max_tokens, etc.
     * @param {Function} onChunk - Callback for each chunk
     * @returns {Promise<string>} - The full response text
     */
    async stream(messages, options = {}, onChunk = null) {
        // Default: just call prompt (no streaming)
        return await this.prompt(messages, options);
    }

    /**
     * Get adapter capabilities
     * @returns {Object} - Capabilities object
     */
    getCapabilities() {
        return {
            name: this.name,
            type: 'unknown',
            maxTokens: 4096,
            streaming: false,
            local: false,
        };
    }

    /**
     * Get adapter status
     * @returns {Object} - Status object
     */
    getStatus() {
        return {
            ready: this.initialized,
            error: this.error,
            model: this.model || null,
        };
    }

    /**
     * Check if this adapter is available in the current environment
     * @returns {Promise<boolean>}
     */
    static async isAvailable() {
        return false;
    }

    /**
     * Get a user-friendly name for this adapter
     * @returns {string}
     */
    static getDisplayName() {
        return 'Unknown Adapter';
    }

    /**
     * Get a description of this adapter
     * @returns {string}
     */
    static getDescription() {
        return 'No description available';
    }
}
