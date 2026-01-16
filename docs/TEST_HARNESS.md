# Lemmascope Test Harness Application

## Overview

A Node.js/Express application serving multiple test scenarios for comprehensive Lemmascope testing. Each scenario simulates real-world web applications with different interaction patterns.

## Project Structure

```
test-harness/
├── package.json
├── server.js                    # Express server with route mounting
├── index.html                   # Main landing page
├── public/
│   └── shared/                  # Shared CSS, images, fonts
│       └── styles.css
├── scenarios/
│   ├── static/                  # Plain HTML pages
│   │   ├── index.html          # Landing page with links
│   │   ├── article.html        # Text extraction test
│   │   └── table.html          # Data tables
│   ├── forms/                   # Form filling scenarios
│   │   ├── login.html          # Login form (email + password)
│   │   ├── registration.html   # Multi-field registration
│   │   ├── checkout.html       # E-commerce checkout
│   │   └── survey.html         # Radio, checkbox, select
│   ├── ecommerce/              # Shopping flow
│   │   ├── catalog.html        # Product grid with filters
│   │   ├── product.html        # Product detail page
│   │   ├── cart.html           # Shopping cart
│   │   └── api.js              # Mock API endpoints
│   ├── spa-react/              # React SPA
│   │   ├── index.html          # React mount point
│   │   └── app.js              # Main app (ESM)
│   ├── modals/                  # Modal/dialog scenarios
│   │   ├── basic.html          # Simple modal
│   │   ├── confirm.html        # Confirm dialog
│   │   ├── nested.html         # Nested modals
│   │   └── cookie-banner.html  # GDPR consent
│   ├── dynamic/                 # Dynamic content
│   │   ├── infinite-scroll.html
│   │   ├── lazy-load.html
│   │   └── live-search.html
│   ├── navigation/              # Multi-page flows
│   │   ├── wizard.html         # Multi-step wizard
│   │   └── tabs.html           # Tab navigation
│   └── edge-cases/             # Edge case testing
│       ├── iframes.html
│       ├── shadow-dom.html
│       └── accessibility.html
└── README.md
```

---

## Scenarios

### 1. Static Pages (`/static`)

**Purpose:** Text extraction, link discovery, basic navigation

| Route | Description | Test Focus |
|-------|-------------|------------|
| `/static/` | Landing page | Link enumeration |
| `/static/article` | News article | Text extraction, headings, paragraphs |
| `/static/table` | Data table | Table parsing, row/column access |

**Elements to test:**
- Headings (h1-h6)
- Paragraphs and text content
- Links (internal, external, anchors)
- Lists (ordered, unordered)
- Images with alt text
- Tables with headers

---

### 2. Forms (`/forms`)

**Purpose:** Form filling, validation, submission

| Route | Description | Test Focus |
|-------|-------------|------------|
| `/forms/login` | Login form | Email/password, remember me, submit |
| `/forms/registration` | Registration | Multiple inputs, validation, dropdowns |
| `/forms/checkout` | Checkout | Address, payment, multi-section |
| `/forms/survey` | Survey form | Radio groups, checkboxes, multi-select |

**Elements to test:**
- Text inputs (text, email, password, tel, url)
- Textareas
- Select dropdowns (single and multi)
- Radio button groups
- Checkbox groups
- Date/time pickers
- File uploads
- Form validation (required, pattern, min/max)
- Submit buttons
- Form reset

**Pattern detection:**
- Login form pattern (email + password + submit)
- Search form pattern (input + button)

---

### 3. E-commerce (`/shop`)

**Purpose:** Product browsing, cart management, checkout flow

| Route | Description | Test Focus |
|-------|-------------|------------|
| `/shop/` | Product catalog | Filters, sorting, pagination |
| `/shop/product/:id` | Product detail | Add to cart, quantity, variants |
| `/shop/cart` | Shopping cart | Update qty, remove, proceed |
| `/shop/checkout` | Checkout flow | Address, payment, confirmation |

**API Endpoints:**
```
GET  /shop/api/products       # List products (with filters)
GET  /shop/api/products/:id   # Get single product
POST /shop/api/cart           # Add to cart
GET  /shop/api/cart           # Get cart contents
PUT  /shop/api/cart/:id       # Update cart item
DELETE /shop/api/cart/:id     # Remove from cart
```

