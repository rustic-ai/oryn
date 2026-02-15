/**
 * Claude Adapter
 *
 * Uses the Anthropic API to access Claude models (Claude 3.5 Sonnet, Claude 3 Opus, etc.)
 * Requires an Anthropic API key.
 *
 * See: https://docs.anthropic.com/claude/reference/messages_post
 */

import { LLMAdapter } from './llm_adapter.js';

export class ClaudeAdapter extends LLMAdapter {
    constructor() {
        super('claude');
        this.apiKey = null;
        this.endpoint = 'https://api.anthropic.com/v1/messages';
        this.apiVersion = '2023-06-01';
    }

    async initialize(model = 'claude-3-5-sonnet-20241022', config = {}) {
        try {
            if (!config.apiKey) {
                throw new Error('Anthropic API key is required');
            }

            this.apiKey = config.apiKey;
            this.model = model;
            this.endpoint = config.endpoint || this.endpoint;
            this.apiVersion = config.apiVersion || this.apiVersion;

            // Test the API key with a minimal request
            await this._testConnection();

            this.initialized = true;
            this.error = null;

            console.log('[Claude] Initialized successfully with model:', this.model);
        } catch (error) {
            console.error('[Claude] Initialization failed:', error);
            this.error = error.message;
            this.initialized = false;
            throw error;
        }
    }

    async prompt(messages, options = {}) {
        if (!this.initialized || !this.apiKey) {
            throw new Error('Claude adapter not initialized');
        }

        try {
            // Separate system message from conversation
            const { system, messages: conversationMessages } = this._formatMessages(messages);

            const requestBody = {
                model: this.model,
                messages: conversationMessages,
                max_tokens: options.max_tokens || 4096,
                temperature: options.temperature || 0.7,
            };

            if (system) {
                requestBody.system = system;
            }

            const response = await fetch(this.endpoint, {
                method: 'POST',
                headers: {
                    'Content-Type': 'application/json',
                    'x-api-key': this.apiKey,
                    'anthropic-version': this.apiVersion,
                },
                body: JSON.stringify(requestBody),
            });

            if (!response.ok) {
                const error = await response.json();
                throw new Error(`Claude API error: ${error.error?.message || response.statusText}`);
            }

            const data = await response.json();

            if (!data.content || data.content.length === 0) {
                throw new Error('No response from Claude API');
            }

            // Extract text from content blocks
            return data.content
                .filter(block => block.type === 'text')
                .map(block => block.text)
                .join('');
        } catch (error) {
            console.error('[Claude] Prompt failed:', error);
            throw error;
        }
    }

    async stream(messages, options = {}, onChunk = null) {
        if (!this.initialized || !this.apiKey) {
            throw new Error('Claude adapter not initialized');
        }

        try {
            const { system, messages: conversationMessages } = this._formatMessages(messages);

            const requestBody = {
                model: this.model,
                messages: conversationMessages,
                max_tokens: options.max_tokens || 4096,
                temperature: options.temperature || 0.7,
                stream: true,
            };

            if (system) {
                requestBody.system = system;
            }

            const response = await fetch(this.endpoint, {
                method: 'POST',
                headers: {
                    'Content-Type': 'application/json',
                    'x-api-key': this.apiKey,
                    'anthropic-version': this.apiVersion,
                },
                body: JSON.stringify(requestBody),
            });

            if (!response.ok) {
                const error = await response.json();
                throw new Error(`Claude API error: ${error.error?.message || response.statusText}`);
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

                        try {
                            const json = JSON.parse(data);

                            if (json.type === 'content_block_delta' && json.delta?.type === 'text_delta') {
                                const text = json.delta.text;
                                fullResponse += text;
                                if (onChunk) {
                                    onChunk(text);
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
            console.error('[Claude] Streaming failed:', error);
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
                'claude-3-5-sonnet-20241022',
                'claude-3-5-haiku-20241022',
                'claude-3-opus-20240229',
                'claude-3-sonnet-20240229',
                'claude-3-haiku-20240307',
            ],
        };
    }

    _getMaxTokens() {
        // All Claude 3 models support up to 200k context, but we limit output
        return 4096;
    }

    /**
     * Format messages for Claude API
     * Claude requires system messages to be separate and alternating user/assistant messages
     * @private
     */
    _formatMessages(messages) {
        let system = null;
        const conversationMessages = [];

        for (const msg of messages) {
            if (msg.role === 'system') {
                // Combine system messages
                system = system ? `${system}\n\n${msg.content}` : msg.content;
            } else {
                conversationMessages.push({
                    role: msg.role,
                    content: msg.content,
                });
            }
        }

        return { system, messages: conversationMessages };
    }

    async _testConnection() {
        // Test with a minimal request
        const response = await fetch(this.endpoint, {
            method: 'POST',
            headers: {
                'Content-Type': 'application/json',
                'x-api-key': this.apiKey,
                'anthropic-version': this.apiVersion,
            },
            body: JSON.stringify({
                model: this.model,
                messages: [{ role: 'user', content: 'Hi' }],
                max_tokens: 10,
            }),
        });

        if (!response.ok) {
            const error = await response.json();
            throw new Error(`API key validation failed: ${error.error?.message || response.statusText}`);
        }
    }

    static async isAvailable() {
        // Claude API is always available if the user has an API key
        return true;
    }

    static getDisplayName() {
        return 'Claude (Anthropic)';
    }

    static getDescription() {
        return 'Anthropic Claude models via API. Requires an API key from console.anthropic.com. Excellent reasoning and code understanding.';
    }
}
