import {resolveLocalTime} from './timezone-resolution.mjs';

const form = document.querySelector('#appointment-search');
const status = document.querySelector('#selection-status');
const occurrences = document.querySelector('#occurrence');
const review = document.querySelector('#review-appointment');
let slots = [];
let choices = [];
let requestGeneration = 0;
let availabilityReady = false;
let pending;

function update() {
  review.disabled = true;
  occurrences.replaceChildren(new Option('Choose occurrence', ''));
  choices = [];
  try {
    choices = resolveLocalTime(form.elements.local.value, form.elements.zone.value);
    if (!choices.length) {
      status.dataset.state = 'gap';
      status.textContent = 'This local time does not exist. Choose another time.';
      return;
    }
    for (const [index, instant] of choices.entries()) {
      occurrences.append(new Option(`${index === 0 ? 'First' : 'Second'} occurrence: ${instant}`, instant));
    }
    if (choices.length === 1) occurrences.value = choices[0];
    status.dataset.state = choices.length > 1 ? 'ambiguous' : 'ready';
    status.textContent = choices.length > 1 ? 'This time occurs twice. Choose an occurrence.' : 'Choose an available appointment.';
    selectOccurrence();
  } catch (error) {
    status.dataset.state = 'error';
    status.textContent = error.message;
  }
}

function selectOccurrence() {
  const slot = slots.find(slot => slot.available && slot.utc === occurrences.value);
  review.disabled = !availabilityReady || !slot;
  if (occurrences.value) status.dataset.state = slot ? 'available' : 'unavailable';
  if (occurrences.value) status.textContent = slot ? `Available: ${occurrences.value}` : 'This appointment is unavailable.';
}

async function refresh() {
  const generation = ++requestGeneration;
  pending?.abort();
  const controller = new AbortController();
  pending = controller;
  availabilityReady = false;
  slots = [];
  review.disabled = true;
  status.dataset.state = 'loading';
  status.textContent = 'Loading availability';
  try {
    const response = await fetch('./availability-data', {credentials: 'same-origin', cache: 'no-store', signal: controller.signal});
    if (!response.ok) throw new Error('Availability could not be loaded');
    const data = await response.json();
    if (generation !== requestGeneration) return;
    if (!Array.isArray(data.slots) || data.slots.some(slot =>
      typeof slot.slot !== 'string' || typeof slot.utc !== 'string' ||
      typeof slot.available !== 'boolean')) throw new Error('Invalid availability response');
    slots = data.slots;
    availabilityReady = true;
    update();
  } catch (error) {
    if (generation === requestGeneration) {
      availabilityReady = false;
      slots = [];
      review.disabled = true;
      status.dataset.state = 'error';
      status.textContent = error.message;
    }
  }
}

addEventListener('pagehide', () => {
  ++requestGeneration;
  pending?.abort();
  availabilityReady = false;
  slots = [];
  review.disabled = true;
});
addEventListener('pageshow', event => { if (event.persisted) refresh(); });

form.elements.local.addEventListener('input', update);
form.elements.zone.addEventListener('change', update);
occurrences.addEventListener('change', selectOccurrence);
document.querySelector('#refresh-availability').addEventListener('click', refresh);
form.addEventListener('submit', event => {
  event.preventDefault();
  const slot = slots.find(slot => slot.available && slot.utc === occurrences.value);
  if (!availabilityReady || !slot || !choices.includes(occurrences.value)) return;
  const query = new URLSearchParams({slot: slot.slot, zone: form.elements.zone.value,
    client_local: form.elements.local.value, client_utc: occurrences.value,
    appointment_option: form.elements.appointment_option.value});
  location.href = `./review?${query}`;
});
refresh();
