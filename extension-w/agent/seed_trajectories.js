/**
 * Seed Trajectories
 *
 * Pre-defined successful task execution examples for few-shot learning.
 * These trajectories help the Ralph agent understand common web automation patterns.
 */

export const SEED_TRAJECTORIES = [
    // Search Tasks
    {
        task: 'Search for blue backpacks',
        url: 'https://www.amazon.com',
        commands: [
            'type "blue backpacks" into searchbox',
            'click "Go" button',
        ],
        success: true,
        metadata: {
            category: 'search',
            description: 'Simple search on e-commerce site',
        },
    },
    {
        task: 'Search for laptops',
        url: 'https://www.google.com',
        commands: [
            'type "laptops" into search',
            'press Enter',
        ],
        success: true,
        metadata: {
            category: 'search',
            description: 'Google search',
        },
    },
    {
        task: 'Find running shoes',
        url: 'https://www.nike.com',
        commands: [
            'click search icon',
            'type "running shoes" into searchbox',
            'press Enter',
        ],
        success: true,
        metadata: {
            category: 'search',
            description: 'Search with icon click first',
        },
    },

    // E-commerce Tasks
    {
        task: 'Add item to cart',
        url: 'https://www.amazon.com/product',
        commands: [
            'click "Add to Cart" button',
        ],
        success: true,
        metadata: {
            category: 'ecommerce',
            description: 'Simple add to cart',
        },
    },
    {
        task: 'Buy blue backpack',
        url: 'https://www.amazon.com',
        commands: [
            'type "blue backpack" into searchbox',
            'click "Go" button',
            'click first product',
            'click "Add to Cart" button',
        ],
        success: true,
        metadata: {
            category: 'ecommerce',
            description: 'Complete purchase flow',
        },
    },
    {
        task: 'Add laptop to cart and proceed to checkout',
        url: 'https://www.bestbuy.com',
        commands: [
            'type "laptop" into search',
            'press Enter',
            'click first product',
            'click "Add to Cart" button',
            'click "Proceed to Checkout" button',
        ],
        success: true,
        metadata: {
            category: 'ecommerce',
            description: 'Multi-step purchase with checkout',
        },
    },

    // Navigation Tasks
    {
        task: 'Go to settings page',
        url: 'https://app.example.com',
        commands: [
            'click "Settings" link',
        ],
        success: true,
        metadata: {
            category: 'navigation',
            description: 'Simple navigation',
        },
    },
    {
        task: 'Navigate to profile settings',
        url: 'https://app.example.com',
        commands: [
            'click "Profile" menu',
            'click "Settings" link',
        ],
        success: true,
        metadata: {
            category: 'navigation',
            description: 'Menu-based navigation',
        },
    },

    // Form Filling Tasks
    {
        task: 'Fill out contact form',
        url: 'https://www.example.com/contact',
        commands: [
            'type "John Doe" into name field',
            'type "john@example.com" into email field',
            'type "Hello world" into message field',
            'click "Submit" button',
        ],
        success: true,
        metadata: {
            category: 'forms',
            description: 'Basic form filling',
        },
    },
    {
        task: 'Subscribe to newsletter',
        url: 'https://www.example.com',
        commands: [
            'type "user@example.com" into email field',
            'click "Subscribe" button',
        ],
        success: true,
        metadata: {
            category: 'forms',
            description: 'Simple subscription form',
        },
    },
    {
        task: 'Register new account',
        url: 'https://www.example.com/register',
        commands: [
            'type "john@example.com" into email field',
            'type "SecurePass123" into password field',
            'type "SecurePass123" into confirm password field',
            'click "I agree to terms" checkbox',
            'click "Create Account" button',
        ],
        success: true,
        metadata: {
            category: 'forms',
            description: 'Registration form with checkbox',
        },
    },

    // Login Tasks
    {
        task: 'Login to account',
        url: 'https://www.example.com/login',
        commands: [
            'type "user@example.com" into email field',
            'type "password123" into password field',
            'click "Login" button',
        ],
        success: true,
        metadata: {
            category: 'login',
            description: 'Basic login flow',
        },
    },
    {
        task: 'Sign in with username',
        url: 'https://www.github.com/login',
        commands: [
            'type "username" into username field',
            'type "password" into password field',
            'click "Sign in" button',
        ],
        success: true,
        metadata: {
            category: 'login',
            description: 'Login with username instead of email',
        },
    },

    // Selection Tasks
    {
        task: 'Select shirt size large',
        url: 'https://www.store.com/product',
        commands: [
            'click size dropdown',
            'click "Large" option',
        ],
        success: true,
        metadata: {
            category: 'selection',
            description: 'Dropdown selection',
        },
    },
    {
        task: 'Filter products by price',
        url: 'https://www.store.com/products',
        commands: [
            'click "Price" filter',
            'click "$50 - $100" option',
        ],
        success: true,
        metadata: {
            category: 'selection',
            description: 'Filter application',
        },
    },

    // Content Reading Tasks
    {
        task: 'Read the first article',
        url: 'https://www.news.com',
        commands: [
            'click first article headline',
        ],
        success: true,
        metadata: {
            category: 'reading',
            description: 'Navigate to article',
        },
    },
    {
        task: 'Find product reviews',
        url: 'https://www.amazon.com/product',
        commands: [
            'scroll to reviews section',
            'click "See all reviews" link',
        ],
        success: true,
        metadata: {
            category: 'reading',
            description: 'Navigate to reviews',
        },
    },

    // Modal/Dialog Tasks
    {
        task: 'Accept cookie banner',
        url: 'https://www.example.com',
        commands: [
            'click "Accept" button',
        ],
        success: true,
        metadata: {
            category: 'modal',
            description: 'Cookie banner acceptance',
        },
    },
    {
        task: 'Close popup',
        url: 'https://www.example.com',
        commands: [
            'click close button',
        ],
        success: true,
        metadata: {
            category: 'modal',
            description: 'Modal dismissal',
        },
    },

    // Download Tasks
    {
        task: 'Download PDF document',
        url: 'https://www.example.com/docs',
        commands: [
            'click "Download PDF" button',
        ],
        success: true,
        metadata: {
            category: 'download',
            description: 'Simple download',
        },
    },

    // Pagination Tasks
    {
        task: 'Go to next page of results',
        url: 'https://www.example.com/search',
        commands: [
            'click "Next" button',
        ],
        success: true,
        metadata: {
            category: 'pagination',
            description: 'Navigate to next page',
        },
    },
];

/**
 * Load seed trajectories into the store
 */
export async function loadSeedTrajectories(trajectoryStore) {
    console.log('[Seed Trajectories] Loading seed trajectories...');

    let loaded = 0;
    for (const trajectory of SEED_TRAJECTORIES) {
        try {
            // Add timestamp
            trajectory.timestamp = Date.now();
            await trajectoryStore.save(trajectory);
            loaded++;
        } catch (error) {
            console.error('[Seed Trajectories] Failed to load trajectory:', error);
        }
    }

    console.log('[Seed Trajectories] Loaded', loaded, 'seed trajectories');
    return loaded;
}
