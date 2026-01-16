const express = require('express');
const path = require('path');

const app = express();
const PORT = process.env.PORT || 3000;

// JSON body parsing for API
app.use(express.json());

// Serve shared assets
app.use('/shared', express.static(path.join(__dirname, 'public/shared')));

// Mount scenario routes
app.use('/static', express.static(path.join(__dirname, 'scenarios/static')));
app.use('/forms', express.static(path.join(__dirname, 'scenarios/forms')));
app.use('/shop', require('./scenarios/ecommerce/api'));
app.use('/spa/react', express.static(path.join(__dirname, 'scenarios/spa-react')));
app.use('/modals', express.static(path.join(__dirname, 'scenarios/modals')));
app.use('/dynamic', express.static(path.join(__dirname, 'scenarios/dynamic')));
app.use('/nav', express.static(path.join(__dirname, 'scenarios/navigation')));
app.use('/edge', express.static(path.join(__dirname, 'scenarios/edge-cases')));

// Main index
app.get('/', (req, res) => {
    res.sendFile(path.join(__dirname, 'index.html'));
});

app.listen(PORT, () => {
    console.log(`Test harness running at http://localhost:${PORT}`);
});
