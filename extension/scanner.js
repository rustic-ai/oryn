/**
 * Oryn Universal Scanner
 * Version 1.0
 *
 * Implements the scanner protocol for element discovery, interaction, and state extraction.
 * Works across Embedded (oryn-e), Headless (oryn-h), and Remote (oryn-r) backends.
 */
(function (global) {
    // --- State ---

    const STATE = {
        elementMap: new Map(),
        inverseMap: new WeakMap(),
        cache: new Map(),
        nextId: 1,
        config: { debug: false }
    };

    // --- State Management ---
    const StateManager = {
        invalidate: () => {
            STATE.elementMap.clear();
            STATE.inverseMap = new WeakMap();
            STATE.cache.clear();
            STATE.nextId = 1;
            if (STATE.config.debug) console.log('Scanner state invalidated due to navigation');
        },

        init: () => {
            window.addEventListener('hashchange', StateManager.invalidate);
            window.addEventListener('popstate', StateManager.invalidate);

            const wrapHistoryMethod = (methodName) => {
                const original = window.history[methodName];
                window.history[methodName] = function (...args) {
                    original.apply(this, args);
                    StateManager.invalidate();
                };
            };

            wrapHistoryMethod('pushState');
            wrapHistoryMethod('replaceState');
        }
    };
    StateManager.init();

    // --- Protocol ---

    const Protocol = {
        success: (result = {}, timingStart = null) => {
            const response = { status: 'ok', ...result };
            if (timingStart) response.timing = { duration_ms: performance.now() - timingStart };
            return response;
        },
        error: (msg, code = 'UNKNOWN_ERROR') => ({ status: 'error', error: msg, code, message: msg })
    };

    // --- Helpers ---

    const Utils = {
        isVisible: (el) => {
            if (!el.isConnected) return false;
            // Check robust visibility: layout size, style, and ancestry
            const rect = el.getBoundingClientRect();
            if (rect.width === 0 || rect.height === 0) return false;

            // Use element's own document view for computed style (supports iframe elements)
            const win = el.ownerDocument.defaultView || window;
            const style = win.getComputedStyle(el);
            if (style.visibility === 'hidden' || style.display === 'none' || style.opacity === '0') return false;

            // Allow checking ancestry efficiently?
            // For now, basic checks. deep hidden checks can be expensive on full scan.
            return true;
        },

        isInViewport: (el) => {
            const rect = el.getBoundingClientRect();
            return rect.top < window.innerHeight && rect.bottom > 0 && rect.left < window.innerWidth && rect.right > 0;
        },

        generateSelector: (el) => {
            const rootNode = el.getRootNode();
            const isUnique = (selector) => rootNode.querySelectorAll(selector).length === 1;
            const isValidId = (id) => /^[a-zA-Z][a-zA-Z0-9_-]*$/.test(id);

            // Priority 1: ID (if valid and unique)
            if (el.id && isValidId(el.id)) {
                const selector = `#${CSS.escape(el.id)}`;
                if (isUnique(selector)) return selector;
            }

            // Priority 2: data-testid
            const testId = el.getAttribute('data-testid');
            if (testId) {
                const selector = `[data-testid="${CSS.escape(testId)}"]`;
                if (isUnique(selector)) return selector;
            }

            // Priority 3: Aria Label (if unique)
            const ariaLabel = el.getAttribute('aria-label');
            if (ariaLabel) {
                const selector = `${el.tagName.toLowerCase()}[aria-label="${CSS.escape(ariaLabel)}"]`;
                if (isUnique(selector)) return selector;
            }

            // Priority 4: Unique Class Combination
            if (el.className && typeof el.className === 'string') {
                const classes = el.className.split(/\s+/).filter((c) => c.trim().length > 0);
                if (classes.length > 0) {
                    const selector = `${el.tagName.toLowerCase()}.${classes.map((c) => CSS.escape(c)).join('.')}`;
                    if (isUnique(selector)) return selector;
                }
            }

            // Fallback: structural path
            const path = [];
            let current = el;
            while (current && current.nodeType === Node.ELEMENT_NODE) {
                const tag = current.tagName.toLowerCase();
                if (current.id && isValidId(current.id)) {
                    path.unshift(`#${CSS.escape(current.id)}`);
                    break;
                }
                let sibling = current;
                let nth = 1;
                while ((sibling = sibling.previousElementSibling)) {
                    if (sibling.tagName.toLowerCase() === tag) nth++;
                }
                path.unshift(`${tag}:nth-of-type(${nth})`);
                current = current.parentNode;
                // Stop at shadow root boundary
                if (current === rootNode) break;
            }
            return path.join(' > ');
        },

        getXPath: (el) => {
            if (el.id) return `//*[@id="${el.id}"]`;
            const path = [];
            while (el.nodeType === Node.ELEMENT_NODE) {
                const tag = el.tagName.toLowerCase();
                let sibling = el,
                    nth = 1;
                while ((sibling = sibling.previousElementSibling)) {
                    if (sibling.tagName.toLowerCase() === tag) nth++;
                }
                path.unshift(`${tag}[${nth}]`);
                el = el.parentNode;
            }
            return '/' + path.join('/');
        },

        getClassName: (el) => {
            try {
                if (typeof el.className === 'string') {
                    return el.className.toLowerCase();
                }
                if (el.className && el.className.baseVal !== undefined) {
                    return el.className.baseVal.toLowerCase();
                }
                if (el.className && typeof el.className.toString === 'function') {
                    return el.className.toString().toLowerCase();
                }
            } catch (_e) {
                // Ignore errors from edge cases
            }
            return '';
        },

        detectRole: (el) => {
            const tag = el.tagName.toLowerCase();
            const type = el.getAttribute('type')?.toLowerCase();
            const role = el.getAttribute('role');

            // Input type detection with pattern matching
            if (tag === 'input') {
                if (type === 'submit') return 'submit';
                if (['button', 'image', 'reset'].includes(type)) return 'button';
                if (['checkbox', 'radio'].includes(type)) return type;

                const hints = Utils.getFieldHints(el);
                const inputRoles = [
                    { role: 'search', types: ['search'], names: ['search', 'q', 'query'], keywords: ['search'] },
                    { role: 'email', types: ['email'], names: [], keywords: ['email'] },
                    { role: 'username', types: [], names: ['username', 'user', 'login'], keywords: ['username'], autocomplete: ['username', 'nickname'] },
                    { role: 'password', types: ['password'], names: [], keywords: [], autocomplete: ['password'] },
                    { role: 'tel', types: ['tel'], names: [], keywords: ['phone'], autocomplete: ['tel'] },
                    { role: 'url', types: ['url'], names: [], keywords: ['website'], autocomplete: ['url'] }
                ];

                for (const r of inputRoles) {
                    if (r.types.includes(type)) return r.role;
                    if (r.autocomplete?.some((ac) => hints.autocomplete.includes(ac))) return r.role;
                    if (r.names.includes(hints.name)) return r.role;
                    if (r.keywords.some((kw) => hints.name.includes(kw) || hints.placeholder.includes(kw) || hints.label.includes(kw) || hints.ariaLabel.includes(kw))) return r.role;
                }

                return 'input';
            }

            if (tag === 'textarea') return 'textarea';
            if (tag === 'select') return 'select';

            if (tag === 'button') {
                if (el.getAttribute('type') === 'submit') return 'submit';
                return Utils.isPrimaryButton(el) ? 'primary' : 'button';
            }

            if (tag === 'a' && el.hasAttribute('href')) return 'link';

            if (role === 'button') return Utils.isPrimaryButton(el) ? 'primary' : 'button';
            if (role === 'checkbox') return 'checkbox';
            if (role === 'link') return 'link';

            const TAG_ROLES = {
                h1: 'heading', h2: 'heading', h3: 'heading', h4: 'heading', h5: 'heading', h6: 'heading',
                label: 'text', strong: 'text', b: 'text', em: 'text', span: 'text', p: 'text',
                li: 'listitem',
                td: 'cell', th: 'cell'
            };

            return TAG_ROLES[tag] || 'generic';
        },

        getLabelText: (el) => {
            const rootNode = el.getRootNode();

            if (el.id) {
                const label = rootNode.querySelector(`label[for="${el.id}"]`);
                if (label) return label.textContent || '';
            }

            const parentLabel = el.closest('label');
            if (parentLabel) return parentLabel.textContent || '';

            const labelledBy = el.getAttribute('aria-labelledby');
            if (labelledBy) {
                const labelEl = rootNode.getElementById?.(labelledBy);
                if (labelEl) return labelEl.textContent || '';
            }

            return '';
        },

        getFieldHints: (el) => ({
            autocomplete: (el.getAttribute('autocomplete') || '').toLowerCase(),
            name: (el.getAttribute('name') || '').toLowerCase(),
            placeholder: (el.getAttribute('placeholder') || '').toLowerCase(),
            ariaLabel: (el.getAttribute('aria-label') || '').toLowerCase(),
            label: Utils.getLabelText(el).toLowerCase()
        }),

        getClickCoordinates: (el, offset) => {
            const rect = el.getBoundingClientRect();
            if (offset) {
                return { x: rect.left + (offset.x || 0), y: rect.top + (offset.y || 0) };
            }
            return { x: rect.left + rect.width / 2, y: rect.top + rect.height / 2 };
        },

        createChangeTracker: () => {
            const initialUrl = window.location.href;
            const domChanges = { added: 0, removed: 0, attributes: 0 };

            const processMutations = (mutations) => {
                for (const m of mutations) {
                    if (m.type === 'childList') {
                        domChanges.added += m.addedNodes.length;
                        domChanges.removed += m.removedNodes.length;
                    } else if (m.type === 'attributes') {
                        domChanges.attributes++;
                    }
                }
            };

            const observer = new window.MutationObserver(processMutations);
            observer.observe(document.body, { childList: true, subtree: true, attributes: true });

            return {
                get navigationDetected() {
                    return window.location.href !== initialUrl;
                },
                domChanges,
                cleanup: () => {
                    processMutations(observer.takeRecords());
                    observer.disconnect();
                }
            };
        },

        isPrimaryButton: (el) => {
            const className = Utils.getClassName(el);
            const text = (el.textContent || '').toLowerCase().trim();

            // Check for primary button indicators
            if (
                className.includes('primary') ||
                className.includes('btn-primary') ||
                className.includes('main') ||
                className.includes('cta')
            ) {
                return true;
            }

            // Check if it's the submit button in a form
            const form = el.closest('form');
            if (form) {
                const isOnlySubmit = form.querySelectorAll('button[type="submit"], input[type="submit"]').length === 1;
                const isSubmitType = el.getAttribute('type') === 'submit';
                if (isOnlySubmit && isSubmitType) return true;
            }

            // Common primary action text patterns
            const primaryTexts = [
                'submit',
                'sign in',
                'log in',
                'login',
                'continue',
                'next',
                'save',
                'confirm',
                'proceed'
            ];
            if (primaryTexts.some((t) => text === t || text.startsWith(t))) {
                return true;
            }

            return false;
        },

        isInteractable: (el) => {
            const rect = el.getBoundingClientRect();
            const centerX = rect.left + rect.width / 2;
            const centerY = rect.top + rect.height / 2;

            // Recursively find deepest element at point, penetrating shadow boundaries
            const getDeepestElementAt = (x, y, root = document) => {
                const element = root.elementFromPoint(x, y);
                if (!element) return null;
                if (!element.shadowRoot) return element;
                return getDeepestElementAt(x, y, element.shadowRoot) || element;
            };

            const topElement = getDeepestElementAt(centerX, centerY, el.ownerDocument);
            if (!topElement) return false;

            // Interactable if element is at top, contains top, or is contained by top
            return el === topElement || el.contains(topElement) || topElement.contains(el);
        },

        getTextRects: (searchQuery) => {
            const results = [];
            if (!searchQuery) return results;
            const query = searchQuery.toLowerCase();

            const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT, {
                acceptNode: (node) => {
                    if (!Utils.isVisible(node.parentElement)) return NodeFilter.FILTER_REJECT;
                    if (node.textContent.toLowerCase().includes(query)) return NodeFilter.FILTER_ACCEPT;
                    return NodeFilter.FILTER_REJECT;
                }
            });

            while (walker.nextNode()) {
                const node = walker.currentNode;
                const range = document.createRange();
                range.selectNodeContents(node);
                const rects = range.getClientRects();
                for (const rect of rects) {
                    results.push(rect);
                }
            }
            return results;
        },

        isNear: (elRect, targetRects, threshold = 50) => {
            if (!targetRects || targetRects.length === 0) return false;
            for (const tr of targetRects) {
                const intersects =
                    elRect.left < tr.right + threshold &&
                    elRect.right > tr.left - threshold &&
                    elRect.top < tr.bottom + threshold &&
                    elRect.bottom > tr.top - threshold;
                if (intersects) return true;
            }
            return false;
        },

        getElementText: (el) => {
            const text = el.innerText || el.textContent || el.value || el.getAttribute('placeholder') || el.getAttribute('aria-label') || '';
            return text.trim().substring(0, 100);
        },

        getElementState: (el) => {
            const isVisible = Utils.isVisible(el);
            const tag = el.tagName.toLowerCase();

            const state = {
                visible: isVisible,
                hidden: !isVisible,
                disabled: !!el.disabled,
                focused: document.activeElement === el,
                primary: Utils.isPrimaryButton(el)
            };

            if (el.type === 'checkbox' || el.type === 'radio') {
                state.checked = !!el.checked;
                state.unchecked = !el.checked;
            }

            if (tag === 'input' || tag === 'textarea' || tag === 'select') {
                state.required = !!el.required;
                state.readonly = !!el.readOnly;
                state.value = el.value || '';
            }

            return state;
        },

        getDataAttributes: (el) => {
            const attrs = {};
            for (const name of el.getAttributeNames()) {
                if (name.startsWith('data-')) attrs[name] = el.getAttribute(name);
            }
            return attrs;
        },

        getElementAttributes: (el, dataAttrs) => {
            const ATTR_LIST = ['href', 'src', 'placeholder', 'name', 'autocomplete', 'aria-label', 'aria-labelledby', 'aria-hidden', 'aria-disabled', 'aria-describedby', 'for', 'title', 'tabindex'];
            const attrs = { ...dataAttrs };

            for (const attr of ATTR_LIST) {
                const val = el.getAttribute(attr);
                if (val) attrs[attr] = val;
            }

            if (el.id) attrs.id = el.id;
            if (el.className) attrs.class = el.className;

            return attrs;
        }
    };

    // --- Shadow DOM Utils ---

    const ShadowUtils = {
        /**
         * Collect elements from a root, including those inside open shadow roots.
         * @param {Element|ShadowRoot} root - The root to start from
         * @param {function} filter - Function to test if element should be included
         * @param {Array} results - Array to collect results into
         * @param {number} maxElements - Maximum elements to collect
         * @returns {Array} - The results array
         */
        collectElements: (root, filter, results = [], maxElements = 200) => {
            if (results.length >= maxElements) return results;

            const walker = document.createTreeWalker(root, NodeFilter.SHOW_ELEMENT, {
                acceptNode: (node) => {
                    if (['SCRIPT', 'STYLE', 'NOSCRIPT', 'OBJECT'].includes(node.tagName)) {
                        return NodeFilter.FILTER_REJECT;
                    }
                    // ACCEPT all elements so we can check shadow roots
                    // (we'll filter for results separately)
                    return NodeFilter.FILTER_ACCEPT;
                }
            });

            // Helper to process each element:
            // 1. Add to results only if filter passes AND under limit
            // 2. ALWAYS recurse into shadow roots (even if element didn't pass filter)
            const processElement = (el) => {
                // Only add to results if passes filter and under limit
                if (filter(el) && results.length < maxElements) {
                    results.push(el);
                }
                // ALWAYS check for shadow root, even if element didn't pass filter
                // This ensures shadow hosts like <div id="shadow-host"> are explored
                if (el.shadowRoot) {
                    ShadowUtils.collectElements(el.shadowRoot, filter, results, maxElements);
                }
            };

            let node = walker.currentNode;
            // Process root if it's an element (not document/shadowRoot)
            if (node.nodeType === Node.ELEMENT_NODE && node.tagName) {
                processElement(node);
            }

            // Continue walking - don't stop early just because results.length >= maxElements
            // We need to visit ALL elements to find shadow roots
            while (walker.nextNode()) {
                if (results.length >= maxElements) {
                    // We have enough results, but still check this element for shadow roots
                    const el = walker.currentNode;
                    if (el.shadowRoot) {
                        ShadowUtils.collectElements(el.shadowRoot, filter, results, maxElements);
                    }
                } else {
                    processElement(walker.currentNode);
                }
            }

            return results;
        },

        /**
         * Search for text in both regular DOM and shadow DOM.
         * @param {string} searchQuery - Text to search for
         * @param {Element|ShadowRoot} root - Root to start from
         * @returns {Array} - Array of DOMRect objects
         */
        getTextRectsWithShadow: (searchQuery, root = document.body) => {
            const results = [];
            if (!searchQuery) return results;
            const query = searchQuery.toLowerCase();

            const searchInRoot = (searchRoot) => {
                const walker = document.createTreeWalker(searchRoot, NodeFilter.SHOW_TEXT, {
                    acceptNode: (node) => {
                        if (!Utils.isVisible(node.parentElement)) return NodeFilter.FILTER_REJECT;
                        if (node.textContent.toLowerCase().includes(query)) return NodeFilter.FILTER_ACCEPT;
                        return NodeFilter.FILTER_REJECT;
                    }
                });

                while (walker.nextNode()) {
                    const node = walker.currentNode;
                    const range = document.createRange();
                    range.selectNodeContents(node);
                    const rects = range.getClientRects();
                    for (const rect of rects) {
                        results.push(rect);
                    }
                }

                // Also search in shadow roots
                const elementWalker = document.createTreeWalker(searchRoot, NodeFilter.SHOW_ELEMENT, null);
                while (elementWalker.nextNode()) {
                    const el = elementWalker.currentNode;
                    if (el.shadowRoot) {
                        searchInRoot(el.shadowRoot);
                    }
                }
            };

            searchInRoot(root);
            return results;
        },

        /**
         * Find the first element matching a selector, including inside open shadow roots.
         * @param {Element|ShadowRoot|Document} root - The root to start from
         * @param {string} selector - CSS selector to match
         * @returns {Element|null} - First matching element or null
         */
        querySelectorWithShadow: (root, selector) => {
            // Try to find in the current root first
            const directMatch = root.querySelector(selector);
            if (directMatch) return directMatch;

            // Search in shadow roots
            const walker = document.createTreeWalker(root, NodeFilter.SHOW_ELEMENT, null);
            while (walker.nextNode()) {
                const el = walker.currentNode;
                if (el.shadowRoot) {
                    const shadowMatch = ShadowUtils.querySelectorWithShadow(el.shadowRoot, selector);
                    if (shadowMatch) return shadowMatch;
                }
            }

            return null;
        },

        /**
         * Find all elements matching a selector, including inside open shadow roots.
         * @param {Element|ShadowRoot|Document} root - The root to start from
         * @param {string} selector - CSS selector to match
         * @returns {Array<Element>} - Array of matching elements
         */
        querySelectorAllWithShadow: (root, selector) => {
            const results = [];

            // Get all matches from current root
            const directMatches = root.querySelectorAll(selector);
            results.push(...directMatches);

            // Search in shadow roots recursively
            const walker = document.createTreeWalker(root, NodeFilter.SHOW_ELEMENT, null);
            while (walker.nextNode()) {
                const el = walker.currentNode;
                if (el.shadowRoot) {
                    const shadowMatches = ShadowUtils.querySelectorAllWithShadow(el.shadowRoot, selector);
                    results.push(...shadowMatches);
                }
            }

            return results;
        },

        /**
         * Find a text node containing the search text, including inside open shadow roots.
         * @param {Element|ShadowRoot} root - The root to start from
         * @param {string} searchText - Text to search for (case-insensitive)
         * @returns {Element|null} - Parent element containing the text, or null
         */
        findTextNodeWithShadow: (root, searchText) => {
            const normalizedSearch = searchText.toLowerCase().trim();

            // Search text nodes in current root
            const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT, null);
            while (walker.nextNode()) {
                const node = walker.currentNode;
                if (node.textContent.toLowerCase().includes(normalizedSearch)) {
                    const parent = node.parentElement;
                    if (parent && Utils.isVisible(parent)) {
                        return parent;
                    }
                }
            }

            // Search in shadow roots recursively
            const elementWalker = document.createTreeWalker(root, NodeFilter.SHOW_ELEMENT, null);
            while (elementWalker.nextNode()) {
                const el = elementWalker.currentNode;
                if (el.shadowRoot) {
                    const shadowMatch = ShadowUtils.findTextNodeWithShadow(el.shadowRoot, searchText);
                    if (shadowMatch) return shadowMatch;
                }
            }

            return null;
        }
    };

    // --- Modules ---

    const Scanner = {
        scan: (params) => {
            const t0 = performance.now();
            const maxElements = params.max_elements || 200;
            const includeHidden = params.include_hidden || false;
            const includeIframes = params.include_iframes !== false; // Default true
            const contextNode = params.within ? ShadowUtils.querySelectorWithShadow(document.body, params.within) : document.body;

            if (!contextNode) return Protocol.error('Container not found', 'SELECTOR_INVALID');
            const monitorChanges = params.monitor_changes === true;

            const elements = [];
            const iframes = [];
            const seenIds = new Set();
            const changes = [];

            // 1. Reset check - if first scan or forced clean, we could reset.
            // But usually we want persistent IDs.
            // We will prune STATE.elementMap at the end.

            // 2. Discover main document elements (including Shadow DOM)
            // Filter function for referenceable elements
            const elementFilter = (node) => {
                // Collect iframes for separate processing
                if (node.tagName === 'IFRAME') {
                    iframes.push(node);
                    return false;
                }
                // Check if interactive-ish
                return Scanner.isReferenceable(node);
            };

            // Use ShadowUtils to collect elements including those in shadow DOM
            const candidateElements = ShadowUtils.collectElements(contextNode, elementFilter, [], maxElements * 2);

            // Pre-calculate text rects if near is requested (with shadow DOM support)
            let nearRects = null;
            if (params.near) {
                nearRects = ShadowUtils.getTextRectsWithShadow(params.near, contextNode);
                // If near text not found, maybe return warning or empty?
                // Spec implies filtering, so if nothing found, nothing returned usually.
            }

            // Process candidate elements
            for (const el of candidateElements) {
                if (elements.length >= maxElements) break;

                if (!includeHidden && !Utils.isVisible(el)) continue;
                if (params.viewport_only && !Utils.isInViewport(el)) continue;

                // Near Check
                if (nearRects) {
                    const elRect = el.getBoundingClientRect();
                    if (!Utils.isNear(elRect, nearRects)) continue;
                }

                // Assign or get persistent ID
                let id = STATE.inverseMap.get(el);
                if (!id) {
                    id = STATE.nextId++;
                    STATE.inverseMap.set(el, id);
                    if (monitorChanges) changes.push({ id, change_type: 'appeared' });
                }
                // Always ensure element is in the active map for execution
                STATE.elementMap.set(id, el);
                seenIds.add(id);

                const serialized = Scanner.serializeElement(el, id);

                if (monitorChanges) {
                    const cached = STATE.cache.get(id);
                    if (cached) {
                        const elementChanges = Scanner.diffElements(cached, serialized);
                        if (elementChanges.length > 0) {
                            changes.push(...elementChanges);
                        }
                    }
                    STATE.cache.set(id, serialized);
                }

                elements.push(serialized);
            }

            // 3. Process iframes
            const iframeInfo = [];
            if (includeIframes) {
                for (const iframe of iframes) {
                    if (elements.length >= maxElements) break;

                    const iframeData = Scanner.processIframe(
                        iframe,
                        includeHidden,
                        params.viewport_only,
                        maxElements - elements.length,
                        monitorChanges,
                        changes
                    );

                    // Add iframe element itself
                    const iframeId = STATE.nextId++;
                    STATE.elementMap.set(iframeId, iframe);
                    STATE.inverseMap.set(iframe, iframeId);

                    const iframeElement = Scanner.serializeElement(iframe, iframeId);
                    iframeElement.iframe = {
                        accessible: iframeData.accessible,
                        src: iframe.src || '',
                        origin: iframeData.origin
                    };
                    elements.push(iframeElement);
                    iframeInfo.push(iframeElement.iframe);

                    seenIds.add(iframeId);

                    // Add elements from accessible iframes
                    if (iframeData.accessible && iframeData.elements) {
                        for (const elData of iframeData.elements) {
                            if (elements.length >= maxElements) break;
                            elData.iframe_context = {
                                iframe_id: iframeId,
                                src: iframe.src || ''
                            };
                            elements.push(elData);
                            // IDs for iframe elements are already handled in processIframe
                            seenIds.add(elData.id);
                        }
                    }
                }
            }

            // --- Cleanup Disappeared elements ---
            for (const id of STATE.elementMap.keys()) {
                if (!seenIds.has(id)) {
                    if (monitorChanges) {
                        changes.push({ id, change_type: 'disappeared' });
                        STATE.cache.delete(id);
                    }
                    STATE.elementMap.delete(id);
                }
            }
            // ------------------------------------

            // Detect patterns
            const patterns = Patterns.detectAll(elements);

            const response = {
                page: {
                    url: window.location.href,
                    title: document.title,
                    viewport: { width: window.innerWidth, height: window.innerHeight },
                    scroll: {
                        x: window.scrollX,
                        y: window.scrollY,
                        max_y: document.documentElement.scrollHeight - window.innerHeight
                    },
                    readyState: document.readyState
                },
                settings_applied: {
                    max_elements: maxElements,
                    include_hidden: includeHidden,
                    include_iframes: includeIframes,
                    viewport_only: !!params.viewport_only
                },
                elements: elements,
                stats: {
                    total: elements.length,
                    scanned: elements.length,
                    iframes: {
                        total: iframes.length,
                        accessible: iframeInfo.filter((i) => i.accessible).length,
                        cross_origin: iframeInfo.filter((i) => !i.accessible).length
                    }
                }
            };

            if (patterns) response.patterns = patterns;

            if (monitorChanges) {
                response.changes = changes;
            }

            return Protocol.success(response, t0);
        },

        diffElements: (oldData, newData) => {
            const changes = [];
            const id = newData.id;

            // 1. Text change
            if (oldData.text !== newData.text) {
                changes.push({
                    id,
                    change_type: 'text_changed',
                    old_value: oldData.text,
                    new_value: newData.text
                });
            }

            // 2. State change (checked, disabled, etc.)
            const stateKeys = ['checked', 'disabled', 'selected', 'focused', 'expanded'];
            for (const key of stateKeys) {
                if (oldData.state[key] !== newData.state[key]) {
                    changes.push({
                        id,
                        change_type: 'state_changed',
                        old_value: `${key}:${oldData.state[key]}`,
                        new_value: `${key}:${newData.state[key]}`
                    });
                }
            }

            // 3. Position/Size change (if significant)
            const threshold = 5;
            const r1 = oldData.rect;
            const r2 = newData.rect;
            if (Math.abs(r1.x - r2.x) > threshold || Math.abs(r1.y - r2.y) > threshold) {
                changes.push({ id, change_type: 'position_changed' });
            }

            return changes;
        },

        processIframe: (iframe, includeHidden, viewportOnly, maxElements, monitorChanges, changes) => {
            const result = {
                accessible: false,
                origin: null,
                elements: []
            };

            try {
                // Try to access iframe's contentDocument (will throw for cross-origin)
                const iframeDoc = iframe.contentDocument || iframe.contentWindow?.document;

                if (!iframeDoc) {
                    result.origin = 'cross-origin';
                    return result;
                }

                result.accessible = true;
                result.origin = 'same-origin';

                // Scan iframe content using shadow-aware collection
                const elementFilter = (el) => {
                    // Reject non-interactive script/style elements
                    if (['SCRIPT', 'STYLE', 'NOSCRIPT', 'IFRAME', 'OBJECT'].includes(el.tagName)) {
                        return false;
                    }
                    // Check if element is referenceable
                    if (!Scanner.isReferenceable(el)) return false;
                    // Visibility check within iframe
                    if (!includeHidden && !Utils.isVisible(el)) return false;
                    return true;
                };

                const iframeRoot = iframeDoc.body || iframeDoc.documentElement;
                const collectedElements = ShadowUtils.collectElements(iframeRoot, elementFilter, [], maxElements);

                // Process collected elements
                for (const el of collectedElements) {
                    // Assign or get persistent ID
                    let id = STATE.inverseMap.get(el);
                    if (!id) {
                        id = STATE.nextId++;
                        STATE.inverseMap.set(el, id);
                        STATE.elementMap.set(id, el);
                        if (monitorChanges) changes.push({ id, change_type: 'appeared' });
                    }

                    const serialized = Scanner.serializeElement(el, id);

                    if (monitorChanges) {
                        const cached = STATE.cache.get(id);
                        if (cached) {
                            const elementChanges = Scanner.diffElements(cached, serialized);
                            if (elementChanges.length > 0) {
                                changes.push(...elementChanges);
                            }
                        }
                        STATE.cache.set(id, serialized);
                    }

                    result.elements.push(serialized);
                }
            } catch (_e) {
                // Cross-origin access denied
                result.accessible = false;
                result.origin = 'cross-origin';
            }

            return result;
        },

        isReferenceable: (el) => {
            const tag = el.tagName.toLowerCase();
            const INTERACTIVE_TAGS = new Set(['input', 'select', 'textarea', 'button', 'a', 'img', 'table']);
            const TEXT_ANCHOR_TAGS = new Set(['h1', 'h2', 'h3', 'h4', 'h5', 'h6', 'label', 'strong', 'b', 'em', 'span', 'p', 'li', 'td', 'th']);

            if (INTERACTIVE_TAGS.has(tag)) return true;
            if (el.getAttribute('role')) return true;
            if (el.hasAttribute('onclick') || el.isContentEditable) return true;
            if (window.getComputedStyle(el).cursor === 'pointer') return true;

            if (TEXT_ANCHOR_TAGS.has(tag)) {
                const text = el.textContent?.trim();
                if (text && text.length > 0 && text.length < 100 && el.childElementCount < 3) return true;
            }

            return false;
        },

        serializeElement: (el, id) => {
            const rect = el.getBoundingClientRect();
            const dataAttrs = Utils.getDataAttributes(el);
            const label = Utils.getLabelText(el);

            return {
                id,
                type: el.tagName.toLowerCase(),
                role: Utils.detectRole(el),
                text: Utils.getElementText(el),
                label: label || null,
                selector: Utils.generateSelector(el),
                xpath: Utils.getXPath(el),
                rect: {
                    x: Math.round(rect.x),
                    y: Math.round(rect.y),
                    width: Math.round(rect.width),
                    height: Math.round(rect.height)
                },
                attributes: Utils.getElementAttributes(el, dataAttrs),
                state: Utils.getElementState(el)
            };
        }
    };

    const Executor = {
        getElement: (id) => {
            const el = STATE.elementMap.get(id);
            if (!el) throw { msg: `Element ${id} not found`, code: 'ELEMENT_NOT_FOUND' };
            if (!el.isConnected) throw { msg: `Element ${id} is stale`, code: 'ELEMENT_STALE' };
            return el;
        },
        getElementFromParams: (params) => {
            if (params.id != null) return Executor.getElement(params.id);
            if (params.selector) {
                const el = ShadowUtils.querySelectorWithShadow(document.body, params.selector);
                if (!el) throw { msg: 'Element not found', code: 'ELEMENT_NOT_FOUND' };
                return el;
            }
            throw { msg: 'Missing target', code: 'INVALID_PARAMS' };
        },

        click: (params) => {
            const el = Executor.getElementFromParams(params);

            // Check visibility unless force is set
            if (!params.force && !Utils.isVisible(el)) {
                const rect = el.getBoundingClientRect();
                throw {
                    msg: `Element ${params.id} is not visible`,
                    code: 'ELEMENT_NOT_VISIBLE',
                    details: {
                        rect: {
                            x: Math.round(rect.x),
                            y: Math.round(rect.y),
                            width: Math.round(rect.width),
                            height: Math.round(rect.height)
                        },
                        viewport: {
                            width: window.innerWidth,
                            height: window.innerHeight
                        }
                    }
                };
            }

            if (params.scroll_into_view !== false) el.scrollIntoView({ block: 'center', behavior: 'instant' });

            if (!params.force && !Utils.isInteractable(el)) {
                throw { msg: `Element ${params.id} is covered by another element`, code: 'ELEMENT_NOT_INTERACTABLE' };
            }

            const { x: clientX, y: clientY } = Utils.getClickCoordinates(el, params.offset);

            const buttonType = (params.button || 'left').toLowerCase();
            const BUTTON_MAP = { left: 0, middle: 1, right: 2 };
            const BUTTONS_MAP = { 0: 1, 1: 4, 2: 2 };
            const button = BUTTON_MAP[buttonType] || 0;

            const clickOpts = {
                bubbles: true,
                cancelable: true,
                view: window,
                clientX,
                clientY,
                button,
                buttons: BUTTONS_MAP[button]
            };

            if (params.modifiers) {
                const MODIFIER_MAP = { shift: 'shiftKey', ctrl: 'ctrlKey', control: 'ctrlKey', alt: 'altKey', meta: 'metaKey' };
                for (const mod of params.modifiers) {
                    const key = MODIFIER_MAP[mod.toLowerCase()];
                    if (key) clickOpts[key] = true;
                }
            }

            const clickCount = params.click_count || 1;

            const { navigationDetected, domChanges, cleanup } = Utils.createChangeTracker();

            try {
                for (let i = 0; i < clickCount; i++) {
                    clickOpts.detail = i + 1;
                    el.dispatchEvent(new MouseEvent('mousedown', clickOpts));
                    el.dispatchEvent(new MouseEvent('mouseup', clickOpts));

                    if (button === 0) {
                        el.click();
                    } else if (button === 2) {
                        el.dispatchEvent(new MouseEvent('contextmenu', clickOpts));
                    }

                    if (i === 1) {
                        el.dispatchEvent(new MouseEvent('dblclick', clickOpts));
                    }
                }
            } finally {
                cleanup();
            }

            return Protocol.success({
                action: clickCount > 1 ? 'double_clicked' : 'clicked',
                id: params.id,
                tag: el.tagName.toLowerCase(),
                selector: Utils.generateSelector(el),
                coordinates: { x: Math.round(clientX), y: Math.round(clientY) },
                button: buttonType,
                navigation: navigationDetected,
                dom_changes: domChanges
            });
        },

        type: async (params) => {
            const el = Executor.getElementFromParams(params);

            // Check if element is disabled
            if (el.disabled) {
                throw { msg: `Element ${params.id} is disabled`, code: 'ELEMENT_DISABLED' };
            }

            if (params.scroll_into_view !== false) el.scrollIntoView({ block: 'center', behavior: 'instant' });

            el.focus();

            if (params.clear !== false) {
                if (el.isContentEditable) {
                    el.innerText = '';
                } else {
                    el.value = '';
                }
                el.dispatchEvent(new Event('input', { bubbles: true }));
                el.dispatchEvent(new Event('change', { bubbles: true }));
            }

            const text = params.text || '';
            const delay = params.delay || 0;

            if (delay > 0) {
                // Character-by-character typing with delay
                for (const char of text) {
                    // Dispatch keydown
                    el.dispatchEvent(
                        new KeyboardEvent('keydown', {
                            key: char,
                            code: `Key${char.toUpperCase()}`,
                            bubbles: true,
                            cancelable: true
                        })
                    );

                    // Insert character
                    if (el.isContentEditable) {
                        el.innerText += char;
                    } else {
                        el.value += char;
                    }

                    // Dispatch keypress and input
                    el.dispatchEvent(
                        new KeyboardEvent('keypress', {
                            key: char,
                            bubbles: true,
                            cancelable: true
                        })
                    );
                    el.dispatchEvent(new Event('input', { bubbles: true }));

                    // Dispatch keyup
                    el.dispatchEvent(
                        new KeyboardEvent('keyup', {
                            key: char,
                            code: `Key${char.toUpperCase()}`,
                            bubbles: true,
                            cancelable: true
                        })
                    );

                    // Wait for delay
                    await new Promise((r) => setTimeout(r, delay));
                }
            } else {
                // Fast path: set value directly
                if (el.isContentEditable) {
                    if (params.clear === false) {
                        el.innerText += text;
                    } else {
                        el.innerText = text;
                    }
                } else {
                    if (params.clear === false) {
                        el.value = (el.value || '') + text;
                    } else {
                        el.value = text;
                    }
                }
                el.dispatchEvent(new Event('input', { bubbles: true }));
            }

            el.dispatchEvent(new Event('change', { bubbles: true }));

            // Handle submit-after-type if requested
            if (params.submit) {
                const form = el.form || el.closest('form');
                if (form) {
                    form.requestSubmit?.() ?? form.submit();
                } else {
                    // No form found - dispatch Enter keydown event
                    el.dispatchEvent(
                        new KeyboardEvent('keydown', {
                            key: 'Enter',
                            code: 'Enter',
                            keyCode: 13,
                            which: 13,
                            bubbles: true,
                            cancelable: true
                        })
                    );
                }
            }

            return Protocol.success({
                action: 'typed',
                id: params.id,
                selector: Utils.generateSelector(el),
                text: text,
                value: el.isContentEditable ? el.innerText : el.value,
                submitted: !!params.submit
            });
        },

        clear: (params) => {
            const el = Executor.getElementFromParams(params);
            el.value = '';
            el.dispatchEvent(new Event('input', { bubbles: true }));
            el.dispatchEvent(new Event('change', { bubbles: true }));
            return Protocol.success({
                action: 'cleared',
                id: params.id,
                selector: Utils.generateSelector(el)
            });
        },

        check: (params, targetState) => {
            const el = Executor.getElementFromParams(params);
            const previousState = el.checked;

            if (el.checked !== targetState) {
                el.click(); // Click usually toggles
                // If click didn't work (prevented), force it
                if (el.checked !== targetState) {
                    el.checked = targetState;
                    el.dispatchEvent(new Event('change', { bubbles: true }));
                }
            }

            return Protocol.success({
                action: targetState ? 'checked' : 'unchecked',
                id: params.id,
                selector: Utils.generateSelector(el),
                checked: el.checked,
                previous: previousState
            });
        },

        select: (params) => {
            const el = Executor.getElementFromParams(params);
            if (el.tagName.toLowerCase() !== 'select') {
                throw { msg: 'Not a select element', code: 'INVALID_ELEMENT_TYPE' };
            }

            const previousValue = el.value;
            const previousText = el.options[el.selectedIndex]?.text || '';
            const selectedValues = [];

            const toArray = (val) => (val != null ? (Array.isArray(val) ? val : [val]) : null);
            const lastOnly = (arr) => (arr && arr.length > 1 ? [arr[arr.length - 1]] : arr);

            let values = toArray(params.value);
            let texts = toArray(params.text != null ? params.text : params.label);
            let indexes = toArray(params.index);

            if (!el.multiple) {
                values = lastOnly(values);
                texts = lastOnly(texts);
                indexes = lastOnly(indexes);
            }

            const options = Array.from(el.options);
            let foundAny = false;

            const selectOption = (o, match) => {
                if (match) {
                    o.selected = true;
                    selectedValues.push(o.value);
                    foundAny = true;
                } else if (el.multiple) {
                    o.selected = false;
                }
            };

            if (values) {
                options.forEach((o) => selectOption(o, values.includes(o.value)));
            } else if (texts) {
                options.forEach((o) => {
                    const optText = o.text.trim().toLowerCase();
                    selectOption(o, texts.some((t) => optText.includes(t.trim().toLowerCase())));
                });
            } else if (indexes) {
                options.forEach((o, i) => selectOption(o, indexes.includes(i)));
            }

            if (!foundAny && (values || texts || indexes)) {
                throw { msg: 'Option not found', code: 'OPTION_NOT_FOUND' };
            }

            el.dispatchEvent(new Event('change', { bubbles: true }));
            el.dispatchEvent(new Event('input', { bubbles: true }));

            const response = {
                action: 'selected',
                id: params.id,
                selector: Utils.generateSelector(el),
                value: selectedValues, // Return array
                previous: {
                    value: previousValue,
                    text: previousText
                }
            };

            // Add index field if selecting by index
            if (indexes) {
                response.index = indexes;
            }

            return Protocol.success(response);
        },

        scroll: (params) => {
            const behavior = params.behavior || 'instant';
            let target = window;
            let isWindow = true;

            if (params.element) {
                target = Executor.getElement(params.element);
                isWindow = false;
            } else if (params.container) {
                target = ShadowUtils.querySelectorWithShadow(document.body, params.container);
                if (!target) throw { msg: 'Container not found', code: 'ELEMENT_NOT_FOUND' };
                isWindow = false;
            }

            if (params.direction) {
                const amount = params.amount || 100;
                const DIRECTION_MAP = { up: [0, -1], down: [0, 1], left: [-1, 0], right: [1, 0] };
                const [xDir, yDir] = DIRECTION_MAP[params.direction] || [0, 0];
                target.scrollBy({ left: xDir * amount, top: yDir * amount, behavior });
            } else if (params.element && !isWindow) {
                target.scrollIntoView({ behavior, block: 'center' });
            }

            const scrollX = isWindow ? window.scrollX : target.scrollLeft;
            const scrollY = isWindow ? window.scrollY : target.scrollTop;
            const maxX = isWindow ? document.documentElement.scrollWidth - window.innerWidth : target.scrollWidth - target.clientWidth;
            const maxY = isWindow ? document.documentElement.scrollHeight - window.innerHeight : target.scrollHeight - target.clientHeight;

            return Protocol.success({
                scroll: {
                    x: Math.round(scrollX),
                    y: Math.round(scrollY),
                    max_x: Math.round(Math.max(0, maxX)),
                    max_y: Math.round(Math.max(0, maxY))
                }
            });
        },

        focus: (params) => {
            const el = Executor.getElementFromParams(params);
            el.focus();
            return Protocol.success({
                action: 'focused',
                id: params.id,
                selector: Utils.generateSelector(el)
            });
        },

        hover: (params) => {
            const el = Executor.getElementFromParams(params);

            if (!Utils.isVisible(el)) {
                const rect = el.getBoundingClientRect();
                throw {
                    msg: `Element ${params.id} is not visible`,
                    code: 'ELEMENT_NOT_VISIBLE',
                    details: {
                        rect: {
                            x: Math.round(rect.x),
                            y: Math.round(rect.y),
                            width: Math.round(rect.width),
                            height: Math.round(rect.height)
                        },
                        viewport: {
                            width: window.innerWidth,
                            height: window.innerHeight
                        }
                    }
                };
            }

            const { x: clientX, y: clientY } = Utils.getClickCoordinates(el, params.offset);
            const mouseOpts = { view: window, bubbles: true, cancelable: true, clientX, clientY };

            el.dispatchEvent(new MouseEvent('mouseenter', { ...mouseOpts, bubbles: false }));
            el.dispatchEvent(new MouseEvent('mouseover', mouseOpts));
            el.dispatchEvent(new MouseEvent('mousemove', mouseOpts));

            return Protocol.success({
                action: 'hovered',
                id: params.id,
                selector: Utils.generateSelector(el),
                coordinates: { x: Math.round(clientX), y: Math.round(clientY) }
            });
        },

        submit: (params) => {
            let el;
            if (params.id != null || params.selector) {
                const target = Executor.getElementFromParams(params);
                if (target.tagName === 'FORM') el = target;
                else el = target.form;
            } else {
                // Try to find form of focused element
                if (document.activeElement && document.activeElement.form) {
                    el = document.activeElement.form;
                }
            }

            if (!el) throw { msg: 'No form found to submit', code: 'ELEMENT_NOT_FOUND' };

            // Try clicking submit button first, it's safer for SPA handlers
            const submitBtn = el.querySelector('button[type="submit"], input[type="submit"]');
            if (submitBtn) {
                submitBtn.click();
            } else {
                el.requestSubmit ? el.requestSubmit() : el.submit();
            }

            return Protocol.success({
                action: 'submitted',
                form_selector: Utils.generateSelector(el),
                form_id: el.id || null
            });
        },

        wait_for: async (params) => {
            const timeout = params.timeout ?? params.timeout_ms ?? 30000;
            const pollInterval = params.poll_interval || 100;
            const start = performance.now();
            const initialUrl = window.location.href;

            // Support both 'selector' and 'target' parameter names
            const selector = params.selector || params.target;
            const textToFind = params.text;
            const expression = params.expression;
            const countTarget = params.count;

            // Find element by text content (searches visible text in the document, including shadow DOM)
            const findByText = (text) => {
                return ShadowUtils.findTextNodeWithShadow(document.body, text);
            };

            const getElement = () => {
                if (params.id) return STATE.elementMap.get(params.id);
                if (selector) return ShadowUtils.querySelectorWithShadow(document.body, selector);
                if (textToFind) return findByText(textToFind);
                return null;
            };

            const checkCondition = () => {
                const condition = params.condition;

                switch (condition) {
                    case 'exists': {
                        if (selector) return !!ShadowUtils.querySelectorWithShadow(document.body, selector);
                        if (textToFind) return !!findByText(textToFind);
                        if (params.id)
                            return STATE.elementMap.has(params.id) && STATE.elementMap.get(params.id).isConnected;
                        return false;
                    }
                    case 'visible': {
                        const el = getElement();
                        return el && Utils.isVisible(el);
                    }
                    case 'hidden': {
                        const el = getElement();
                        // Hidden means either doesn't exist or exists but not visible
                        return !el || !Utils.isVisible(el);
                    }
                    case 'gone': {
                        if (selector) return !ShadowUtils.querySelectorWithShadow(document.body, selector);
                        if (textToFind) return !findByText(textToFind);
                        if (params.id) {
                            const el = STATE.elementMap.get(params.id);
                            return !el || !el.isConnected;
                        }
                        return true;
                    }
                    case 'enabled': {
                        const el = getElement();
                        return el && !el.disabled;
                    }
                    case 'disabled': {
                        const el = getElement();
                        return el && el.disabled === true;
                    }
                    case 'navigation': {
                        // Check if URL has changed from initial
                        return window.location.href !== initialUrl;
                    }
                    case 'load': {
                        return document.readyState === 'complete';
                    }
                    case 'idle': {
                        return document.readyState === 'complete';
                    }
                    case 'custom': {
                        if (!expression) {
                            throw { msg: 'Missing expression for custom wait', code: 'INVALID_PARAMS' };
                        }
                        // eslint-disable-next-line no-new-func
                        return !!Function(`return (${expression})`)();
                    }
                    case 'count': {
                        if (!selector || countTarget == null) return false;
                        const count =
                            typeof countTarget === 'number'
                                ? countTarget
                                : parseInt(countTarget, 10);
                        if (Number.isNaN(count)) {
                            throw { msg: 'Invalid count for wait', code: 'INVALID_PARAMS' };
                        }
                        return ShadowUtils.querySelectorAllWithShadow(document.body, selector).length >= count;
                    }
                    default:
                        return false;
                }
            };

            const poll = () => {
                const elapsed = performance.now() - start;
                if (elapsed >= timeout) {
                    // Use NAVIGATION_ERROR for navigation condition timeout
                    if (params.condition === 'navigation') {
                        return Promise.reject({
                            msg: 'Navigation did not occur within timeout',
                            code: 'NAVIGATION_ERROR'
                        });
                    }
                    return Promise.reject({ msg: 'Timeout waiting for condition', code: 'TIMEOUT' });
                }

                if (checkCondition()) {
                    return Promise.resolve(true);
                }

                return new Promise((r) => setTimeout(r, pollInterval)).then(poll);
            };

            try {
                const met = await poll();
                const result = {
                    condition_met: met,
                    waited_ms: Math.round(performance.now() - start)
                };
                // Include URL info for navigation condition
                if (params.condition === 'navigation') {
                    result.previous_url = initialUrl;
                    result.current_url = window.location.href;
                }
                return Protocol.success(result);
            } catch (e) {
                return Protocol.error(e.msg, e.code);
            }
        },

        login: async (params) => {
            const scanRes = Scanner.scan({ max_elements: 500 });
            if (!scanRes.patterns || !scanRes.patterns.login) {
                throw { msg: 'Login form not detected', code: 'PATTERN_NOT_FOUND' };
            }
            const login = scanRes.patterns.login;

            if (login.email && params.username) {
                await Executor.type({ id: login.email, text: params.username });
            } else if (login.username && params.username) {
                await Executor.type({ id: login.username, text: params.username });
            }

            await Executor.type({ id: login.password, text: params.password });

            if (login.submit) {
                Executor.click({ id: login.submit });
            } else {
                const pwEl = STATE.elementMap.get(login.password);
                if (pwEl && pwEl.form) pwEl.form.submit();
            }

            return Protocol.success({ action: 'login_initiated' });
        },

        search: async (params) => {
            const scanRes = Scanner.scan({ max_elements: 500 });
            if (!scanRes.patterns || !scanRes.patterns.search) {
                throw { msg: 'Search box not detected', code: 'PATTERN_NOT_FOUND' };
            }
            const search = scanRes.patterns.search;

            await Executor.type({ id: search.input, text: params.query });

            if (search.submit) {
                Executor.click({ id: search.submit });
            } else {
                const inputEl = STATE.elementMap.get(search.input);
                if (inputEl && inputEl.form) inputEl.form.submit();
                else {
                    // Try pressing Enter
                    const enterEvent = new KeyboardEvent('keydown', { key: 'Enter', bubbles: true });
                    inputEl.dispatchEvent(enterEvent);
                }
            }

            return Protocol.success({ action: 'search_initiated' });
        },

        dismiss: (params) => {
            const target = (params.target || 'popups').toLowerCase();
            const scanRes = Scanner.scan({ max_elements: 500 });

            const CLOSE_BUTTON_TEXTS = [
                'close', 'cancel', 'dismiss', 'ok', 'x', '×',
                'continue', 'confirm', 'got it', 'no thanks'
            ];

            // Helper: Find visible overlays/modals based on visual and semantic characteristics
            const findVisibleOverlays = () => {
                const candidates = [];
                const allElements = ShadowUtils.querySelectorAllWithShadow(document.body, '*');
                const viewportArea = window.innerWidth * window.innerHeight;

                for (const el of allElements) {
                    if (!Utils.isVisible(el)) continue;

                    const style = window.getComputedStyle(el);
                    const rect = el.getBoundingClientRect();
                    const zIndex = parseInt(style.zIndex) || 0;

                    let score = 0;

                    // 1. Positioning (weight: 2) - fixed/absolute positioning is common for overlays
                    if (style.position === 'fixed' || style.position === 'absolute') score += 2;

                    // 2. Z-index (weight: 2) - high z-index indicates overlay
                    if (zIndex > 100) score += 2;

                    // 3. Coverage area (weight: 2) - modals typically cover significant viewport area
                    if ((rect.width * rect.height) / viewportArea > 0.3) score += 2;

                    // 4. Semantic indicators (weight: 3 each - highest priority)
                    const role = el.getAttribute('role');
                    if (role === 'dialog' || role === 'alertdialog') score += 3;
                    if (el.getAttribute('aria-modal') === 'true') score += 3;

                    // 5. Class/ID patterns (weight: 1 - hints only)
                    const classAndId = Utils.getClassName(el) + ' ' + (el.id || '').toLowerCase();
                    if (/modal|popup|dialog|overlay|lightbox|drawer/.test(classAndId)) score += 1;

                    // Threshold: score >= 4 likely indicates a modal/overlay
                    if (score >= 4) {
                        candidates.push({ element: el, score, zIndex });
                    }
                }

                // Sort by z-index (highest first), then by score
                candidates.sort((a, b) => b.zIndex - a.zIndex || b.score - a.score);

                return candidates.map(c => c.element);
            };

            // Find close button within a modal element
            const findCloseButton = (modal) => {
                // 1. Try semantic selectors first (class names and ARIA labels)
                const semanticClose = ShadowUtils.querySelectorWithShadow(modal,
                    '.close, [aria-label*="close" i], [aria-label*="dismiss" i]'
                );
                if (semanticClose) return semanticClose;

                // 2. Search buttons by text content or icon characteristics
                const buttons = ShadowUtils.querySelectorAllWithShadow(modal, 'button, [role="button"]');
                const modalRect = modal.getBoundingClientRect();

                for (const btn of buttons) {
                    const text = btn.textContent.toLowerCase().trim();

                    // Match by close button text
                    if (CLOSE_BUTTON_TEXTS.includes(text)) return btn;

                    // Match by icon (SVG or close symbol)
                    const hasSvg = btn.querySelector('svg') !== null;
                    const hasCloseIcon = /×|✕|✖/.test(btn.textContent);

                    if (hasSvg || hasCloseIcon) {
                        const btnRect = btn.getBoundingClientRect();
                        const isTopRight = btnRect.right > modalRect.right - 100 &&
                            btnRect.top < modalRect.top + 100;
                        if (isTopRight || hasSvg) return btn;
                    }
                }

                return null;
            };

            // Try to dismiss a modal, optionally filtering by text content
            const tryDismissModal = (textFilter = null) => {
                for (const modal of findVisibleOverlays()) {
                    if (textFilter && !modal.textContent.toLowerCase().includes(textFilter)) continue;

                    const closeBtn = findCloseButton(modal);
                    if (closeBtn) {
                        closeBtn.click();
                        return true;
                    }
                }
                return false;
            };

            const MODAL_TARGETS = new Set(['popups', 'popup', 'modals', 'modal']);
            const COOKIE_TARGETS = new Set(['cookie_banners', 'cookies', 'banner', 'banners']);

            let clicked = false;

            if (MODAL_TARGETS.has(target)) {
                // Use characteristics-based detection, with pattern detection fallback
                clicked = tryDismissModal();
                if (!clicked && scanRes.patterns?.modal?.close) {
                    Executor.click({ id: scanRes.patterns.modal.close });
                    clicked = true;
                }
            } else if (COOKIE_TARGETS.has(target)) {
                const banner = scanRes.patterns?.cookie_banner;
                if (banner?.reject) {
                    Executor.click({ id: banner.reject });
                    clicked = true;
                } else if (banner?.accept) {
                    Executor.click({ id: banner.accept });
                    clicked = true;
                }
            } else {
                // Handle arbitrary string targets - find overlays with matching text
                clicked = tryDismissModal(target);
            }

            if (!clicked) throw { msg: `Could not find anything to dismiss for: ${target}`, code: 'NOT_FOUND' };
            return Protocol.success({ action: 'dismissed', target });
        },

        accept: (params) => {
            const target = params.target || 'cookies';
            if (target === 'cookies' || target === 'cookie_banners') {
                const scanRes = Scanner.scan({ max_elements: 500 });
                if (scanRes.patterns?.cookie_banner?.accept) {
                    Executor.click({ id: scanRes.patterns.cookie_banner.accept });
                    return Protocol.success({ action: 'accepted', target });
                }
            }
            throw { msg: `Could not find anything to accept for: ${target}`, code: 'NOT_FOUND' };
        }
    };

    const Extractor = {
        get_text: (params) => {
            const el = params.selector ? ShadowUtils.querySelectorWithShadow(document.body, params.selector) : document.body;
            if (!el) throw { msg: 'Element not found', code: 'ELEMENT_NOT_FOUND' };
            return Protocol.success({ text: el.innerText || el.textContent || '' });
        },

        get_html: (params) => {
            const el = params.selector ? ShadowUtils.querySelectorWithShadow(document.body, params.selector) : document.documentElement;
            if (!el) throw { msg: 'Element not found', code: 'ELEMENT_NOT_FOUND' };
            const html = params.outer !== false ? el.outerHTML : el.innerHTML;
            return Protocol.success({ html: html || '' });
        },

        get_value: (params) => {
            const el = Executor.getElementFromParams(params);

            // Handle different element types
            const tag = el.tagName.toLowerCase();
            const type = (el.getAttribute('type') || '').toLowerCase();

            // Checkbox/radio returns boolean
            if (type === 'checkbox' || type === 'radio') {
                return Protocol.success({ value: el.checked });
            }

            // Multi-select returns array
            if (tag === 'select' && el.multiple) {
                const values = Array.from(el.selectedOptions).map((o) => o.value);
                return Protocol.success({ value: values });
            }

            // Default: return string value
            return Protocol.success({ value: el.value || '' });
        },

        exists: (params) => {
            const el = ShadowUtils.querySelectorWithShadow(document.body, params.selector);
            return Protocol.success({ exists: !!el });
        },

        execute: (params) => {
            try {
                // Dangerous but required by spec: execute arbitrary JS
                // Function constructor is safer than eval for scope isolation
                const func = new Function('args', params.script);
                const result = func(params.args || []);
                return Protocol.success({ result: result });
            } catch (e) {
                return Protocol.error(e.message, 'SCRIPT_ERROR');
            }
        },

        extract: (params) => {
            const source = params.source || 'links';
            const container = params.selector ? ShadowUtils.querySelectorWithShadow(document.body, params.selector) : document.body;
            if (!container) throw { msg: 'Container not found', code: 'ELEMENT_NOT_FOUND' };

            let results = [];
            switch (source) {
                case 'links':
                    results = ShadowUtils.querySelectorAllWithShadow(container, 'a[href]').map((a) => ({
                        text: a.innerText.trim(),
                        url: a.href,
                        id: STATE.inverseMap.get(a)
                    }));
                    break;
                case 'images':
                    results = ShadowUtils.querySelectorAllWithShadow(container, 'img').map((img) => ({
                        alt: img.alt,
                        src: img.src,
                        id: STATE.inverseMap.get(img)
                    }));
                    break;
                case 'tables':
                    results = ShadowUtils.querySelectorAllWithShadow(container, 'table').map((table) => {
                        const rows = Array.from(table.rows).map((row) =>
                            Array.from(row.cells).map((cell) => cell.innerText.trim())
                        );
                        return { rows, id: STATE.inverseMap.get(table) };
                    });
                    break;
                case 'meta':
                    results = Array.from(document.querySelectorAll('meta')).map((m) => ({
                        name: m.name || m.getAttribute('property'),
                        content: m.content
                    }));
                    break;
                case 'css':
                    if (!params.selector) throw { msg: 'Selector required for CSS extraction', code: 'INVALID_PARAMS' };
                    results = ShadowUtils.querySelectorAllWithShadow(document.documentElement, params.selector).map((el) => ({
                        text: el.innerText,
                        html: el.outerHTML,
                        id: STATE.inverseMap.get(el)
                    }));
                    break;
                case 'text':
                    // Extract text content from the container or selected element
                    results = {
                        text: container.innerText || container.textContent || '',
                        selector: params.selector || 'body'
                    };
                    break;
                default:
                    throw { msg: `Unknown extraction source: ${source}`, code: 'INVALID_PARAMS' };
            }
            return Protocol.success({ results });
        }
    };

    // --- Pattern Detection ---

    const Patterns = {
        getElementProps: (el) => ({
            role: el.role,
            type: el.type,
            text: (el.text || '').toLowerCase(),
            placeholder: (el.attributes?.placeholder || '').toLowerCase(),
            name: (el.attributes?.name || '').toLowerCase(),
            ariaLabel: (el.attributes?.['aria-label'] || '').toLowerCase()
        }),

        isButtonRole: (role) => ['button', 'primary', 'submit', 'link'].includes(role),

        detectAll: (elements) => {
            const detectors = [
                ['login', Patterns.detectLogin],
                ['search', Patterns.detectSearch],
                ['pagination', Patterns.detectPagination],
                ['modal', Patterns.detectModal],
                ['cookie_banner', Patterns.detectCookieBanner]
            ];

            const patterns = {};
            for (const [name, detector] of detectors) {
                const result = detector(elements);
                if (result) patterns[name] = result;
            }

            return Object.keys(patterns).length > 0 ? patterns : null;
        },

        detectLogin: (elements) => {
            let emailField = null;
            let usernameField = null;
            let passwordField = null;
            let submitButton = null;
            let rememberCheckbox = null;

            const LOGIN_BUTTON_TEXTS = ['sign in', 'log in', 'login', 'submit'];

            for (const el of elements) {
                const { role, type, text, placeholder, name } = Patterns.getElementProps(el);

                if (role === 'email' || name.includes('email') || placeholder.includes('email')) {
                    emailField = el.id;
                }

                if ((role === 'username' || role === 'input') && !emailField &&
                    (name.includes('user') || name.includes('login') || placeholder.includes('username') || placeholder.includes('user'))) {
                    usernameField = el.id;
                }

                if (role === 'password') passwordField = el.id;

                if ((Patterns.isButtonRole(role) || type === 'input') && LOGIN_BUTTON_TEXTS.some((t) => text.includes(t))) {
                    submitButton = el.id;
                }

                if (role === 'checkbox' && (text.includes('remember') || name.includes('remember'))) {
                    rememberCheckbox = el.id;
                }
            }

            // Valid login form needs at least (email OR username) AND password
            if ((emailField || usernameField) && passwordField) {
                const result = {
                    password: passwordField
                };
                if (emailField) result.email = emailField;
                if (usernameField) result.username = usernameField;
                if (submitButton) result.submit = submitButton;
                if (rememberCheckbox) result.remember = rememberCheckbox;

                // Try to find form container
                const pwEl = STATE.elementMap.get(passwordField);
                let isInForm = false;
                if (pwEl?.form) {
                    result.form = Utils.generateSelector(pwEl.form);
                    isInForm = true;
                }

                // Calculate confidence score based on presence of login form indicators
                // Base: 0.5 (password field required), max bonus: 0.5 from other indicators
                const CONFIDENCE_BASE = 0.5;
                const CONFIDENCE_HAS_IDENTITY_FIELD = 0.2;  // email or username
                const CONFIDENCE_HAS_SUBMIT = 0.15;
                const CONFIDENCE_IN_FORM = 0.15;

                let confidence = CONFIDENCE_BASE;
                if (emailField || usernameField) confidence += CONFIDENCE_HAS_IDENTITY_FIELD;
                if (submitButton) confidence += CONFIDENCE_HAS_SUBMIT;
                if (isInForm) confidence += CONFIDENCE_IN_FORM;
                result.confidence = Math.min(confidence, 1.0);

                return result;
            }
            return null;
        },

        detectSearch: (elements) => {
            let searchInput = null;
            let submitButton = null;

            const SEARCH_NAMES = new Set(['q', 'query']);

            for (const el of elements) {
                const { role, type, text, placeholder, name } = Patterns.getElementProps(el);

                if (role === 'search' || type === 'search' || name.includes('search') || SEARCH_NAMES.has(name) || placeholder.includes('search')) {
                    searchInput = el.id;
                }

                if (Patterns.isButtonRole(role) && (text.includes('search') || text === 'go' || name.includes('search'))) {
                    submitButton = el.id;
                }
            }

            if (!searchInput) return null;

            const result = { input: searchInput };
            if (submitButton) result.submit = submitButton;
            return result;
        },

        detectPagination: (elements) => {
            let prevButton = null;
            let nextButton = null;
            const pageNumbers = [];

            const PREV_TEXTS = new Set(['prev', 'previous', '‹', '«']);
            const NEXT_TEXTS = new Set(['next', '›', '»']);

            for (const el of elements) {
                const { role, ariaLabel } = Patterns.getElementProps(el);
                const text = (el.text || '').toLowerCase().trim();

                if (!Patterns.isButtonRole(role)) continue;

                if (PREV_TEXTS.has(text) || ariaLabel.includes('previous') || ariaLabel.includes('prev')) {
                    prevButton = el.id;
                } else if (NEXT_TEXTS.has(text) || ariaLabel.includes('next')) {
                    nextButton = el.id;
                } else if (/^\d{1,3}$/.test(text)) {
                    pageNumbers.push({ page: parseInt(text), id: el.id });
                }
            }

            if (!prevButton && !nextButton && pageNumbers.length <= 1) return null;

            const result = {};
            if (prevButton) result.prev = prevButton;
            if (nextButton) result.next = nextButton;
            if (pageNumbers.length > 0) result.pages = pageNumbers.sort((a, b) => a.page - b.page);
            return result;
        },

        detectModal: () => {
            const MODAL_SELECTORS = [
                '[role="dialog"]', '[aria-modal="true"]', '.modal:not(.hidden)',
                '.modal.show', '.modal.open', '[class*="modal"][class*="open"]',
                '[class*="modal"][class*="show"]', '[class*="dialog"][class*="open"]'
            ];
            const CLOSE_SELECTORS = [
                '[aria-label*="close"]', '[aria-label*="Close"]', '.close',
                '.modal-close', '[class*="close"]', 'button:has(svg)'
            ];
            const TITLE_SELECTORS = ['.modal-title', '[class*="title"]', 'h1', 'h2', 'h3'];

            const findFirst = (root, selectors, predicate = () => true) => {
                for (const sel of selectors) {
                    try {
                        const el = ShadowUtils.querySelectorWithShadow(root, sel);
                        if (el && predicate(el)) return el;
                    } catch (_e) { /* ignore */ }
                }
                return null;
            };

            const modal = findFirst(document.body, MODAL_SELECTORS, Utils.isVisible);
            if (!modal) return null;

            const result = { container: Utils.generateSelector(modal) };

            const closeBtn = findFirst(modal, CLOSE_SELECTORS);
            if (closeBtn) {
                const closeId = STATE.inverseMap.get(closeBtn);
                if (closeId) result.close = closeId;
            }

            const titleEl = findFirst(modal, TITLE_SELECTORS, (el) => el.textContent.trim());
            if (titleEl) result.title = titleEl.textContent.trim().substring(0, 100);

            return result;
        },

        detectCookieBanner: () => {
            const BANNER_SELECTORS = [
                '[class*="cookie"]', '[class*="consent"]', '[class*="gdpr"]',
                '[id*="cookie"]', '[id*="consent"]', '[id*="gdpr"]',
                '[aria-label*="cookie"]', '[aria-label*="consent"]'
            ];
            const ACCEPT_PATTERNS = ['accept', 'agree', 'allow', 'ok', 'got it', 'i understand'];
            const REJECT_PATTERNS = ['reject', 'decline', 'deny', 'refuse', 'no thanks'];

            for (const selector of BANNER_SELECTORS) {
                try {
                    const banners = ShadowUtils.querySelectorAllWithShadow(document.body, selector);
                    for (const banner of banners) {
                        if (!Utils.isVisible(banner)) continue;

                        let acceptBtn = null;
                        let rejectBtn = null;

                        const buttons = ShadowUtils.querySelectorAllWithShadow(banner, 'button, a[role="button"], [class*="btn"]');
                        for (const btn of buttons) {
                            const btnText = (btn.textContent || '').toLowerCase().trim();
                            if (!acceptBtn && ACCEPT_PATTERNS.some((p) => btnText.includes(p))) acceptBtn = STATE.inverseMap.get(btn);
                            if (!rejectBtn && REJECT_PATTERNS.some((p) => btnText.includes(p))) rejectBtn = STATE.inverseMap.get(btn);
                        }

                        if (acceptBtn || rejectBtn) {
                            const result = { container: Utils.generateSelector(banner) };
                            if (acceptBtn) result.accept = acceptBtn;
                            if (rejectBtn) result.reject = rejectBtn;
                            return result;
                        }
                    }
                } catch (_e) { /* ignore */ }
            }
            return null;
        }
    };

    const System = {
        version: () => {
            return Protocol.success({
                protocol: '1.0',
                scanner: '1.0.0',
                features: [
                    'scan',
                    'click',
                    'type',
                    'clear',
                    'check',
                    'uncheck',
                    'select',
                    'scroll',
                    'focus',
                    'hover',
                    'submit',
                    'wait_for',
                    'get_text',
                    'get_html',
                    'get_value',
                    'exists',
                    'execute',
                    'version'
                ]
            });
        }
    };

    // --- Main Dispatch ---

    async function process(message) {
        const t0 = performance.now();
        try {
            if (typeof message === 'string') message = JSON.parse(message);

            const cmd = message.cmd || message.action;
            if (!cmd) return Protocol.error('Missing command', 'INVALID_REQUEST');

            // Dispatch
            let result;
            switch (cmd) {
                case 'scan':
                    result = Scanner.scan(message);
                    break;

                // Actions
                case 'click':
                    result = Executor.click(message);
                    break;
                case 'type':
                    result = await Executor.type(message);
                    break;
                case 'clear':
                    result = Executor.clear(message);
                    break;
                case 'check':
                    result = Executor.check(message, true);
                    break;
                case 'uncheck':
                    result = Executor.check(message, false);
                    break;
                case 'select':
                    result = Executor.select(message);
                    break;
                case 'scroll':
                    result = Executor.scroll(message);
                    break;
                case 'focus':
                    result = Executor.focus(message);
                    break;
                case 'hover':
                    result = Executor.hover(message);
                    break;
                case 'submit':
                    result = Executor.submit(message);
                    break;
                case 'wait_for':
                    result = await Executor.wait_for(message);
                    break;
                case 'login':
                    result = await Executor.login(message);
                    break;
                case 'search':
                    result = await Executor.search(message);
                    break;
                case 'dismiss':
                    result = Executor.dismiss(message);
                    break;
                case 'accept':
                    result = Executor.accept(message);
                    break;

                // Extraction
                case 'get_text':
                    result = Extractor.get_text(message);
                    break;
                case 'get_html':
                    result = Extractor.get_html(message);
                    break;
                case 'get_value':
                    result = Extractor.get_value(message);
                    break;
                case 'exists':
                    result = Extractor.exists(message);
                    break;
                case 'extract':
                    result = Extractor.extract(message);
                    break;
                case 'execute':
                    result = Extractor.execute(message);
                    break;

                // System
                case 'version':
                    result = System.version();
                    break;

                default:
                    return Protocol.error(`Unknown command: ${cmd}`, 'UNKNOWN_COMMAND');
            }

            // Ensure timing is present on success
            if (result && result.status === 'ok' && !result.timing) {
                result.timing = { duration_ms: performance.now() - t0 };
            }

            if (!result) {
                return Protocol.error('Internal Error: Result is null/undefined', 'INTERNAL_ERROR');
            }

            return result;
        } catch (e) {
            console.error('Scanner error:', e);
            if (e.code) return Protocol.error(e.msg || e.message, e.code);
            return Protocol.error(e.message || 'Internal Error', 'INTERNAL_ERROR');
        }
    }

    // Attach to global
    global.Oryn = global.Oryn || {};
    global.Oryn.process = process;
    global.Oryn.Scanner = Scanner; // Export for debugging
    global.Oryn.State = STATE;
    global.Oryn.ShadowUtils = ShadowUtils; // Export for CSS selector resolution
})(window);
