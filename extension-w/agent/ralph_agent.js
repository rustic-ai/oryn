/**
 * Ralph Agent
 *
 * Retrieval Augmented Language model for Planning in Hypertext
 * This agent uses few-shot learning from stored trajectories to make decisions
 * about web automation tasks.
 */

import { SYSTEM_PROMPT, buildPrompt, parseResponse, validateCommand } from './prompts.js';

export class RalphAgent {
    constructor(llmManager, trajectoryStore, config = {}) {
        this.llmManager = llmManager;
        this.trajectoryStore = trajectoryStore;

        // Configuration
        this.maxIterations = config.maxIterations || 10;
        // Lower default temperature for deterministic web automation
        this.temperature = config.temperature !== undefined ? config.temperature : 0.2;
        this.retrievalCount = config.retrievalCount || 3;
        this.maxRetries = config.maxRetries || 3;

        // Functions from background.js
        this.scanPageFn = config.scanPage || null;
        this.executeOilFn = config.executeOil || null;

        // State
        this.currentTask = null;
        this.currentIteration = 0;
        this.history = [];
        this.observation = null;
        this.previousScan = null; // Track last scan for diff
    }

    /**
     * Execute a task
     */
    async execute(task, tabId) {
        console.log('[Ralph Agent] Starting execution of task:', task);

        this.currentTask = task;
        this.currentIteration = 0;
        this.history = [];

        const startUrl = await this._getCurrentUrl(tabId);
        const startTime = Date.now();

        // Get initial observation
        this.observation = await this._scanPage(tabId);

        // Main agent loop
        while (this.currentIteration < this.maxIterations) {
            this.currentIteration++;

            console.log(`[Ralph Agent] Iteration ${this.currentIteration}/${this.maxIterations}`);

            try {
                // Make a decision
                const decision = await this.decide(task, this.observation);

                console.log('[Ralph Agent] Decision:', decision);

                // Check if complete
                if (decision.isComplete) {
                    console.log('[Ralph Agent] Task completed');
                    console.log('[Ralph Agent] User response:', decision.userResponse);

                    // Save successful trajectory
                    await this._saveTrajectory(task, startUrl, true, startTime);

                    return {
                        success: true,
                        iterations: this.currentIteration,
                        history: this.history,
                        finalState: this.observation,
                        response: decision.userResponse || 'Task completed successfully',
                    };
                }

                // Execute command
                const result = await this._executeCommand(decision.command, tabId);

                // Capture extracted data from extract commands
                const isExtract = decision.command.toLowerCase().startsWith('extract');
                const extractedData = isExtract ? result.result?.results ?? null : null;

                this.history.push({
                    iteration: this.currentIteration,
                    thought: decision.thought,
                    command: decision.command,
                    result: result.success,
                    message: result.message || result.error,
                    data: extractedData,
                });

                // Get new observation after command execution
                this.observation = await this._scanPage(tabId);

                // Track element changes
                if (this.previousScan && this.observation.elements) {
                    const oldIds = new Set(this.previousScan.elements.map(e => e.id));
                    const newIds = new Set(this.observation.elements.map(e => e.id));

                    const appeared = this.observation.elements.filter(e => !oldIds.has(e.id));
                    const disappeared = this.previousScan.elements.filter(e => !newIds.has(e.id));

                    if (appeared.length > 0 || disappeared.length > 0) {
                        this.observation.changes = {
                            appeared: appeared.length,
                            disappeared: disappeared.length,
                            summary: `+${appeared.length} -${disappeared.length} elements`
                        };
                    }
                }

                this.previousScan = this.observation;
            } catch (error) {
                console.error('[Ralph Agent] Iteration error:', error);

                this.history.push({
                    iteration: this.currentIteration,
                    thought: 'Error occurred',
                    command: null,
                    result: false,
                    message: error.message,
                });

                // Continue to next iteration
                continue;
            }
        }

        console.log('[Ralph Agent] Max iterations reached without completion');

        // Save failed trajectory
        await this._saveTrajectory(task, startUrl, false, startTime);

        return {
            success: false,
            iterations: this.currentIteration,
            history: this.history,
            finalState: this.observation,
            error: 'Max iterations reached',
        };
    }

