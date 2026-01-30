/**
 * Prompt Templates for Ralph Agent
 *
 * These templates are used to construct prompts for the LLM
 * with few-shot examples and current state information.
 */

/**
 * System prompt that explains the agent's role and capabilities
 */
export const SYSTEM_PROMPT = `You are a web automation agent that helps users complete tasks on websites using OIL (Oryn Intent Language).

OIL is a simple language for web automation with these commands:

BASIC ACTIONS:
- type "text" into "element" - Type text into an input field
- click "element" - Click on a button, link, or other clickable element
- press Enter - Press the Enter key
- scroll to "element" - Scroll to make an element visible

ELEMENT IDENTIFICATION:
Elements can be identified by:
- Text content: click "Submit"
- Type/role: click button, click link
- ID numbers: click [123] (from scan results)

OBSERVATION:
- observe - Scan the current page (default: smart filtered)
- observe --full - See ALL elements (use sparingly, verbose)
- observe --minimal - Minimal element scan
- observe --viewport - Only viewport elements
- observe --hidden - Include hidden elements

EXTRACTION (when data is visible):
- extract links - Extract all links from the page
- extract text - Extract all text content
- extract images - Extract all images
- extract tables - Extract all tables
- extract meta - Extract meta information
- extract links --selector "a.product" - Extract links matching CSS selector
- extract text --selector ".price" - Extract text from elements matching selector
- extract links --format json - Format output as JSON

IMPORTANT RULES:
1. Complete the task as requested by the user
2. Use observe to understand the page structure when needed
3. Be specific when identifying elements (e.g., "Search button" not just "button")
4. Execute tasks step by step - don't skip steps
5. When task is complete, provide a clear response explaining what you accomplished

Your responses must follow this format:
Thought: <brief reasoning about what to do next>
Action: <OIL command to execute>

Or when the task is done:
Thought: <why the task is complete>
Response: <explain what you accomplished and provide any relevant results in markdown>
Status: COMPLETE`;

/**
 * Score an element for relevance based on its properties
 */
function scoreElement(el, task) {
    let score = 0;

    // Base scoring by element type (interactive elements are higher value)
    if (el.type === 'button') score += 8;
    if (el.type === 'link' && el.text?.length > 0) score += 10;
    if (el.type === 'input') score += 9;
    if (el.type === 'select') score += 8;
    if (el.type === 'textarea') score += 8;

    // Boost elements with meaningful state
    if (el.state?.primary) score += 5;
    if (el.state?.visible !== false) score += 3;
    if (el.state?.checked !== undefined) score += 2;
    if (el.state?.selected !== undefined) score += 2;

    // Boost elements with substantial text content
    const textLength = (el.text || el.label || el.placeholder || '').length;
    if (textLength > 20) score += 5;
    if (textLength > 50) score += 3;

    // Boost elements with specific roles
    if (el.role === 'submit') score += 6;
    if (el.role === 'search') score += 5;
    if (el.role === 'navigation') score += 4;

    // Penalize noise elements (empty containers, decorative elements)
    if (el.type === 'div' && !el.text) score -= 5;
    if (el.type === 'span' && !el.text) score -= 4;
    if (el.text === '' || el.text?.match(/^[\s-|]+$/)) score -= 3;

    // Penalize very small elements (likely icons/decorative)
    if (el.rect?.width < 10 || el.rect?.height < 10) score -= 10;

    // Penalize hidden/disabled elements
    if (el.state?.visible === false) score -= 15;
    if (el.state?.disabled) score -= 8;

    return Math.max(0, score);
}

/**
 * Smart filter elements by relevance
 */
function smartFilter(elements, task, maxElements = 30) {
    // Score all elements
    const scored = elements.map(el => ({
        element: el,
        score: scoreElement(el, task)
    }));

    // Sort by score descending
    scored.sort((a, b) => b.score - a.score);

    // Take top N
    return scored.slice(0, maxElements).map(item => item.element);
}

/**
 * Build the main prompt with few-shot examples
 */
