const express = require('express');
const path = require('path');

const app = express();
const PORT = process.env.PORT || 3000;

app.use((req, res, next) => {
    console.log(`${new Date().toISOString()} [HARNESS] ${req.method} ${req.url}`);
    next();
});

app.get('/ping', (req, res) => res.send('pong'));

app.get('/log', (req, res) => {
    console.log(`${new Date().toISOString()} [EXT_LOG] ${req.query.msg}`);
    res.send('ok');
});

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
app.use('/intent-tests', express.static(path.join(__dirname, 'scenarios/intent-tests')));

// Main index
app.get('/', (req, res) => {
    res.sendFile(path.join(__dirname, 'index.html'));
});

app.listen(PORT, '0.0.0.0', () => {
    console.log(`Test harness running at http://0.0.0.0:${PORT}`);
});
