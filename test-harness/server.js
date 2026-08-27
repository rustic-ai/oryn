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
app.use('/spa', express.static(path.join(__dirname, 'scenarios/spa')));
app.use('/spa/react', express.static(path.join(__dirname, 'scenarios/spa-react')));
app.use('/modals', express.static(path.join(__dirname, 'scenarios/modals')));
app.use('/dynamic', express.static(path.join(__dirname, 'scenarios/dynamic')));
app.use('/nav', express.static(path.join(__dirname, 'scenarios/navigation')));
app.use('/edge', express.static(path.join(__dirname, 'scenarios/edge-cases')));
app.use('/intent-tests', express.static(path.join(__dirname, 'scenarios/intent-tests')));
app.use('/g3/auth', require('./scenarios/g3-auth/api'));

// G3 framework build outputs are reproducible, ignored artifacts. The source
// applications and lockfile live under benchmarks/g3/frameworks.
const g3Apps = path.join(__dirname, '..', 'benchmarks', 'g3', 'frameworks', 'apps');
const g3Outputs = {
    angular: path.join(g3Apps, 'angular', 'dist', 'browser'),
    next: path.join(g3Apps, 'next', 'out'),
    nuxt: path.join(g3Apps, 'nuxt', '.output', 'public'),
    preact: path.join(g3Apps, 'preact', 'dist'),
    react: path.join(g3Apps, 'react', 'dist'),
    svelte: path.join(g3Apps, 'svelte', 'dist'),
    sveltekit: path.join(g3Apps, 'sveltekit', 'build'),
    vue: path.join(g3Apps, 'vue', 'dist'),
};
for (const [framework, directory] of Object.entries(g3Outputs)) {
    app.use(`/g3/frameworks/${framework}`, express.static(directory, { extensions: ['html'] }));
}

// Main index
app.get('/', (req, res) => {
    res.sendFile(path.join(__dirname, 'index.html'));
});

app.listen(PORT, '0.0.0.0', () => {
    console.log(`Test harness running at http://0.0.0.0:${PORT}`);
});