export function buildPrompt(task, trajectories, observation, history) {
    let prompt = '';

    // Add few-shot examples from trajectories (limit to 2 to save tokens)
    if (trajectories && trajectories.length > 0) {
        prompt += '\n## SUCCESSFUL EXAMPLES\n\n';

        const examples = trajectories.slice(0, 2);

        examples.forEach((traj, i) => {
            prompt += `Example ${i + 1}: ${traj.task}\n`;
            prompt += `Starting URL: ${traj.url}\n`;
            prompt += `Steps taken:\n`;

            traj.commands.slice(0, 5).forEach((cmd, j) => {
                prompt += `  ${j + 1}. ${cmd}\n`;
            });

            if (traj.commands.length > 5) {
                prompt += `  ... (${traj.commands.length - 5} more steps)\n`;
            }

            prompt += `Result: ✓ Success (${traj.commands.length} total steps)\n\n`;
        });

        if (trajectories.length > examples.length) {
            prompt += `(${trajectories.length - examples.length} more similar examples available)\n\n`;
        }
    }

    // Add current task
    prompt += `\nCURRENT TASK: ${task}\n\n`;

    // Add page observation
    if (observation) {
        prompt += 'CURRENT PAGE STATE:\n';
        prompt += `URL: ${observation.url || 'unknown'}\n`;
        prompt += `Title: ${observation.title || 'unknown'}\n`;

        // Add page changes if detected
        if (observation.changes) {
            prompt += `\nPage changes since last action: ${observation.changes.summary}\n`;
            if (observation.changes.appeared > 0) {
                prompt += `New elements appeared - content may have loaded\n`;
            }
        }

        if (observation.elements && observation.elements.length > 0) {
            const totalElements = observation.elements.length;
            const showingCount = Math.min(30, totalElements);

            // Count by type
            const typeCounts = {};
            observation.elements.forEach(el => {
                const type = el.type;
                typeCounts[type] = (typeCounts[type] || 0) + 1;
            });

            // Format type summary
            const topTypes = Object.entries(typeCounts)
                .sort((a, b) => b[1] - a[1])
                .slice(0, 5)
                .map(([type, count]) => `${type}:${count}`)
                .join(', ');

            prompt += `\n# SCAN SUMMARY\n`;
            prompt += `Showing ${showingCount} of ${totalElements} elements (smart filtered by relevance)\n`;
            prompt += `Element types: ${topTypes}\n`;

            if (totalElements > 30) {
                prompt += `Hidden: ${totalElements - 30} elements (mostly layout/noise)\n`;
                prompt += `To see more: use "observe --full" in next action\n`;
            }

            // Scroll context
            if (observation.page?.scroll) {
                const scroll = observation.page.scroll;
                if (scroll.max_y > 0) {
                    const scrollPercent = Math.round((scroll.y / scroll.max_y) * 100);
                    prompt += `Page scroll: ${scrollPercent}% (more content below)\n`;
                }
            }

            prompt += '\nAvailable elements:\n';

            // Use smart filtering instead of simple slice
            const elementsToShow = smartFilter(observation.elements, task, 30);

            for (const el of elementsToShow) {
                const typeStr = el.role ? `${el.type}/${el.role}` : el.type;
                const label = el.text || el.label || el.placeholder || '';
                const id = el.id ? `[${el.id}]` : '';

                // Build state flags
                const flags = [];
                if (el.state?.checked) flags.push('checked');
                if (el.state?.selected) flags.push('selected');
                if (el.state?.disabled) flags.push('disabled');
                if (el.state?.primary) flags.push('primary');

                const flagsStr = flags.length > 0 ? ` {${flags.join(', ')}}` : '';

                prompt += `${id} ${typeStr} "${label}"${flagsStr}\n`;
            }

            if (observation.elements.length > 30) {
                prompt += `\n(Use "observe" to refresh the scan)\n`;
            }
        }

        // Add detected patterns with actionable information
        if (observation.patterns) {
            const patterns = observation.patterns;
            const hasPatterns = Object.keys(patterns).some(key => patterns[key]);

            if (hasPatterns) {
                prompt += '\n# DETECTED PATTERNS\n';

                // Login form
                if (patterns.login) {
                    const login = patterns.login;
                    prompt += `✓ Login Form (${Math.round(login.confidence * 100)}% confidence):\n`;
                    if (login.email) prompt += `  - Email: [${login.email}]\n`;
                    if (login.username) prompt += `  - Username: [${login.username}]\n`;
                    if (login.password) prompt += `  - Password: [${login.password}]\n`;
                    if (login.submit) prompt += `  - Submit: [${login.submit}]\n`;
                    prompt += `  → ACTION: type credentials, click [${login.submit}]\n\n`;
                }

                // Search
                if (patterns.search) {
                    const search = patterns.search;
                    prompt += `✓ Search Box:\n`;
                    prompt += `  - Input: [${search.input}]\n`;
                    if (search.submit) prompt += `  - Submit: [${search.submit}]\n`;
                    prompt += `  → ACTION: type query into [${search.input}]${search.submit ? `, click [${search.submit}]` : ''}\n\n`;
                }

                // Pagination
                if (patterns.pagination) {
                    const pagination = patterns.pagination;
                    prompt += `✓ Pagination:\n`;
                    if (pagination.prev) prompt += `  - Previous: [${pagination.prev}]\n`;
                    if (pagination.next) prompt += `  - Next: [${pagination.next}]\n`;
                    if (pagination.pages && pagination.pages.length > 0) {
                        prompt += `  - Pages: ${pagination.pages.slice(0, 5).join(', ')}${pagination.pages.length > 5 ? '...' : ''}\n`;
                    }
                    prompt += `  → ACTION: click [${pagination.next}] to see more results\n\n`;
                }

                // Modal
                if (patterns.modal) {
                    prompt += `✓ Modal/Dialog detected\n`;
                    if (patterns.modal.close) prompt += `  → ACTION: click [${patterns.modal.close}] to close\n\n`;
                }

                // Cookie banner
                if (patterns.cookie_banner) {
                    prompt += `✓ Cookie Banner detected\n`;
                    prompt += `  → ACTION: May need to dismiss before proceeding\n\n`;
                }
            }
        }
    }

    // Add execution history
    if (history && history.length > 0) {
        prompt += '\nPREVIOUS ACTIONS:\n';
        for (const item of history) {
            prompt += `Thought: ${item.thought}\n`;
            prompt += `Action: ${item.command}\n`;
            prompt += `Result: ${item.result ? 'Success' : 'Failed'}\n\n`;
        }
    }

    // Add instruction for next action
    prompt += '\nBased on the task, examples, and current page state, what should be the next action?\n';
    prompt += 'Respond in the format:\n';
    prompt += 'Thought: <your reasoning>\n';
    prompt += 'Action: <OIL command>\n\n';
    prompt += 'Or if the task is complete:\n';
    prompt += 'Thought: <why complete>\n';
    prompt += 'Status: COMPLETE\n';

    return prompt;
}

