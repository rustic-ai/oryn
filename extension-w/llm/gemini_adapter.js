/**
 * Gemini Adapter
 *
 * Uses the Google Generative AI API to access Gemini models.
 * Requires a Google AI API key.
 *
 * See: https://ai.google.dev/api/rest
 */

import { LLMAdapter } from './llm_adapter.js';

export class GeminiAdapter extends LLMAdapter {
    constructor() {
        super('gemini');
        this.apiKey = null;
        this.baseEndpoint = 'https://generativelanguage.googleapis.com/v1beta';
    }

    async initialize(model = 'gemini-1.5-flash', config = {}) {
        try {
            if (!config.apiKey) {
                throw new Error('Google AI API key is required');
            }

            this.apiKey = config.apiKey;
            this.model = model;
            this.baseEndpoint = config.endpoint || this.baseEndpoint;

            // Test the API key with a minimal request
            await this._testConnection();

            this.initialized = true;
            this.error = null;

            console.log('[Gemini] Initialized successfully with model:', this.model);
        } catch (error) {
            console.error('[Gemini] Initialization failed:', error);
            this.error = error.message;
            this.initialized = false;
            throw error;
        }
    }

    async prompt(messages, options = {}) {
        if (!this.initialized || !this.apiKey) {
            throw new Error('Gemini adapter not initialized');
        }

        try {
            const endpoint = `${this.baseEndpoint}/models/${this.model}:generateContent?key=${this.apiKey}`;

            const contents = this._formatMessages(messages);

            const response = await fetch(endpoint, {
                method: 'POST',
                headers: {
                    'Content-Type': 'application/json',
                },
                body: JSON.stringify({
                    contents: contents,
                    generationConfig: {
                        temperature: options.temperature || 0.7,
                        maxOutputTokens: options.max_tokens || 2048,
                    },
                }),
            });

            if (!response.ok) {
                const error = await response.json();
                throw new Error(`Gemini API error: ${error.error?.message || response.statusText}`);
            }

            const data = await response.json();

            if (!data.candidates || data.candidates.length === 0) {
                throw new Error('No response from Gemini API');
            }

            const candidate = data.candidates[0];
            if (!candidate.content || !candidate.content.parts) {
                throw new Error('Invalid response format from Gemini API');
            }

            // Extract text from parts
            return candidate.content.parts
                .filter(part => part.text)
                .map(part => part.text)
                .join('');
        } catch (error) {
            console.error('[Gemini] Prompt failed:', error);
            throw error;
        }
    }

    async stream(messages, options = {}, onChunk = null) {
        if (!this.initialized || !this.apiKey) {
            throw new Error('Gemini adapter not initialized');
        }

        try {
            const endpoint = `${this.baseEndpoint}/models/${this.model}:streamGenerateContent?key=${this.apiKey}`;

            const contents = this._formatMessages(messages);

            const response = await fetch(endpoint, {
                method: 'POST',
                headers: {
                    'Content-Type': 'application/json',
                },
                body: JSON.stringify({
                    contents: contents,
                    generationConfig: {
                        temperature: options.temperature || 0.7,
                        maxOutputTokens: options.max_tokens || 2048,
                    },
                }),
            });

            if (!response.ok) {
                const error = await response.json();
                throw new Error(`Gemini API error: ${error.error?.message || response.statusText}`);
            }

            const reader = response.body.getReader();
            const decoder = new TextDecoder();
            let fullResponse = '';

            while (true) {
                const { done, value } = await reader.read();
                if (done) break;

                const chunk = decoder.decode(value);

                // Gemini streams JSON objects separated by newlines
                const lines = chunk.split('\n').filter(line => line.trim() !== '');

                for (const line of lines) {
                    try {
                        const json = JSON.parse(line);

                        if (json.candidates && json.candidates.length > 0) {
                            const candidate = json.candidates[0];
                            if (candidate.content && candidate.content.parts) {
                                const text = candidate.content.parts
                                    .filter(part => part.text)
                                    .map(part => part.text)
                                    .join('');

                                if (text) {
                                    fullResponse += text;
                                    if (onChunk) {
                                        onChunk(text);
                                    }
                                }
                            }
                        }
                    } catch (e) {
                        // Ignore parse errors
                    }
                }
            }

            return fullResponse;
        } catch (error) {
            console.error('[Gemini] Streaming failed:', error);
            throw error;
        }
    }

    getCapabilities() {
        return {
            name: this.name,
            type: 'remote',
            maxTokens: this._getMaxTokens(),
            streaming: true,
            local: false,
            models: [
                'gemini-1.5-pro',
                'gemini-1.5-flash',
                'gemini-pro',
            ],
        };
    }

    _getMaxTokens() {
        // Gemini 1.5 models support up to 2M tokens context, but we limit output
        if (this.model.includes('1.5')) {
            return 8192;
        }
        return 2048;
    }

    /**
     * Format messages for Gemini API
     * Gemini uses a different message format with "contents" and "parts"
     * @private
     */
    _formatMessages(messages) {
        const contents = [];
        let systemInstruction = null;

        for (const msg of messages) {
            if (msg.role === 'system') {
                // Gemini treats system messages specially
                systemInstruction = msg.content;
                // For now, we'll prepend it to the first user message
                continue;
            }

            // Map OpenAI roles to Gemini roles
            let role = msg.role;
            if (role === 'assistant') {
                role = 'model';
            }

            contents.push({
                role: role,
                parts: [{ text: msg.content }],
            });
        }

        // Prepend system instruction to first user message if present
        if (systemInstruction && contents.length > 0 && contents[0].role === 'user') {
            contents[0].parts[0].text = `${systemInstruction}\n\n${contents[0].parts[0].text}`;
        }

        return contents;
    }

    async _testConnection() {
        // Test with a minimal request
        const endpoint = `${this.baseEndpoint}/models/${this.model}:generateContent?key=${this.apiKey}`;

        const response = await fetch(endpoint, {
            method: 'POST',
            headers: {
                'Content-Type': 'application/json',
            },
            body: JSON.stringify({
                contents: [
                    {
                        role: 'user',
                        parts: [{ text: 'Hi' }],
                    },
                ],
                generationConfig: {
                    maxOutputTokens: 10,
                },
            }),
        });

        if (!response.ok) {
            const error = await response.json();
            throw new Error(`API key validation failed: ${error.error?.message || response.statusText}`);
        }
    }

    static async isAvailable() {
        // Gemini API is always available if the user has an API key
        return true;
    }

    static getDisplayName() {
        return 'Gemini (Google AI)';
    }

    static getDescription() {
        return 'Google Gemini models via API. Requires an API key from aistudio.google.com. Fast and cost-effective.';
    }
}
