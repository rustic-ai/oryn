/**
 * Chrome AI Adapter
 *
 * Uses the Chrome built-in AI API (window.LanguageModel) to access Gemini Nano locally.
 * This is the fastest and most private option when available.
 *
 * Requires Chrome 127+ with Prompt API enabled.
 * See: https://developer.chrome.com/docs/ai/prompt-api
 *
 * Current API (2026): Uses global LanguageModel, not window.ai.languageModel
 */

import { LLMAdapter } from './llm_adapter.js';

export class ChromeAIAdapter extends LLMAdapter {
    constructor() {
        super('chrome-ai');
        this.session = null;
        this.capabilities = null;
    }

    async initialize(model = 'gemini-nano', config = {}) {
        try {
            // Check if LanguageModel API is available (current API)
            if (typeof self.LanguageModel === 'undefined') {
                throw new Error('Chrome AI not available. Requires Chrome 127+ with Prompt API enabled (chrome://flags/#prompt-api-for-gemini-nano)');
            }

            // Get availability status
            const availability = await self.LanguageModel.availability();
            console.log('[Chrome AI] Availability:', availability);

            if (availability === 'no') {
                throw new Error('Chrome AI language model is not available on this device.');
            }

            // If model needs to be downloaded, inform the user
            if (availability === 'after-download') {
                console.log('[Chrome AI] Model needs to be downloaded. Creating session will trigger download...');
            }

            // Create a session
            this.session = await self.LanguageModel.create({
                temperature: config.temperature || 0.7,
                topK: config.topK || 40,
            });

            this.model = model;
            this.initialized = true;
            this.error = null;

            console.log('[Chrome AI] Initialized successfully');
            console.log('[Chrome AI] Session created:', this.session);

            return;
        } catch (error) {
            console.error('[Chrome AI] Initialization failed:', error);
            this.error = error.message;
            this.initialized = false;
            throw error;
        }
    }

    async prompt(messages, options = {}) {
        if (!this.initialized || !this.session) {
            throw new Error('Chrome AI adapter not initialized');
        }

        try {
            // Convert messages to a single prompt string
            // Chrome AI doesn't support multi-turn conversations in the same way
            const prompt = this._formatMessages(messages);

            console.log('[Chrome AI] Sending prompt:', prompt.substring(0, 100) + '...');

            // Send prompt
            const response = await this.session.prompt(prompt);

            console.log('[Chrome AI] Received response:', response.substring(0, 100) + '...');

            return response;
        } catch (error) {
            console.error('[Chrome AI] Prompt failed:', error);
            throw error;
        }
    }

    async stream(messages, options = {}, onChunk = null) {
        if (!this.initialized || !this.session) {
            throw new Error('Chrome AI adapter not initialized');
        }

        try {
            const prompt = this._formatMessages(messages);

            console.log('[Chrome AI] Streaming prompt:', prompt.substring(0, 100) + '...');

            // Use promptStreaming
            const stream = await this.session.promptStreaming(prompt);

            let fullResponse = '';

            for await (const chunk of stream) {
                // Each chunk is the full text so far, not just the new part
                fullResponse = chunk;
                if (onChunk) {
                    onChunk(chunk);
                }
            }

            console.log('[Chrome AI] Stream complete');

            return fullResponse;
        } catch (error) {
            console.error('[Chrome AI] Streaming failed:', error);
            throw error;
        }
    }

    getCapabilities() {
        return {
            name: this.name,
            type: 'local',
            maxTokens: 1024,
            streaming: true,
            local: true,
            defaultTemperature: 0.7,
        };
    }

    /**
     * Format messages array into a single prompt string
     * @private
     */
    _formatMessages(messages) {
        let prompt = '';

        for (const msg of messages) {
            if (msg.role === 'system') {
                prompt += `System: ${msg.content}\n\n`;
            } else if (msg.role === 'user') {
                prompt += `User: ${msg.content}\n\n`;
            } else if (msg.role === 'assistant') {
                prompt += `Assistant: ${msg.content}\n\n`;
            }
        }

        // Add final assistant prompt
        prompt += 'Assistant: ';

        return prompt;
    }

    /**
     * Destroy the session
     */
    async destroy() {
        if (this.session) {
            await this.session.destroy();
            this.session = null;
            this.initialized = false;
        }
    }

    static async isAvailable() {
        try {
            console.log('[Chrome AI] Checking availability using current API...');

            // Check if running in a service worker or extension context
            if (typeof self === 'undefined') {
                console.log('[Chrome AI] Not available: self is undefined');
                return false;
            }

            // Check if LanguageModel API exists (current API, not window.ai.languageModel)
            if (typeof self.LanguageModel === 'undefined') {
                console.log('[Chrome AI] Not available: self.LanguageModel not found');
                console.log('[Chrome AI] Hint: Enable chrome://flags/#prompt-api-for-gemini-nano');
                return false;
            }

            // Check availability
            const availability = await self.LanguageModel.availability();
            console.log('[Chrome AI] Availability status:', availability);

            // Return true if immediately available OR after-download
            // We'll let the user trigger the download if needed
            const isAvailable = availability === 'available' || availability === 'after-download';
            console.log('[Chrome AI] Is available:', isAvailable);

            return isAvailable;
        } catch (error) {
            console.error('[Chrome AI] Availability check failed:', error);
            return false;
        }
    }

    static getDisplayName() {
        return 'Chrome AI (Gemini Nano)';
    }

    static getDescription() {
        return 'Local AI model built into Chrome. Fast, private, and free. Requires Chrome 127+ with Prompt API enabled.';
    }
}
