 'use strict';
// Diagnostic variant keeps actual native mappings reachable until the runner
// captures placement; it supplies no placement or process claims itself.
const placementResources = [];
const retainPlacement = new URL(location.href).searchParams.get('placement') === '1';
onmessage = async ({data}) => {
  const {id, command, buffer, memory} = data;
  if (retainPlacement) placementResources.push(data);
  try {
    if (command === 'wait') {
      const values = new Int32Array(buffer);
      postMessage({id, event:'armed'});
      const result = Atomics.wait(values, 0, 0, 10000);
      postMessage({id, event:'done', result});
    } else if (command === 'notify') {
      postMessage({id, event:'done', count:Atomics.notify(new Int32Array(buffer), 0, 1)});
    } else if (command === 'grow') {
      const view = new Uint8Array(buffer);
      buffer.grow(data.length);
      postMessage({id, event:'done', length:buffer.byteLength, viewLength:view.length,
        tail:view[data.length-1]});
    } else if (command === 'wasm-grow') {
      const previous = memory.grow(1);
      postMessage({id, event:'done', previous, length:memory.buffer.byteLength});
    }
  } catch (error) {
    postMessage({id, event:'error', name:error.name, message:error.message});
  }
};