**Elements to test:**
- Product cards with images
- Filter sidebar (checkboxes, range sliders)
- Sort dropdown
- Pagination controls
- Add to cart buttons
- Quantity selectors (+/- buttons, input)
- Cart item list
- Price calculations
- Promo code input

**Pattern detection:**
- Pagination pattern (prev/next, page numbers)
- Search pattern (search input)

---

### 4. React SPA (`/spa/react`)

**Purpose:** SPA navigation, virtual DOM, client-side routing

| Route | Description | Test Focus |
|-------|-------------|------------|
| `/spa/react/` | Dashboard | Component rendering, stats cards |
| `/spa/react/users` | User list | Data fetching, list rendering |
| `/spa/react/user/:id` | User detail | Dynamic routes, back navigation |
| `/spa/react/settings` | Settings page | Form state, toggles, save |

**Features to test:**
- Client-side routing (React Router)
- Component state changes
- Async data loading
- Loading spinners
- Error states
- Form state management
- Navigation between routes
- Browser back/forward

**Technical approach:**
- Use ES modules with import maps (no build step)
- Load React/ReactDOM from CDN
- Simple component structure

---

### 5. Modals & Dialogs (`/modals`)

**Purpose:** Overlay handling, focus trapping, accessibility

| Route | Description | Test Focus |
|-------|-------------|------------|
| `/modals/basic` | Basic modal | Open/close, backdrop click, ESC key |
| `/modals/confirm` | Confirm dialog | Accept/cancel actions |
| `/modals/nested` | Nested modals | Multiple layers, z-index |
| `/modals/cookie` | Cookie banner | Accept/reject, GDPR compliance |
| `/modals/toast` | Toast notifications | Auto-dismiss, stacking |

**Elements to test:**
- Modal trigger buttons
- Modal containers (role="dialog")
- Close buttons (X, Cancel)
- Action buttons (Confirm, Accept, Reject)
- Backdrop/overlay
- Focus management
- Keyboard navigation (Tab, ESC)

**Pattern detection:**
- Modal pattern (container, close, title)
- Cookie banner pattern (accept/reject buttons)

---

### 6. Dynamic Content (`/dynamic`)

**Purpose:** Async loading, DOM mutations, real-time updates

| Route | Description | Test Focus |
|-------|-------------|------------|
| `/dynamic/infinite` | Infinite scroll | Load more on scroll, loading indicator |
| `/dynamic/lazy` | Lazy loading | Images, components on demand |
| `/dynamic/search` | Live search | Debounced input, autocomplete |
| `/dynamic/chat` | Chat interface | Message list, new messages |

**Elements to test:**
- Dynamically added elements
- Loading states/spinners
- Scroll position detection
- Debounced inputs
- Autocomplete dropdowns
- Real-time updates (simulated)

**Scanner considerations:**
- Elements may not exist on initial scan
- Need to re-scan after actions
- wait_for conditions (exists, visible)

---

### 7. Navigation Patterns (`/nav`)

**Purpose:** Multi-step flows, tab interfaces

| Route | Description | Test Focus |
|-------|-------------|------------|
| `/nav/wizard` | Multi-step wizard | Next/back, step validation, progress |
| `/nav/tabs` | Tab interface | Tab switching, active state |
| `/nav/accordion` | Accordion | Expand/collapse sections |
| `/nav/breadcrumb` | Breadcrumb nav | Navigation history |

**Elements to test:**
- Step indicators
- Next/Previous buttons
- Tab buttons
- Tab panels (show/hide)
- Accordion headers
- Accordion content panels
- Progress indicators

---

### 8. Edge Cases (`/edge`)

**Purpose:** Testing scanner edge case handling

| Route | Description | Test Focus |
|-------|-------------|------------|
| `/edge/iframes` | Iframe scenarios | Same-origin, cross-origin, nested |
| `/edge/shadow` | Shadow DOM | Encapsulated components |
| `/edge/a11y` | Accessibility | ARIA labels, roles, landmarks |
| `/edge/covered` | Covered elements | Overlays, z-index, pointer-events |
| `/edge/hidden` | Hidden elements | display:none, visibility, opacity |