/**
 * Parse the LLM response to extract thought, command, and response
 */
export function parseResponse(response) {
    const lines = response.trim().split('\n');

    let thought = '';
    let command = '';
    let userResponse = '';
    let isComplete = false;

    for (let i = 0; i < lines.length; i++) {
        const trimmedLine = lines[i].trim();

        if (trimmedLine.toLowerCase().startsWith('thought:')) {
            thought = trimmedLine.substring(8).trim();
        } else if (trimmedLine.toLowerCase().startsWith('action:')) {
            command = trimmedLine.substring(7).trim();
        } else if (trimmedLine.toLowerCase().startsWith('response:')) {
            // Response may span multiple lines, collect everything after "Response:"
            userResponse = trimmedLine.substring(9).trim();
            // Append subsequent lines that aren't other fields
            for (let j = i + 1; j < lines.length; j++) {
                const nextLine = lines[j].trim();
                if (nextLine.toLowerCase().startsWith('status:') ||
                    nextLine.toLowerCase().startsWith('thought:') ||
                    nextLine.toLowerCase().startsWith('action:')) {
                    break;
                }
                userResponse += '\n' + lines[j];
            }
        } else if (trimmedLine.toLowerCase().startsWith('status:')) {
            const status = trimmedLine.substring(7).trim().toLowerCase();
            if (status === 'complete' || status === 'done' || status === 'finished') {
                isComplete = true;
            }
        }
    }

    return {
        thought,
        command,
        userResponse: userResponse.trim(),
        isComplete,
    };
}

/**
 * Validate that a command is valid OIL syntax (basic check)
 */
export function validateCommand(command) {
    if (!command || command.trim() === '') {
        return { valid: false, error: 'Empty command' };
    }

    const cmd = command.toLowerCase().trim();

    // Check for common OIL patterns
    const validPatterns = [
        /^observe(\s+--\w+)*$/,
        /^extract\s+(links|text|images|tables|meta)/,
        /^type\s+".+"\s+into\s+.+/,
        /^click\s+.+/,
        /^press\s+(enter|escape|tab)/i,
        /^scroll(\s+to\s+.+)?/,
        /^goto\s+.+/,
        /^wait\s+\d+/,
        /^login\s+".+"\s+".+"/,
        /^search\s+".+"/,
    ];

    if (validPatterns.some(pattern => pattern.test(cmd))) {
        return { valid: true };
    }

    return {
        valid: false,
        error: 'Command does not match known OIL syntax patterns',
    };
}
