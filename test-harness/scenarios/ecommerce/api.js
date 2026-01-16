const express = require('express');
const router = express.Router();

// Mock data
const products = [
    { id: 1, name: 'Laptop', price: 999.99, category: 'Electronics', description: 'Powerful work laptop' },
    { id: 2, name: 'Smartphone', price: 699.99, category: 'Electronics', description: 'Latest flagship model' },
    { id: 3, name: 'Coffee Maker', price: 49.99, category: 'Home', description: 'Brews the perfect cup' },
    { id: 4, name: 'Running Shoes', price: 89.99, category: 'Apparel', description: 'Comfortable and durable' }
];

let cart = [];

// GET products
router.get('/api/products', (req, res) => {
    let filtered = products;
    if (req.query.category) {
        filtered = products.filter(p => p.category.toLowerCase() === req.query.category.toLowerCase());
    }
    res.json(filtered);
});

// GET product by id
router.get('/api/products/:id', (req, res) => {
    const product = products.find(p => p.id === parseInt(req.params.id));
    if (product) res.json(product);
    else res.status(404).json({ error: 'Product not found' });
});

// POST add to cart
router.post('/api/cart', (req, res) => {
    const { productId, quantity } = req.body;
    const product = products.find(p => p.id === productId);
    if (product) {
        cart.push({ ...product, quantity: quantity || 1 });
        res.json({ message: 'Added to cart', cartSize: cart.length });
    } else {
        res.status(404).json({ error: 'Product not found' });
    }
});

// GET cart
router.get('/api/cart', (req, res) => {
    res.json(cart);
});

// DELETE from cart
router.delete('/api/cart/:id', (req, res) => {
    cart = cart.filter(p => p.id !== parseInt(req.params.id));
    res.json({ message: 'Removed from cart', cartSize: cart.length });
});

// Serve static files for e-commerce
router.use('/', express.static(require('path').join(__dirname)));

module.exports = router;
