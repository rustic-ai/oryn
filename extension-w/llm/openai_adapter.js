/**
 * OpenAI Adapter
 *
 * Uses the OpenAI API to access GPT models (GPT-4, GPT-3.5, etc.)
 * Requires an OpenAI API key.
 *
 * See: https://platform.openai.com/docs/api-reference/chat
 */

import { LLMAdapter } from './llm_adapter.js';

export class OpenAIAdapter extends LLMAdapter {
    constructor() {
        super('openai');
        this.apiKey = null;
        this.endpoint = 'https://api.openai.com/v1/chat/completions';
    }

    async initialize(model = 'gpt-4o-mini', config = {}) {
        try {
            if (!config.apiKey) {
                throw new Error('OpenAI API key is required');
            }

            this.apiKey = config.apiKey;
            this.model = model;
            this.endpoint = config.endpoint || this.endpoint;

            // Test the API key with a minimal request
            await this._testConnection();

            this.initialized = true;
            this.error = null;

            console.log('[OpenAI] Initialized successfully with model:', this.model);
        } catch (error) {
            console.error('[OpenAI] Initialization failed:', error);
            this.error = error.message;
            this.initialized = false;
            throw error;
        }
    }

    async prompt(messages, options = {}) {
        if (!this.initialized || !this.apiKey) {
            throw new Error('OpenAI adapter not initialized');
        }

        try {
            const response = await fetch(this.endpoint, {
                method: 'POST',
                headers: {
                    'Content-Type': 'application/json',
                    'Authorization': `Bearer ${this.apiKey}`,
                },
                body: JSON.stringify({
                    model: this.model,
                    messages: messages,
                    temperature: options.temperature || 0.7,
                    max_tokens: options.max_tokens || 2048,
                }),
            });

            if (!response.ok) {
                const error = await response.json();
                throw new Error(`OpenAI API error: ${error.error?.message || response.statusText}`);
            }

            const data = await response.json();

            if (!data.choices || data.choices.length === 0) {
                throw new Error('No response from OpenAI API');
            }

            return data.choices[0].message.content;
        } catch (error) {
            console.error('[OpenAI] Prompt failed:', error);
            throw error;
        }
    }

    async stream(messages, options = {}, onChunk = null) {
        if (!this.initialized || !this.apiKey) {
            throw new Error('OpenAI adapter not initialized');
        }

        try {
            const response = await fetch(this.endpoint, {
                method: 'POST',
                headers: {
                    'Content-Type': 'application/json',
                    'Authorization': `Bearer ${this.apiKey}`,
                },
                body: JSON.stringify({
                    model: this.model,
                    messages: messages,
                    temperature: options.temperature || 0.7,
                    max_tokens: options.max_tokens || 2048,
                    stream: true,
                }),
            });

            if (!response.ok) {
                const error = await response.json();
                throw new Error(`OpenAI API error: ${error.error?.message || response.statusText}`);
            }

            const reader = response.body.getReader();
            const decoder = new TextDecoder();
            let fullResponse = '';

            while (true) {
                const { done, value } = await reader.read();
                if (done) break;

                const chunk = decoder.decode(value);
                const lines = chunk.split('\n').filter(line => line.trim() !== '');

                for (const line of lines) {
                    if (line.startsWith('data: ')) {
                        const data = line.slice(6);
                        if (data === '[DONE]') continue;

                        try {
                            const json = JSON.parse(data);
                            const content = json.choices?.[0]?.delta?.content;

                            if (content) {
                                fullResponse += content;
                                if (onChunk) {
                                    onChunk(content);
                                }
                            }
                        } catch (e) {
                            // Ignore parse errors
                        }
                    }
                }
            }

            return fullResponse;
        } catch (error) {
            console.error('[OpenAI] Streaming failed:', error);
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
                'gpt-4o',
                'gpt-4o-mini',
                'gpt-4-turbo',
                'gpt-4',
                'gpt-3.5-turbo',
            ],
        };
    }

    _getMaxTokens() {
        // Return max tokens based on model
        if (this.model.includes('gpt-4')) {
            return 8192;
        } else if (this.model.includes('gpt-3.5')) {
            return 4096;
        }
        return 4096;
    }

    async _testConnection() {
        // Test with a minimal request
        const response = await fetch(this.endpoint, {
            method: 'POST',
            headers: {
                'Content-Type': 'application/json',
                'Authorization': `Bearer ${this.apiKey}`,
            },
            body: JSON.stringify({
                model: this.model,
                messages: [{ role: 'user', content: 'Hi' }],
                max_tokens: 5,
            }),
        });

        if (!response.ok) {
            const error = await response.json();
            throw new Error(`API key validation failed: ${error.error?.message || response.statusText}`);
        }
    }

    static async isAvailable() {
        // OpenAI API is always available if the user has an API key
        return true;
    }

    static getDisplayName() {
        return 'OpenAI (GPT-4, GPT-3.5)';
    }

    static getDescription() {
        return 'OpenAI GPT models via API. Requires an API key from platform.openai.com. Most capable models available.';
    }
}