    /**
     * Make a decision about the next action
     */
    async decide(task, observation) {
        console.log('[Ralph Agent] Making decision for task:', task);

        // Retrieve similar trajectories
        const trajectories = await this.trajectoryStore.retrieve(task, this.retrievalCount);
        console.log('[Ralph Agent] Retrieved', trajectories.length, 'similar trajectories');

        // Build prompt
        let userPrompt = buildPrompt(task, trajectories, observation, this.history);

        // Check for loops before making decision
        const loopDetection = this._detectLoop(this.history);
        if (loopDetection) {
            console.warn('[Ralph Agent] Loop detected:', loopDetection);
            // Add warning to prompt
            userPrompt += `\n⚠️ WARNING: ${loopDetection.suggestion}\n`;
            userPrompt += `Recent history shows: ${loopDetection.type}\n\n`;
        }

        // Get LLM response with retries
        let response = null;
        let lastError = null;

        for (let retry = 0; retry < this.maxRetries; retry++) {
            try {
                response = await this.llmManager.prompt([
                    { role: 'system', content: SYSTEM_PROMPT },
                    { role: 'user', content: userPrompt },
                ], {
                    temperature: this.temperature,
                    max_tokens: 500,
                });

                if (response && response.trim() !== '') {
                    break;
                }
            } catch (error) {
                console.error(`[Ralph Agent] LLM prompt failed (attempt ${retry + 1}):`, error);
                lastError = error;

                if (retry < this.maxRetries - 1) {
                    // Wait before retrying
                    await new Promise(resolve => setTimeout(resolve, 1000));
                }
            }
        }

        if (!response) {
            throw new Error(`Failed to get LLM response after ${this.maxRetries} attempts: ${lastError?.message}`);
        }

        console.log('[Ralph Agent] LLM response:', response);

        // Parse response
        const decision = parseResponse(response);

        if (!decision.thought) {
            console.warn('[Ralph Agent] No thought in response, using default');
            decision.thought = 'Continuing with task';
        }

        // Validate command if not complete
        if (!decision.isComplete) {
            if (!decision.command) {
                throw new Error('No command provided by LLM');
            }

            const validation = validateCommand(decision.command);
            if (!validation.valid) {
                console.warn('[Ralph Agent] Invalid command:', validation.error);
                // Try to continue anyway - the OIL executor might handle it
            }
        }

        return decision;
    }

    /**
     * Detect if agent is stuck in a loop
     */
    _detectLoop(history) {
        if (history.length < 3) return null;

        const recentCommands = history.slice(-3).map(h => h.command);

        // Check for repeated commands
        const uniqueCommands = new Set(recentCommands);
        if (uniqueCommands.size === 1) {
            return {
                type: 'repeated_command',
                command: recentCommands[0],
                count: 3,
                suggestion: 'Try a different approach - this command has been repeated 3 times'
            };
        }

        // Check for ping-pong (A, B, A, B pattern)
        if (history.length >= 4) {
            const last4 = history.slice(-4).map(h => h.command);
            if (last4[0] === last4[2] && last4[1] === last4[3]) {
                return {
                    type: 'ping_pong',
                    commands: [last4[0], last4[1]],
                    suggestion: 'Alternating between two commands - try something different'
                };
            }
        }

        return null;
    }

    /**
     * Execute an OIL command
     */
    async _executeCommand(command, tabId) {
        console.log('[Ralph Agent] Executing command:', command);

        try {
            const response = this.executeOilFn
                ? await this.executeOilFn(command, tabId)
                : await chrome.runtime.sendMessage({
                    type: 'execute_oil',
                    oil: command,
                    tabId: tabId,
                });

            if (response.error) {
                return { success: false, error: response.error };
            }

            return {
                success: true,
                message: 'Command executed successfully',
                result: response,
            };
        } catch (error) {
            console.error('[Ralph Agent] Command execution failed:', error);
            return { success: false, error: error.message };
        }
    }

    /**
     * Scan the current page
     */
    async _scanPage(tabId) {
        try {
            if (this.scanPageFn) {
                return await this.scanPageFn(tabId);
            }

            // Fallback to direct message (legacy)
            const response = await chrome.tabs.sendMessage(tabId, {
                action: 'scan',
                include_patterns: true,
            });

            if (response.error) {
                throw new Error(response.error);
            }

            return response;
        } catch (error) {
            console.error('[Ralph Agent] Failed to scan page:', error);
            throw error;
        }
    }

    /**
     * Get current URL from tab
     */
    async _getCurrentUrl(tabId) {
        try {
            const tab = await chrome.tabs.get(tabId);
            return tab.url;
        } catch (error) {
            console.error('[Ralph Agent] Failed to get URL:', error);
            return null;
        }
    }

    /**
     * Save trajectory to store
     */
    async _saveTrajectory(task, startUrl, success, startTime) {
        try {
            const trajectory = {
                task,
                url: startUrl,
                commands: this.history
                    .filter(h => h.command)
                    .map(h => h.command),
                success,
                timestamp: Date.now(),
                metadata: {
                    duration_ms: Date.now() - startTime,
                    iterations: this.currentIteration,
                    llm_used: this.llmManager.getActiveAdapter()?.name || 'unknown',
                },
            };

            await this.trajectoryStore.save(trajectory);
            console.log('[Ralph Agent] Trajectory saved');
        } catch (error) {
            console.error('[Ralph Agent] Failed to save trajectory:', error);
        }
    }

    /**
     * Get current agent state
     */
    getState() {
        return {
            currentTask: this.currentTask,
            currentIteration: this.currentIteration,
            maxIterations: this.maxIterations,
            historyLength: this.history.length,
            hasObservation: this.observation !== null,
        };
    }

    /**
     * Reset agent state
     */
    reset() {
        this.currentTask = null;
        this.currentIteration = 0;
        this.history = [];
        this.observation = null;
        this.previousScan = null;
    }
}