**Elements to test:**
- Same-origin iframes (accessible)
- Cross-origin iframes (blocked)
- Nested iframes
- Shadow DOM components
- Custom elements
- ARIA attributes
- Covered elements (should detect)
- Various hidden states

---

## Implementation

### Server Setup

```javascript
// server.js
const express = require('express');
const path = require('path');

const app = express();
const PORT = process.env.PORT || 3000;

// JSON body parsing for API
app.use(express.json());

// Serve shared assets
app.use('/shared', express.static('public/shared'));

// Mount scenario routes
app.use('/static', express.static('scenarios/static'));
app.use('/forms', express.static('scenarios/forms'));
app.use('/shop', require('./scenarios/ecommerce/api'));
app.use('/spa/react', express.static('scenarios/spa-react'));
app.use('/modals', express.static('scenarios/modals'));
app.use('/dynamic', express.static('scenarios/dynamic'));
app.use('/nav', express.static('scenarios/navigation'));
app.use('/edge', express.static('scenarios/edge-cases'));

// Main index
app.get('/', (req, res) => {
  res.sendFile(path.join(__dirname, 'index.html'));
});

app.listen(PORT, () => {
  console.log(`Test harness running at http://localhost:${PORT}`);
});
```

### Package.json

```json
{
  "name": "lemmascope-test-harness",
  "version": "1.0.0",
  "description": "Test harness for Lemmascope browser automation",
  "main": "server.js",
  "scripts": {
    "start": "node server.js",
    "dev": "nodemon server.js"
  },
  "dependencies": {
    "express": "^4.18.2"
  },
  "devDependencies": {
    "nodemon": "^3.0.0"
  }
}
```

### Shared Styles

Minimal CSS for consistent look:
- CSS variables for theming
- Responsive layout utilities
- Form styling
- Modal/overlay base styles
- No external framework dependencies

---

## Testing with Lemmascope

### Basic Flow

```bash
# Start test harness
cd test-harness
npm install
npm start

# In another terminal, run Lemmascope
cargo run --bin lscope-h

# Test commands
> goto "http://localhost:3000"
> observe
> click "Forms"
> observe                        # Should detect login pattern
> type #email "test@example.com"
> type #password "secret123"
> click "Sign In"
```

### Scenario-Specific Tests

**Login Form:**
```
> goto "http://localhost:3000/forms/login"
> observe
> type "Email" "user@example.com"     # Semantic target
> type "Password" "password123"
> check "Remember me"
> click "Sign In"
```

**E-commerce Flow:**
```
> goto "http://localhost:3000/shop"
> observe
> click "Electronics"                  # Filter
> click "Add to Cart" near "Laptop"   # Proximity
> goto "http://localhost:3000/shop/cart"
> observe
> click "Proceed to Checkout"
```

**Modal Handling:**
```
> goto "http://localhost:3000/modals/basic"
> click "Open Modal"
> wait visible [role="dialog"]
> observe                              # Should detect modal pattern
> click "Close"
> wait hidden [role="dialog"]
```

**SPA Navigation:**
```
> goto "http://localhost:3000/spa/react"
> observe
> click "Users"
> wait visible ".user-list"
> observe
> click "View" near "John Doe"
> back
```

---

## Priority Order

### Phase 1 (Core)
1. `server.js` - Express setup
2. `index.html` - Main landing page
3. `scenarios/static/*` - Basic pages
4. `scenarios/forms/login.html` - Login form

### Phase 2 (Forms & Shopping)
5. `scenarios/forms/*` - All form types
6. `scenarios/ecommerce/*` - Shop flow

### Phase 3 (Dynamic & SPA)
7. `scenarios/modals/*` - Modal scenarios
8. `scenarios/spa-react/*` - React app
9. `scenarios/dynamic/*` - Dynamic content

### Phase 4 (Edge Cases)
10. `scenarios/navigation/*` - Nav patterns
11. `scenarios/edge-cases/*` - Edge cases

---

## Success Criteria

- [ ] All scenarios load without errors
- [ ] Scanner detects appropriate patterns per scenario
- [ ] Form filling works with semantic targets
- [ ] SPA navigation triggers proper re-scans
- [ ] Modal detection works correctly
- [ ] Dynamic content can be waited for
- [ ] Edge cases are handled gracefully
