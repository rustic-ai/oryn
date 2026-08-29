const express = require('express');

const router = express.Router();
const rpOrigin = request => `${request.protocol}://${request.get('host')}`;
const page = (title, body, script = '') => `<!doctype html>
<html><head><meta charset="utf-8"><title>${title}</title></head>
<body>${body}<script>${script}</script></body></html>`;

router.get('/rp/popup', (request, response) => {
    const origin = rpOrigin(request);
    response.send(page('OAuth popup RP', `
        <h1>OAuth popup RP</h1>
        <button id="start">Sign in with IdP</button>
        <output id="status">Signed out</output>`, `
        let child = null;
        document.querySelector('#start').onclick = async () => {
            const verifier = 'oryn-pkce-verifier-17';
            const digest = new Uint8Array(await crypto.subtle.digest('SHA-256', new TextEncoder().encode(verifier)));
            const challenge = btoa(String.fromCharCode(...digest)).replace(/=/g, '').replace(/\\+/g, '-').replace(/\\//g, '_');
            const state = 'popup-state';
            sessionStorage.setItem('oauth-state', state);
            child = open('/app-compatibility/auth/idp/authorize?state=' + encodeURIComponent(state) + '&challenge=' + encodeURIComponent(challenge));
            document.querySelector('#status').textContent = child ? 'Popup opened' : 'Popup denied';
        };
        addEventListener('message', event => {
            if (event.origin !== ${JSON.stringify(origin)} || event.data.state !== sessionStorage.getItem('oauth-state')) return;
            document.querySelector('#status').textContent = event.data.code === 'pkce-code' && event.data.challenge ? 'Authenticated with PKCE' : 'Rejected callback';
            child && child.close();
        });`));
});

router.get('/idp/authorize', (request, response) => {
    const state = String(request.query.state || '');
    const challenge = String(request.query.challenge || '');
    const existingSession = /(?:^|;\s*)idp_session=ready(?:;|$)/.test(request.get('cookie') || '');
    const sso = request.query.sso === '1';
    const approve = `opener.postMessage({code:${JSON.stringify(sso ? 'sso-code' : 'pkce-code')},state:${JSON.stringify(state)},challenge:${JSON.stringify(challenge)}}, location.origin);close();`;
    response.send(page('Controlled IdP', `
        <h1>Controlled IdP</h1>
        <button id="approve">Approve</button>
        <button id="cancel">Cancel</button>
        <output id="status">Awaiting approval</output>`, `
        document.querySelector('#approve').onclick = () => { ${approve} };
        document.querySelector('#cancel').onclick = () => close();
        ${sso && existingSession ? `queueMicrotask(() => { ${approve} });` : ''}`));
});

router.get('/rp/redirect', (_request, response) => {
    response.send(page('OAuth redirect RP', `
        <h1>OAuth redirect RP</h1>
        <button id="start">Start redirect sign in</button>
        <output id="status">Signed out</output>`, `
        document.querySelector('#start').onclick = () => {
            sessionStorage.setItem('redirect-state', 'redirect-state');
            location.assign('/app-compatibility/auth/idp/redirect?state=redirect-state');
        };`));
});

router.get('/idp/redirect', (request, response) => {
    const state = String(request.query.state || '');
    response.send(page('OAuth redirect IdP', `
        <h1>OAuth redirect IdP</h1>
        <button id="approve">Approve redirect</button>`, `
        document.querySelector('#approve').onclick = () => location.assign('/app-compatibility/auth/rp/callback?code=redirect-code&state=' + encodeURIComponent(${JSON.stringify(state)}));`));
});

router.get('/rp/callback', (request, response) => {
    const state = String(request.query.state || '');
    const code = String(request.query.code || '');
    response.send(page('OAuth redirect callback', `
        <h1>OAuth redirect callback</h1>
        <output id="status">Validating</output>`, `
        const valid = sessionStorage.getItem('redirect-state') === ${JSON.stringify(state)} && ${JSON.stringify(code)} === 'redirect-code';
        document.querySelector('#status').textContent = valid ? 'Redirect authenticated' : 'Redirect rejected';`));
});

router.get('/idp/session', (_request, response) => {
    response.set('Set-Cookie', 'idp_session=ready; Path=/app-compatibility/auth; HttpOnly; SameSite=Lax');
    response.send(page('Controlled IdP session', '<h1>Session established</h1>'));
});

router.get('/rp/sso', (request, response) => {
    const origin = rpOrigin(request);
    response.send(page('Existing-session SSO RP', `
        <h1>Existing-session SSO RP</h1>
        <button id="start">Continue with SSO</button>
        <output id="status">Signed out</output>`, `
        document.querySelector('#start').onclick = () => open('/app-compatibility/auth/idp/authorize?sso=1&state=sso-state');
        addEventListener('message', event => {
            if (event.origin === ${JSON.stringify(origin)} && event.data.code === 'sso-code' && event.data.state === 'sso-state') document.querySelector('#status').textContent = 'SSO authenticated';
        });`));
});

router.get('/rp/mfa', (request, response) => {
    const origin = rpOrigin(request);
    response.send(page('MFA handoff RP', `
        <h1>MFA handoff RP</h1>
        <button id="start">Start MFA</button>
        <output id="status">MFA required</output>`, `
        document.querySelector('#start').onclick = () => open('/app-compatibility/auth/idp/mfa?state=mfa-state');
        addEventListener('message', event => {
            if (event.origin === ${JSON.stringify(origin)} && event.data.code === 'mfa-code' && event.data.state === 'mfa-state') document.querySelector('#status').textContent = 'MFA authenticated';
        });`));
});

router.get('/idp/mfa', (request, response) => {
    const state = String(request.query.state || '');
    response.send(page('Controlled MFA', `
        <h1>Controlled MFA</h1>
        <label>Verification code<input aria-label="Verification code"></label>
        <button id="verify">Verify code</button>
        <button id="cancel">Cancel</button>`, `
        document.querySelector('#verify').onclick = () => {
            if (document.querySelector('input').value === '246810') {
                opener.postMessage({code:'mfa-code',state:${JSON.stringify(state)}}, location.origin);
                close();
            }
        };
        document.querySelector('#cancel').onclick = () => close();`));
});

router.get('/rp/recovery', (_request, response) => {
    response.send(page('Popup recovery RP', `
        <h1>Popup recovery RP</h1>
        <button id="start">Open popup</button>
        <button id="check">Check popup</button>
        <output id="status">Ready</output>`, `
        let child = null;
        document.querySelector('#start').onclick = () => {
            child = open('/app-compatibility/auth/idp/authorize?state=recovery-state');
            document.querySelector('#status').textContent = child ? 'Popup opened' : 'Popup denied';
        };
        document.querySelector('#check').onclick = () => {
            globalThis.__oryn_poll_context?.();
            document.querySelector('#status').textContent = child && child.closed ? 'Popup closed; opener recovered' : document.querySelector('#status').textContent;
        };`));
});

module.exports = router;
