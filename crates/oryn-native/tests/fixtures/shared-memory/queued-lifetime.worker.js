self.onmessage = event => {
  const {shared, alias, port} = event.data;
  const words = new Int32Array(shared);
  const value = Atomics.load(words, 0);
  Atomics.store(words, 0, value + 1);
  shared.grow(16);
  port.postMessage({alias: shared === alias, value});
  port.close();
};
