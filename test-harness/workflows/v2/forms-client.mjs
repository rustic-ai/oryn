// Keep all successful controls and native file handles in the actual form.
// The browser owns validation, encoding, submitter selection and upload bytes.
export function hydrateApplication(document, fetcher = fetch) {
  const form = document.querySelector('#application');
  if (!form) return;
  const first = document.querySelector('#identity-step');
  const details = document.querySelector('#details-step');
  const email = document.querySelector('#email');
  const status = document.querySelector('#validation-status');
  let generation = 0;
  let acceptedEmail = null;
  let pending;
  function returnToIdentity() {
    ++generation;
    pending?.abort();
    pending = null;
    acceptedEmail = null;
    first.hidden = false;
    details.hidden = true;
    form.dataset.state = 'editing';
    status.textContent = '';
  }
  email.addEventListener('input', returnToIdentity);
  document.defaultView?.addEventListener('pagehide', () => {
    returnToIdentity();
    document.querySelector('#role-options')?.remove();
  });
  document.querySelector('#next-step').addEventListener('click', async () => {
    if (![...first.querySelectorAll('input,select,textarea')].every(control => control.reportValidity())) return;
    const current = ++generation;
    const value = email.value;
    pending?.abort();
    const controller = new AbortController();
    pending = controller;
    form.dataset.state = 'validating';
    status.textContent = 'Validating email';
    try {
      const response = await fetcher(`/frontend/${form.elements.case_id.value}/validate-email?email=${encodeURIComponent(value)}`,
        {signal: controller.signal, credentials: 'same-origin'});
      if (!response.ok) throw new Error('Validation unavailable');
      const result = await response.json();
      if (generation !== current || value !== email.value) return;
      if (result.valid !== true) {
        form.dataset.state = 'rejected';
        status.textContent = 'Email is not eligible';
        return;
      }
      acceptedEmail = value;
      first.hidden = true;
      details.hidden = false;
      form.dataset.state = 'review';
    } catch (error) {
      if (current !== generation || controller.signal.aborted) return;
      form.dataset.state = 'error';
      status.textContent = error.message;
    }
  });
  document.querySelector('#previous-step').addEventListener('click', returnToIdentity);
  form.addEventListener('submit', event => {
    if (acceptedEmail !== email.value || form.dataset.state !== 'review') event.preventDefault();
  });
  const picker = document.querySelector('#role-picker');
  picker.addEventListener('click', () => {
    if (document.querySelector('#role-options')) return;
    const menu = document.createElement('div');
    menu.id = 'role-options';
    menu.setAttribute('role', 'listbox');
    menu.setAttribute('aria-label', 'Primary role options');
    for (const [value, label] of [['designer', 'Designer'], ['developer', 'Developer']]) {
      const button = document.createElement('button');
      button.type = 'button';
      button.setAttribute('role', 'option');
      button.setAttribute('aria-selected', String(form.elements.role.value === value));
      button.textContent = `Choose ${label}`;
      button.addEventListener('click', () => {
        form.elements.role.value = value;
        picker.textContent = `Primary role: ${label}`;
        menu.remove();
      });
      menu.append(button);
    }
    menu.addEventListener('keydown', event => {
      if (event.key === 'Escape') { menu.remove(); picker.focus(); }
    });
    document.body.append(menu);
    menu.querySelector('button')?.focus();
  });
  form.dataset.state = 'editing';
}
if (typeof document !== 'undefined') hydrateApplication(document);
