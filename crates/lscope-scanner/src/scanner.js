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

    // --- Protocol ---

    const Protocol = {
        success: (data = {}, timingStart = null) => {
            const response = { ok: true, ...data };
            if (timingStart) {
                response.timing = { duration_ms: performance.now() - timingStart };
            }
            return response;
        },
        error: (msg, code = 'UNKNOWN_ERROR') => {
            return { ok: false, error: msg, code: code };
        }
    };

    // --- Helpers ---

    const Utils = {
        isVisible: (el) => {
            if (!el.isConnected) return false;
            // Check robust visibility: layout size, style, and ancestry
            const rect = el.getBoundingClientRect();
            if (rect.width === 0 || rect.height === 0) return false;

            const style = window.getComputedStyle(el);
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
            // Simple robust selector generation
            if (el.id) return `#${CSS.escape(el.id)}`;

            // Fallback to path
            const path = [];
            while (el.nodeType === Node.ELEMENT_NODE) {
                const tag = el.tagName.toLowerCase();
                if (el.id) {
                    path.unshift(`#${CSS.escape(el.id)}`);
                    break;
                } else {
                    let sibling = el,
                        nth = 1;
                    while ((sibling = sibling.previousElementSibling)) {
                        if (sibling.tagName.toLowerCase() === tag) nth++;
                    }
                    path.unshift(`${tag}:nth-of-type(${nth})`);
                    el = el.parentNode;
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

            const topElement = document.elementFromPoint(centerX, centerY);

            // Element is interactable if it's the top element or contains the top element
            if (!topElement) return false;
            return el === topElement || el.contains(topElement) || topElement.contains(el);
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

            while (treeWalker.nextNode() && elements.length < maxElements) {
                const el = treeWalker.currentNode;
                if (!includeHidden && !Utils.isVisible(el)) continue;
                if (params.viewport_only && !Utils.isInViewport(el)) continue;

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
            const state = {
                visible: Utils.isVisible(el),
                disabled: !!el.disabled,
                focused: document.activeElement === el
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
                    'aria-label': el.getAttribute('aria-label')
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

            return Protocol.success({
                action: clickCount > 1 ? 'double_clicked' : 'clicked',
                id: params.id,
                tag: el.tagName.toLowerCase(),
                selector: Utils.generateSelector(el),
                coordinates: { x: Math.round(clientX), y: Math.round(clientY) },
                button: buttonType
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
                text: text,
                value: el.isContentEditable ? el.innerText : el.value
            });
        },

        clear: (params) => {
            const el = Executor.getElement(params.id);
            el.value = '';
            el.dispatchEvent(new Event('input', { bubbles: true }));
            el.dispatchEvent(new Event('change', { bubbles: true }));
            return Protocol.success({ action: 'cleared', id: params.id });
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

            let optionToSelect = null;
            // Strategy: find option by value, then text, then index
            if (params.value !== undefined) {
                optionToSelect = Array.from(el.options).find((o) => o.value === params.value);
            } else if (params.text !== undefined) {
                optionToSelect = Array.from(el.options).find((o) => o.text.includes(params.text));
            } else if (params.index !== undefined) {
                optionToSelect = el.options[params.index];
            }

            if (!optionToSelect) throw { msg: 'Option not found', code: 'OPTION_NOT_FOUND' };

            el.value = optionToSelect.value;
            el.dispatchEvent(new Event('change', { bubbles: true }));
            el.dispatchEvent(new Event('input', { bubbles: true }));

            return Protocol.success({
                action: 'selected',
                id: params.id,
                value: el.value,
                text: optionToSelect.text,
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
            return Protocol.success({ action: 'focused', id: params.id });
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

            return Protocol.success({ action: 'submitted' });
        },

        wait_for: async (params) => {
            const timeout = params.timeout || 30000;
            const start = performance.now();

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
                    default:
                        return false;
                }
            };

            const poll = () => {
                const elapsed = performance.now() - start;
                if (elapsed >= timeout) {
                    return Promise.reject({ msg: 'Timeout waiting for condition', code: 'TIMEOUT' });
                }

                if (checkCondition()) {
                    return Promise.resolve(true);
                }

                return new Promise((r) => setTimeout(r, 100)).then(poll);
            };

            try {
                const met = await poll();
                return Protocol.success({
                    condition_met: met,
                    waited_ms: Math.round(performance.now() - start)
                });
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
        try {
            if (typeof message === 'string') message = JSON.parse(message);

            const cmd = message.cmd;
            if (!cmd) return Protocol.error('Missing command', 'INVALID_REQUEST');

            // Dispatch
            switch (cmd) {
                case 'scan':
                    return Scanner.scan(message);

                // Actions
                case 'click':
                    return Executor.click(message);
                case 'type':
                    return await Executor.type(message);
                case 'clear':
                    return Executor.clear(message);
                case 'check':
                    return Executor.check(message, true);
                case 'uncheck':
                    return Executor.check(message, false);
                case 'select':
                    return Executor.select(message);
                case 'scroll':
                    return Executor.scroll(message);
                case 'focus':
                    return Executor.focus(message);
                case 'hover':
                    return Executor.hover(message);
                case 'submit':
                    return Executor.submit(message);
                case 'wait_for':
                    return await Executor.wait_for(message);

                // Extraction
                case 'get_text':
                    return Extractor.get_text(message);
                case 'get_value':
                    return Extractor.get_value(message);
                case 'exists':
                    return Extractor.exists(message);
                case 'execute':
                    return Extractor.execute(message);

                // System
                case 'version':
                    return System.version();

                default:
                    return Protocol.error(`Unknown command: ${cmd}`, 'UNKNOWN_COMMAND');
            }
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
