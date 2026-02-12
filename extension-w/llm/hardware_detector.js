/**
 * Hardware Detector Module
 * Centralized hardware capability detection for LLM adapter selection
 */

export class HardwareDetector {
    /**
     * Detect hardware capabilities
     * @returns {Promise<Object>} Hardware profile with capabilities and recommendations
     */
    static async detect() {
        const profile = {
            webgpu: await this.detectWebGPU(),
            chromeAI: await this.detectChromeAI(),
            ram: this.detectRAM(),
            timestamp: Date.now(),
            recommendations: []
        };

        // Generate recommendations based on capabilities
        profile.recommendations = this.generateRecommendations(profile);

        return profile;
    }

    /**
     * Detect WebGPU availability
     * @returns {Promise<Object>} WebGPU status and device info
     */
    static async detectWebGPU() {
        try {
            if (!navigator.gpu) {
                return { available: false, device: null, shaderF16: false };
            }

            const adapter = await navigator.gpu.requestAdapter();
            if (!adapter) {
                return { available: false, device: null, shaderF16: false };
            }

            return {
                available: true,
                device: adapter.name || 'Unknown GPU',
                shaderF16: adapter.features.has('shader-f16')
            };
        } catch (error) {
            console.warn('[HardwareDetector] WebGPU detection failed:', error);
            return { available: false, device: null, shaderF16: false };
        }
    }

    /**
     * Detect Chrome AI (Built-in Gemini Nano) availability
     * @returns {Promise<Object>} Chrome AI status
     */
    static async detectChromeAI() {
        try {
            // Check if the API exists (current API: LanguageModel, not self.ai)
            if (typeof LanguageModel === 'undefined') {
                return { available: false, status: 'unavailable' };
            }

            // Check availability status (current API returns string directly)
            const availability = await LanguageModel.availability();

            console.log('Chrome AI availability:', availability);

            if (availability === 'available') {
                return { available: true, status: 'ready' };
            } else if (availability === 'downloadable') {
                return { available: true, status: 'downloadable' };
            } else if (availability === 'downloading') {
                return { available: true, status: 'downloading' };
            } else {
                return { available: false, status: 'unavailable' };
            }
        } catch (error) {
            console.warn('[HardwareDetector] Chrome AI detection failed:', error);
            return { available: false, status: 'unavailable' };
        }
    }

    /**
     * Estimate RAM availability
     * @returns {Object} RAM estimation and sufficiency
     */
    static detectRAM() {
        try {
            // Use Device Memory API (Chrome 63+)
            const estimatedGB = navigator.deviceMemory || null;

            return {
                estimated: estimatedGB,
                sufficient: estimatedGB === null || estimatedGB >= 8
            };
        } catch (error) {
            console.warn('[HardwareDetector] RAM detection failed:', error);
            return { estimated: null, sufficient: true };
        }
    }

    /**
     * Check if an adapter is compatible with hardware profile
     * @param {string} adapterId - Adapter ID to check
     * @param {Object} hwProfile - Hardware profile from detect()
     * @returns {boolean} True if compatible
     */
    static isCompatible(adapterId, hwProfile) {
        const requirements = {
            'webllm': () => hwProfile.webgpu.available,
            'chrome-ai': () => hwProfile.chromeAI.available,
            'wllama': () => true, // Universal CPU fallback
            'openai': () => true, // Cloud APIs always work
            'claude': () => true,
            'gemini': () => true
        };

        const checkFn = requirements[adapterId];
        return checkFn ? checkFn() : true;
    }

    /**
     * Get warning message for adapter if incompatible
     * @param {string} adapterId - Adapter ID to check
     * @param {Object} hwProfile - Hardware profile from detect()
     * @returns {string|null} Warning message or null
     */
    static getWarning(adapterId, hwProfile) {
        const warnings = {
            'webllm': () => {
                if (!hwProfile.webgpu.available) {
                    return 'WebGPU not available. Requires Chrome 113+ and compatible GPU.';
                }
                if (hwProfile.ram.estimated && hwProfile.ram.estimated < 8) {
                    return 'Low RAM detected. This adapter may be slow or unstable with less than 8GB RAM.';
                }
                return null;
            },
            'chrome-ai': () => {
                if (hwProfile.chromeAI.status === 'unavailable') {
                    return 'Chrome AI not available. Enable at chrome://flags/#prompt-api-for-gemini-nano';
                }
                if (hwProfile.chromeAI.status === 'downloadable' || hwProfile.chromeAI.status === 'downloading') {
                    return 'Chrome AI model needs to be downloaded. This will happen automatically on first use.';
                }
                return null;
            },
            'wllama': () => {
                if (hwProfile.ram.estimated && hwProfile.ram.estimated < 4) {
                    return 'Low RAM detected. Small models (TinyLlama) recommended for this device.';
                }
                return null;
            }
        };

        const checkFn = warnings[adapterId];
        return checkFn ? checkFn() : null;
    }

    /**
     * Get recommended adapter based on hardware and available adapters
     * @param {Array} availableAdapters - List of available adapter objects
     * @param {Object} hwProfile - Hardware profile from detect()
     * @returns {Object|null} Recommended adapter object or null
     */
    static getRecommendedAdapter(availableAdapters, hwProfile) {
        // Priority order: Chrome AI > WebLLM > wllama > first remote with API key
        const priorities = ['chrome-ai', 'webllm', 'wllama'];

        // Check local adapters in priority order
        for (const adapterId of priorities) {
            const adapter = availableAdapters.find(a => a.id === adapterId);
            if (adapter && this.isCompatible(adapterId, hwProfile)) {
                console.log(`[HardwareDetector] Recommending ${adapterId} (priority match)`);
                return adapter;
            }
        }

        // Fallback to first available remote adapter
        const remoteAdapter = availableAdapters.find(a =>
            ['openai', 'claude', 'gemini'].includes(a.id)
        );

        if (remoteAdapter) {
            console.log(`[HardwareDetector] Recommending ${remoteAdapter.id} (remote fallback)`);
            return remoteAdapter;
        }

        console.warn('[HardwareDetector] No compatible adapters found');
        return null;
    }

    /**
     * Generate recommendations based on hardware profile
     * @param {Object} hwProfile - Hardware profile
     * @returns {Array<string>} List of recommendation strings
     */
    static generateRecommendations(hwProfile) {
        const recommendations = [];

        if (hwProfile.chromeAI.available && hwProfile.chromeAI.status === 'ready') {
            recommendations.push('Chrome AI is ready - instant and private AI with no downloads needed.');
        } else if (hwProfile.chromeAI.status === 'downloadable' || hwProfile.chromeAI.status === 'downloading') {
            recommendations.push('Chrome AI available after download - enable at chrome://flags/#prompt-api-for-gemini-nano');
        }

        if (hwProfile.webgpu.available) {
            recommendations.push('WebGPU detected - WebLLM will provide high-quality local AI with GPU acceleration.');
        }

        if (!hwProfile.webgpu.available && !hwProfile.chromeAI.available) {
            recommendations.push('No GPU acceleration available - wllama (CPU-based) is your best local option.');
        }

        if (hwProfile.ram.estimated && hwProfile.ram.estimated < 8) {
            recommendations.push(`Limited RAM (${hwProfile.ram.estimated}GB) - consider smaller models or cloud APIs for better performance.`);
        }

        if (recommendations.length === 0) {
            recommendations.push('All local and cloud adapters should work on your device.');
        }

        return recommendations;
    }
}
