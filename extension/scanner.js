/**
 * Lemmascope Universal Scanner
 * Version 1.0
 *
 * Implements the scanner protocol for element discovery, interaction, and state extraction.
 * Works across Embedded (lscope-e), Headless (lscope-h), and Remote (lscope-r) backends.
 */
(function (global) {
    // --- State ---

    const STATE = {
        elementMap: new Map(), // ID (number) -> Element
        inverseMap: new Map(), // Element -> ID (number)
        nextId: 1,
        config: {
            debug: false
        }
    };

    // --- State Management ---
    const StateManager = {
        invalidate: () => {
            STATE.elementMap.clear();
            STATE.inverseMap.clear();
            // Keep nextId incrementing or reset?
            // Spec doesn't strictly say, but unique IDs across sessions are safer.
            // But existing logic resets on scan.
            // Clearing map makes old IDs invalid.
            if (STATE.config.debug) console.log('Scanner state invalidated due to navigation');
        },

        init: () => {
            // Navigation listeners
            window.addEventListener('hashchange', StateManager.invalidate);
            window.addEventListener('popstate', StateManager.invalidate);

            // Monkeypatch history for SPA
            const originalPush = window.history.pushState;
            window.history.pushState = function (...args) {
                originalPush.apply(this, args);
                StateManager.invalidate();
            };

            const originalReplace = window.history.replaceState;
            window.history.replaceState = function (...args) {
                originalReplace.apply(this, args);
                StateManager.invalidate();
            };
        }
    };
    StateManager.init();

    // --- Protocol ---

    const Protocol = {
        success: (result = {}, timingStart = null) => {
            // Rust protocol expects { status: "ok", ...result_fields }
            const response = { status: "ok", ...result };
            if (timingStart) {
                response.timing = { duration_ms: performance.now() - timingStart };
            }
            return response;
        },
        error: (msg, code = 'UNKNOWN_ERROR') => {
            // Rust protocol expects { status: "error", message: "...", code: "..." }
            return { status: "error", message: msg, code: code };
        }
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
            // Priority 1: ID
            if (el.id && /^[a-zA-Z][a-zA-Z0-9_-]*$/.test(el.id)) {
                // Only use ID if it looks valid/stable (not random junk)
                // And simple uniqueness check
                if (document.querySelectorAll(`#${CSS.escape(el.id)}`).length === 1) {
                    return `#${CSS.escape(el.id)}`;
                }
            }

            // Priority 2: data-testid
            const testId = el.getAttribute('data-testid');
            if (testId) {
                const selector = `[data-testid="${CSS.escape(testId)}"]`;
                if (document.querySelectorAll(selector).length === 1) return selector;
            }

            // Priority 3: Aria Label (if unique and exists)
            const ariaLabel = el.getAttribute('aria-label');
            if (ariaLabel) {
                const selector = `${el.tagName.toLowerCase()}[aria-label="${CSS.escape(ariaLabel)}"]`;
                if (document.querySelectorAll(selector).length === 1) return selector;
            }

            // Priority 4: Unique Class Combination
            if (el.className && typeof el.className === 'string') {
                const classes = el.className.split(/\s+/).filter((c) => c.trim().length > 0);
                if (classes.length > 0) {
                    // Start with tag + all classes
                    const selector = `${el.tagName.toLowerCase()}.${classes.map((c) => CSS.escape(c)).join('.')}`;
                    if (document.querySelectorAll(selector).length === 1) return selector;
                }
            }

            // Fallback: structural path
            const path = [];
            let current = el;
            while (current && current.nodeType === Node.ELEMENT_NODE) {
                const tag = current.tagName.toLowerCase();
                if (current.id && /^[a-zA-Z][a-zA-Z0-9_-]*$/.test(current.id)) {
                    path.unshift(`#${CSS.escape(current.id)}`);
                    break;
                } else {
                    let sibling = current,
                        nth = 1;
                    while ((sibling = sibling.previousElementSibling)) {
                        if (sibling.tagName.toLowerCase() === tag) nth++;
                    }
                    path.unshift(`${tag}:nth-of-type(${nth})`);
                    current = current.parentNode;
                }
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

        detectRole: (el) => {
            const tag = el.tagName.toLowerCase();
            const type = el.getAttribute('type')?.toLowerCase();
            const role = el.getAttribute('role');
            const autocomplete = el.getAttribute('autocomplete')?.toLowerCase() || '';
            const name = (el.getAttribute('name') || '').toLowerCase();
            const placeholder = (el.getAttribute('placeholder') || '').toLowerCase();
            const ariaLabel = (el.getAttribute('aria-label') || '').toLowerCase();
            const labelText = Utils.getLabelText(el).toLowerCase();

            if (tag === 'input') {
                // Submit button detection (distinct from generic button)
                if (type === 'submit') return 'submit';
                if (['button', 'image', 'reset'].includes(type)) return 'button';
                if (['checkbox', 'radio'].includes(type)) return type;

                // Search detection
                if (
                    type === 'search' ||
                    autocomplete === 'search' ||
                    name.includes('search') ||
                    name === 'q' ||
                    name === 'query' ||
                    placeholder.includes('search') ||
                    labelText.includes('search') ||
                    ariaLabel.includes('search')
                ) {
                    return 'search';
                }

                // Email detection
                if (
                    type === 'email' ||
                    autocomplete === 'email' ||
                    name.includes('email') ||
                    placeholder.includes('email') ||
                    labelText.includes('email')
                ) {
                    return 'email';
                }

                // Username detection (check before generic input)
                if (
                    autocomplete === 'username' ||
                    autocomplete === 'nickname' ||
                    name === 'username' ||
                    name === 'user' ||
                    name === 'login' ||
                    placeholder.includes('username') ||
                    labelText.includes('username')
                ) {
                    return 'username';
                }

                // Password detection
                if (type === 'password' || autocomplete.includes('password')) {
                    return 'password';
                }

                // Tel detection
                if (
                    type === 'tel' ||
                    autocomplete === 'tel' ||
                    name.includes('phone') ||
                    placeholder.includes('phone') ||
                    labelText.includes('phone')
                ) {
                    return 'tel';
                }

                // URL detection
                if (
                    type === 'url' ||
                    autocomplete === 'url' ||
                    name.includes('website') ||
                    placeholder.includes('website') ||
                    labelText.includes('website')
                ) {
                    return 'url';
                }

                return 'input';
            }

            if (tag === 'textarea') return 'textarea';
            if (tag === 'select') return 'select';

            // Button with submit behavior detection
            if (tag === 'button') {
                const btnType = el.getAttribute('type');
                if (btnType === 'submit') return 'submit';

                // Check if it's a primary/prominent button
                if (Utils.isPrimaryButton(el)) return 'primary';

                return 'button';
            }

            if (tag === 'a' && el.hasAttribute('href')) return 'link';

            if (role === 'button') {
                if (Utils.isPrimaryButton(el)) return 'primary';
                return 'button';
            }
            if (role === 'checkbox') return 'checkbox';
            if (role === 'link') return 'link';

            return 'generic';
        },

        getLabelText: (el) => {
            // Try to find associated label
            if (el.id) {
                const label = document.querySelector(`label[for="${el.id}"]`);
                if (label) return label.textContent || '';
            }
            // Check for parent label
            const parentLabel = el.closest('label');
            if (parentLabel) return parentLabel.textContent || '';
            // Check aria-labelledby
            const labelledBy = el.getAttribute('aria-labelledby');
            if (labelledBy) {
                const labelEl = document.getElementById(labelledBy);
                if (labelEl) return labelEl.textContent || '';
            }
            return '';
        },

        isPrimaryButton: (el) => {
            const className = (el.className || '').toLowerCase();
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
            // Check if element is covered by another element at its center point
            const rect = el.getBoundingClientRect();
            const centerX = rect.left + rect.width / 2;
            const centerY = rect.top + rect.height / 2;

            // Use element's own document for elementFromPoint (supports iframe elements)
            const doc = el.ownerDocument;
            const topElement = doc.elementFromPoint(centerX, centerY);

            // Element is interactable if it's the top element or contains the top element
            if (!topElement) return false;
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
            // Check if elRect intersects or is close to any targetRect
            for (const tr of targetRects) {
                // Expansion needed? Simplified interaction/proximity check
                // Check intersection first
                const intersects =
                    elRect.left < tr.right + threshold &&
                    elRect.right > tr.left - threshold &&
                    elRect.top < tr.bottom + threshold &&
                    elRect.bottom > tr.top - threshold;

                if (intersects) return true;
            }
            return false;
        }
    };

    // --- Modules ---

    const Scanner = {
        scan: (params) => {
            const t0 = performance.now();
            const maxElements = params.max_elements || 200;
            const includeHidden = params.include_hidden || false;
            const includeIframes = params.include_iframes !== false; // Default true
            const contextNode = params.within ? document.querySelector(params.within) : document.body;

            if (!contextNode) return Protocol.error('Container not found', 'SELECTOR_INVALID');

            // 1. Reset Map
            STATE.elementMap.clear();
            STATE.inverseMap.clear();
            STATE.nextId = 1;

            const elements = [];
            const iframes = [];

            // 2. Discover main document elements
            const treeWalker = document.createTreeWalker(contextNode, NodeFilter.SHOW_ELEMENT, {
                acceptNode: function (node) {
                    if (['SCRIPT', 'STYLE', 'NOSCRIPT', 'OBJECT'].includes(node.tagName)) {
                        return NodeFilter.FILTER_REJECT;
                    }
                    // Collect iframes for separate processing
                    if (node.tagName === 'IFRAME') {
                        iframes.push(node);
                        return NodeFilter.FILTER_REJECT;
                    }
                    // Check if interactive-ish
                    const isInteractive = Scanner.isReferenceable(node);
                    return isInteractive ? NodeFilter.FILTER_ACCEPT : NodeFilter.FILTER_SKIP;
                }
            });

            // Pre-calculate text rects if near is requested
            let nearRects = null;
            if (params.near) {
                nearRects = Utils.getTextRects(params.near);
                // If near text not found, maybe return warning or empty?
                // Spec implies filtering, so if nothing found, nothing returned usually.
            }

            while (treeWalker.nextNode() && elements.length < maxElements) {
                const el = treeWalker.currentNode;
                if (!includeHidden && !Utils.isVisible(el)) continue;
                if (params.viewport_only && !Utils.isInViewport(el)) continue;

                // Near Check
                if (nearRects) {
                    const elRect = el.getBoundingClientRect();
                    if (!Utils.isNear(elRect, nearRects)) continue;
                }

                // Assign ID
                const id = STATE.nextId++;
                STATE.elementMap.set(id, el);
                STATE.inverseMap.set(el, id);

                elements.push(Scanner.serializeElement(el, id));
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
                        maxElements - elements.length
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

                    // Add elements from accessible iframes
                    if (iframeData.accessible && iframeData.elements) {
                        for (const el of iframeData.elements) {
                            if (elements.length >= maxElements) break;
                            el.iframe_context = {
                                iframe_id: iframeId,
                                src: iframe.src || ''
                            };
                            elements.push(el);
                        }
                    }
                }
            }

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

            if (patterns) {
                response.patterns = patterns;
            }

            return Protocol.success(response, t0);
        },

        processIframe: (iframe, includeHidden, viewportOnly, maxElements) => {
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

                // Scan iframe content
                const iframeWalker = iframeDoc.createTreeWalker(
                    iframeDoc.body || iframeDoc.documentElement,
                    NodeFilter.SHOW_ELEMENT,
                    {
                        acceptNode: function (node) {
                            if (['SCRIPT', 'STYLE', 'NOSCRIPT', 'IFRAME', 'OBJECT'].includes(node.tagName)) {
                                return NodeFilter.FILTER_REJECT;
                            }
                            const isInteractive = Scanner.isReferenceable(node);
                            return isInteractive ? NodeFilter.FILTER_ACCEPT : NodeFilter.FILTER_SKIP;
                        }
                    }
                );

                while (iframeWalker.nextNode() && result.elements.length < maxElements) {
                    const el = iframeWalker.currentNode;

                    // Visibility check within iframe
                    if (!includeHidden && !Utils.isVisible(el)) continue;

                    // Assign ID and track element
                    const id = STATE.nextId++;
                    STATE.elementMap.set(id, el);
                    STATE.inverseMap.set(el, id);

                    result.elements.push(Scanner.serializeElement(el, id));
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
            // Always accept form controls
            if (['input', 'select', 'textarea', 'button', 'a'].includes(tag)) return true;
            if (el.getAttribute('role')) return true;
            if (el.hasAttribute('onclick') || el.isContentEditable) return true;

            // Check for computed cursor pointer
            const style = window.getComputedStyle(el);
            if (style.cursor === 'pointer') return true;

            return false;
        },

        serializeElement: (el, id) => {
            const rect = el.getBoundingClientRect();
            const role = Utils.detectRole(el);

            // Extract meaningful text
            let text =
                el.innerText ||
                el.textContent ||
                el.value ||
                el.getAttribute('placeholder') ||
                el.getAttribute('aria-label') ||
                '';
            text = text.trim().substring(0, 100); // Truncate

            // Build state object with all modifiers
            const isVisible = Utils.isVisible(el);
            const state = {
                visible: isVisible,
                hidden: !isVisible,
                disabled: !!el.disabled,
                focused: document.activeElement === el,
                primary: Utils.isPrimaryButton(el)
            };

            // Checkbox/radio specific state
            if (el.type === 'checkbox' || el.type === 'radio') {
                state.checked = !!el.checked;
                state.unchecked = !el.checked;
            }

            // Input-specific modifiers
            if (
                el.tagName.toLowerCase() === 'input' ||
                el.tagName.toLowerCase() === 'textarea' ||
                el.tagName.toLowerCase() === 'select'
            ) {
                state.required = !!el.required;
                state.readonly = !!el.readOnly;
                state.value = el.value || '';
            }

            // Get associated label text for form elements
            const label = Utils.getLabelText(el);

            return {
                id: id,
                type: el.tagName.toLowerCase(),
                role: role,
                text: text,
                label: label || null,
                selector: Utils.generateSelector(el),
                xpath: Utils.getXPath(el),
                rect: {
                    x: Math.round(rect.x),
                    y: Math.round(rect.y),
                    width: Math.round(rect.width),
                    height: Math.round(rect.height)
                },
                attributes: {
                    href: el.getAttribute('href'),
                    src: el.getAttribute('src'),
                    placeholder: el.getAttribute('placeholder'),
                    name: el.getAttribute('name'),
                    id: el.id,
                    autocomplete: el.getAttribute('autocomplete'),
                    'aria-label': el.getAttribute('aria-label'),
                    'aria-hidden': el.getAttribute('aria-hidden'),
                    'aria-disabled': el.getAttribute('aria-disabled'),
                    title: el.getAttribute('title'),
                    class: el.className,
                    tabindex: el.getAttribute('tabindex'),
                    'data-testid': el.getAttribute('data-testid')
                },
                state: state
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

        click: (params) => {
            const el = Executor.getElement(params.id);

            // Check visibility unless force is set
            if (!params.force && !Utils.isVisible(el)) {
                throw { msg: `Element ${params.id} is not visible`, code: 'ELEMENT_NOT_VISIBLE' };
            }

            if (params.scroll_into_view !== false) el.scrollIntoView({ block: 'center', behavior: 'instant' });

            // Check if element is interactable (not covered) unless force is set
            if (!params.force && !Utils.isInteractable(el)) {
                throw { msg: `Element ${params.id} is covered by another element`, code: 'ELEMENT_NOT_INTERACTABLE' };
            }

            // Calculate click coordinates
            const rect = el.getBoundingClientRect();
            let clientX, clientY;

            if (params.offset) {
                // Offset from top-left of element
                clientX = rect.left + (params.offset.x || 0);
                clientY = rect.top + (params.offset.y || 0);
            } else {
                // Default to center
                clientX = rect.left + rect.width / 2;
                clientY = rect.top + rect.height / 2;
            }

            // Determine button type (0=left, 1=middle, 2=right)
            let button = 0;
            const buttonType = (params.button || 'left').toLowerCase();
            if (buttonType === 'middle') button = 1;
            else if (buttonType === 'right') button = 2;

            const clickOpts = {
                bubbles: true,
                cancelable: true,
                view: window,
                clientX: clientX,
                clientY: clientY,
                button: button,
                buttons: button === 0 ? 1 : button === 1 ? 4 : 2
            };

            // Add modifier keys
            if (params.modifiers) {
                params.modifiers.forEach((mod) => {
                    const m = mod.toLowerCase();
                    if (m === 'shift') clickOpts.shiftKey = true;
                    if (m === 'ctrl' || m === 'control') clickOpts.ctrlKey = true;
                    if (m === 'alt') clickOpts.altKey = true;
                    if (m === 'meta') clickOpts.metaKey = true;
                });
            }

            // Number of clicks (for double-click support)
            const clickCount = params.click_count || 1;

            // Setup navigation detection
            let navigationDetected = false;
            const initialUrl = window.location.href;
            const checkNavigation = () => {
                navigationDetected = true;
            };
            window.addEventListener('beforeunload', checkNavigation);

            // Watch for DOM mutations
            const domChanges = { added: 0, removed: 0, attributes: 0 };
            const observer = new window.MutationObserver((mutations) => {
                mutations.forEach((m) => {
                    if (m.type === 'childList') {
                        domChanges.added += m.addedNodes.length;
                        domChanges.removed += m.removedNodes.length;
                    } else if (m.type === 'attributes') {
                        domChanges.attributes++;
                    }
                });
            });
            observer.observe(document.body, { childList: true, subtree: true, attributes: true });

            try {
                // Perform click sequence...
                for (let i = 0; i < clickCount; i++) {
                    clickOpts.detail = i + 1; // Click count in event

                    el.dispatchEvent(new MouseEvent('mousedown', clickOpts));
                    el.dispatchEvent(new MouseEvent('mouseup', clickOpts));

                    if (button === 0) {
                        el.click(); // Native click for left button
                    } else if (button === 2) {
                        el.dispatchEvent(new MouseEvent('contextmenu', clickOpts));
                    }

                    // For double-click, also fire dblclick event
                    if (i === 1) {
                        el.dispatchEvent(new MouseEvent('dblclick', clickOpts));
                    }
                }
            } finally {
                window.removeEventListener('beforeunload', checkNavigation);
                // Allow a small tick for mutations to register?
                // Synchronous events should trigger mutations immediately but observer callback is async (microtask).
                // We'll need to wait momentarily? Or just snapshot what we have?
                // Spec usually implies async effects might not be caught instantly.
                // But for valid return, we might need a tiny sleep.
            }

            // Force observer flush (takeRecords returns queue)
            const queuedMutations = observer.takeRecords();
            queuedMutations.forEach((m) => {
                if (m.type === 'childList') {
                    domChanges.added += m.addedNodes.length;
                    domChanges.removed += m.removedNodes.length;
                } else if (m.type === 'attributes') {
                    domChanges.attributes++;
                }
            });
            observer.disconnect();

            // Check URL change (SPA nav)
            if (window.location.href !== initialUrl) {
                navigationDetected = true;
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
            const el = Executor.getElement(params.id);

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

            return Protocol.success({
                action: 'typed',
                id: params.id,
                selector: Utils.generateSelector(el),
                text: text,
                value: el.isContentEditable ? el.innerText : el.value
            });
        },

        clear: (params) => {
            const el = Executor.getElement(params.id);
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
            const el = Executor.getElement(params.id);
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
            const el = Executor.getElement(params.id);
            if (el.tagName.toLowerCase() !== 'select')
                throw { msg: 'Not a select element', code: 'INVALID_ELEMENT_TYPE' };

            // Capture previous selection
            const previousValue = el.value;
            const previousText = el.options[el.selectedIndex]?.text || '';

            const selectedValues = [];
            // Note: unselectedValues tracking removed - not needed for current implementation

            // Normalize inputs to arrays
            let values =
                params.value !== undefined ? (Array.isArray(params.value) ? params.value : [params.value]) : null;
            let texts = params.text !== undefined ? (Array.isArray(params.text) ? params.text : [params.text]) : null;
            let indexes =
                params.index !== undefined ? (Array.isArray(params.index) ? params.index : [params.index]) : null;

            if (
                !el.multiple &&
                ((values && values.length > 1) || (texts && texts.length > 1) || (indexes && indexes.length > 1))
            ) {
                // Warn or just select last? Spec says select updates selection.
                // For single select, let's just pick the last one to be safe/standard
                if (values) values = [values[values.length - 1]];
                if (texts) texts = [texts[texts.length - 1]];
                if (indexes) indexes = [indexes[indexes.length - 1]];
            }

            const options = Array.from(el.options);
            let foundAny = false;

            if (values) {
                options.forEach((o) => {
                    if (values.includes(o.value)) {
                        o.selected = true;
                        selectedValues.push(o.value);
                        foundAny = true;
                    } else if (el.multiple) {
                        // In multiple mode, should we Deselect others? Use case implies 'select these'.
                        // But standard automation often deselects everything else.
                        // Let's assume we just ADD selection or SET selection?
                        // "select" command usually implies "set the selection to THIS".
                        o.selected = false;
                    }
                });
            } else if (texts) {
                options.forEach((o) => {
                    if (texts.some((t) => o.text.includes(t))) {
                        o.selected = true;
                        selectedValues.push(o.value);
                        foundAny = true;
                    } else if (el.multiple) {
                        o.selected = false;
                    }
                });
            } else if (indexes) {
                options.forEach((o, i) => {
                    if (indexes.includes(i)) {
                        o.selected = true;
                        selectedValues.push(o.value);
                        foundAny = true;
                    } else if (el.multiple) {
                        o.selected = false;
                    }
                });
            }

            if (!foundAny && (values || texts || indexes)) throw { msg: 'Option not found', code: 'OPTION_NOT_FOUND' };

            el.dispatchEvent(new Event('change', { bubbles: true }));
            el.dispatchEvent(new Event('input', { bubbles: true }));

            return Protocol.success({
                action: 'selected',
                id: params.id,
                selector: Utils.generateSelector(el),
                value: selectedValues, // Return array
                previous: {
                    value: previousValue,
                    text: previousText
                }
            });
        },

        scroll: (params) => {
            const behavior = params.behavior || 'instant';
            let target = window;
            let isWindow = true;

            if (params.element) {
                target = Executor.getElement(params.element);
                isWindow = false;
            } else if (params.container) {
                target = document.querySelector(params.container);
                if (!target) throw { msg: 'Container not found', code: 'ELEMENT_NOT_FOUND' };
                isWindow = false;
            }

            if (params.direction) {
                const amount = params.amount || 100;
                let x = 0,
                    y = 0;
                if (params.direction === 'up') y = -amount;
                if (params.direction === 'down') y = amount;
                if (params.direction === 'left') x = -amount;
                if (params.direction === 'right') x = amount;

                if (isWindow) {
                    target.scrollBy({ left: x, top: y, behavior: behavior });
                } else {
                    target.scrollBy({ left: x, top: y, behavior: behavior });
                }
            } else if (params.element && !isWindow) {
                target.scrollIntoView({ behavior: behavior, block: 'center' });
            }

            // Calculate scroll positions and maximums
            const scrollX = isWindow ? window.scrollX : target.scrollLeft;
            const scrollY = isWindow ? window.scrollY : target.scrollTop;
            const maxX = isWindow
                ? document.documentElement.scrollWidth - window.innerWidth
                : target.scrollWidth - target.clientWidth;
            const maxY = isWindow
                ? document.documentElement.scrollHeight - window.innerHeight
                : target.scrollHeight - target.clientHeight;

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
            const el = Executor.getElement(params.id);
            el.focus();
            return Protocol.success({
                action: 'focused',
                id: params.id,
                selector: Utils.generateSelector(el)
            });
        },

        hover: (params) => {
            const el = Executor.getElement(params.id);

            // Check visibility
            if (!Utils.isVisible(el)) {
                throw { msg: `Element ${params.id} is not visible`, code: 'ELEMENT_NOT_VISIBLE' };
            }

            // Calculate center coordinates
            const rect = el.getBoundingClientRect();
            const clientX = rect.left + rect.width / 2;
            const clientY = rect.top + rect.height / 2;

            const mouseOpts = {
                view: window,
                bubbles: true,
                cancelable: true,
                clientX: clientX,
                clientY: clientY
            };

            // Dispatch full sequence of mouse events for proper hover behavior
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
            if (params.id) {
                const target = Executor.getElement(params.id);
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
            const timeout = params.timeout || 30000;
            const start = performance.now();
            const initialUrl = window.location.href;

            const getElement = () => {
                if (params.id) return STATE.elementMap.get(params.id);
                if (params.selector) return document.querySelector(params.selector);
                return null;
            };

            const checkCondition = () => {
                const condition = params.condition;

                switch (condition) {
                    case 'exists': {
                        if (params.selector) return !!document.querySelector(params.selector);
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
                        if (params.selector) return !document.querySelector(params.selector);
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

                return new Promise((r) => setTimeout(r, 100)).then(poll);
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
        }
    };

    const Extractor = {
        get_text: (params) => {
            const el = document.querySelector(params.selector);
            if (!el) throw { msg: 'Element not found', code: 'ELEMENT_NOT_FOUND' };
            return Protocol.success({ text: el.innerText || el.textContent });
        },

        get_value: (params) => {
            const el = Executor.getElement(params.id);

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
            const el = document.querySelector(params.selector);
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
        }
    };

    // --- Pattern Detection ---

    const Patterns = {
        detectAll: (elements) => {
            const patterns = {};

            const login = Patterns.detectLogin(elements);
            if (login) patterns.login = login;

            const search = Patterns.detectSearch(elements);
            if (search) patterns.search = search;

            const pagination = Patterns.detectPagination(elements);
            if (pagination) patterns.pagination = pagination;

            const modal = Patterns.detectModal();
            if (modal) patterns.modal = modal;

            const cookieBanner = Patterns.detectCookieBanner();
            if (cookieBanner) patterns.cookie_banner = cookieBanner;

            return Object.keys(patterns).length > 0 ? patterns : null;
        },

        detectLogin: (elements) => {
            // Look for email/username field + password field + submit button
            let emailField = null;
            let usernameField = null;
            let passwordField = null;
            let submitButton = null;
            let rememberCheckbox = null;

            for (const el of elements) {
                const role = el.role;
                const type = el.type;
                const text = (el.text || '').toLowerCase();
                const placeholder = (el.attributes?.placeholder || '').toLowerCase();
                const name = (el.attributes?.name || '').toLowerCase();

                // Email field detection
                if (role === 'email' || name.includes('email') || placeholder.includes('email')) {
                    emailField = el.id;
                }

                // Username field detection (input that mentions username/user/login but not email)
                if (
                    role === 'input' &&
                    !emailField &&
                    (name.includes('user') ||
                        name.includes('login') ||
                        placeholder.includes('username') ||
                        placeholder.includes('user'))
                ) {
                    usernameField = el.id;
                }

                // Password field
                if (role === 'password') {
                    passwordField = el.id;
                }

                // Submit button - look for button with sign in/login/submit text
                if (
                    (role === 'button' || type === 'input') &&
                    (text.includes('sign in') ||
                        text.includes('log in') ||
                        text.includes('login') ||
                        text.includes('submit') ||
                        text === 'sign in' ||
                        text === 'log in')
                ) {
                    submitButton = el.id;
                }

                // Remember me checkbox
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
                if (pwEl?.form) {
                    result.form = Utils.generateSelector(pwEl.form);
                }

                return result;
            }
            return null;
        },

        detectSearch: (elements) => {
            let searchInput = null;
            let submitButton = null;

            for (const el of elements) {
                const role = el.role;
                const type = el.type;
                const text = (el.text || '').toLowerCase();
                const placeholder = (el.attributes?.placeholder || '').toLowerCase();
                const name = (el.attributes?.name || '').toLowerCase();

                // Search input
                if (
                    role === 'search' ||
                    name.includes('search') ||
                    name === 'q' ||
                    name === 'query' ||
                    placeholder.includes('search')
                ) {
                    searchInput = el.id;
                }

                // Search submit button
                if (
                    searchInput &&
                    (role === 'button' || type === 'input') &&
                    (text.includes('search') || text === 'go' || text === '')
                ) {
                    submitButton = el.id;
                }
            }

            if (searchInput) {
                const result = { input: searchInput };
                if (submitButton) result.submit = submitButton;
                return result;
            }
            return null;
        },

        detectPagination: (elements) => {
            let prevButton = null;
            let nextButton = null;
            const pageNumbers = [];

            for (const el of elements) {
                const role = el.role;
                const text = (el.text || '').toLowerCase().trim();
                const ariaLabel = (el.attributes?.['aria-label'] || '').toLowerCase();

                // Previous button
                if (
                    (role === 'button' || role === 'link') &&
                    (text === 'prev' ||
                        text === 'previous' ||
                        text === '‹' ||
                        text === '«' ||
                        ariaLabel.includes('previous') ||
                        ariaLabel.includes('prev'))
                ) {
                    prevButton = el.id;
                }

                // Next button
                if (
                    (role === 'button' || role === 'link') &&
                    (text === 'next' || text === '›' || text === '»' || ariaLabel.includes('next'))
                ) {
                    nextButton = el.id;
                }

                // Page number links (single digits or small numbers)
                if ((role === 'button' || role === 'link') && /^\d{1,3}$/.test(text)) {
                    pageNumbers.push({ page: parseInt(text), id: el.id });
                }
            }

            if (prevButton || nextButton || pageNumbers.length > 1) {
                const result = {};
                if (prevButton) result.prev = prevButton;
                if (nextButton) result.next = nextButton;
                if (pageNumbers.length > 0) {
                    result.pages = pageNumbers.sort((a, b) => a.page - b.page);
                }
                return result;
            }
            return null;
        },

        detectModal: () => {
            // Look for visible modal dialogs
            const modalSelectors = [
                '[role="dialog"]',
                '[aria-modal="true"]',
                '.modal:not(.hidden)',
                '.modal.show',
                '.modal.open',
                '[class*="modal"][class*="open"]',
                '[class*="modal"][class*="show"]',
                '[class*="dialog"][class*="open"]'
            ];

            for (const selector of modalSelectors) {
                try {
                    const modal = document.querySelector(selector);
                    if (modal && Utils.isVisible(modal)) {
                        const result = {
                            container: Utils.generateSelector(modal)
                        };

                        // Find close button
                        const closeSelectors = [
                            '[aria-label*="close"]',
                            '[aria-label*="Close"]',
                            '.close',
                            '.modal-close',
                            '[class*="close"]',
                            'button:has(svg)' // Icon buttons often close modals
                        ];

                        for (const closeSelector of closeSelectors) {
                            try {
                                const closeBtn = modal.querySelector(closeSelector);
                                if (closeBtn) {
                                    const closeId = STATE.inverseMap.get(closeBtn);
                                    if (closeId) result.close = closeId;
                                    break;
                                }
                            } catch (_e) {
                                /* ignore invalid selectors */
                            }
                        }

                        // Find title
                        const titleSelectors = ['.modal-title', '[class*="title"]', 'h1', 'h2', 'h3'];
                        for (const titleSelector of titleSelectors) {
                            const title = modal.querySelector(titleSelector);
                            if (title && title.textContent.trim()) {
                                result.title = title.textContent.trim().substring(0, 100);
                                break;
                            }
                        }

                        return result;
                    }
                } catch (_e) {
                    /* ignore invalid selectors */
                }
            }
            return null;
        },

        detectCookieBanner: () => {
            // Look for cookie consent banners
            const bannerSelectors = [
                '[class*="cookie"]',
                '[class*="consent"]',
                '[class*="gdpr"]',
                '[id*="cookie"]',
                '[id*="consent"]',
                '[id*="gdpr"]',
                '[aria-label*="cookie"]',
                '[aria-label*="consent"]'
            ];

            for (const selector of bannerSelectors) {
                try {
                    const banners = document.querySelectorAll(selector);
                    for (const banner of banners) {
                        if (!Utils.isVisible(banner)) continue;

                        // Look for accept/reject buttons within
                        const acceptPatterns = ['accept', 'agree', 'allow', 'ok', 'got it', 'i understand'];
                        const rejectPatterns = ['reject', 'decline', 'deny', 'refuse', 'no thanks'];

                        let acceptBtn = null;
                        let rejectBtn = null;

                        const buttons = banner.querySelectorAll('button, a[role="button"], [class*="btn"]');
                        for (const btn of buttons) {
                            const btnText = (btn.textContent || '').toLowerCase().trim();

                            if (!acceptBtn && acceptPatterns.some((p) => btnText.includes(p))) {
                                acceptBtn = STATE.inverseMap.get(btn);
                            }
                            if (!rejectBtn && rejectPatterns.some((p) => btnText.includes(p))) {
                                rejectBtn = STATE.inverseMap.get(btn);
                            }
                        }

                        if (acceptBtn || rejectBtn) {
                            const result = {
                                container: Utils.generateSelector(banner)
                            };
                            if (acceptBtn) result.accept = acceptBtn;
                            if (rejectBtn) result.reject = rejectBtn;
                            return result;
                        }
                    }
                } catch (_e) {
                    /* ignore invalid selectors */
                }
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

            const cmd = message.cmd;
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

                // Extraction
                case 'get_text':
                    result = Extractor.get_text(message);
                    break;
                case 'get_value':
                    result = Extractor.get_value(message);
                    break;
                case 'exists':
                    result = Extractor.exists(message);
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
            if (result && result.ok && !result.timing) {
                result.timing = { duration_ms: performance.now() - t0 };
            }
            return result;
        } catch (e) {
            console.error('Scanner error:', e);
            if (e.code) return Protocol.error(e.msg || e.message, e.code);
            return Protocol.error(e.message || 'Internal Error', 'INTERNAL_ERROR');
        }
    }

    // Attach to global
    global.Lemmascope = global.Lemmascope || {};
    global.Lemmascope.process = process;
    global.Lemmascope.Scanner = Scanner; // Export for debugging
    global.Lemmascope.State = STATE;
})(window);
