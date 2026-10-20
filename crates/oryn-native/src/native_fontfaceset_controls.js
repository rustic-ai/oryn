// Run only through the coordinator's installed native FontFaceSet fixture.
// These controls are not a substitute for the original pinned WPT population.
(() => {
  const check = (value, message) => { if (!value) throw Error(message); };
  const typeError = callback => {
    try { callback(); } catch (error) { check(error.name === 'TypeError', 'receiver/arity exception'); return; }
    throw Error('missing TypeError');
  };
  const fonts = document.fonts;
  typeError(() => Object.getOwnPropertyDescriptor(Document.prototype, 'fonts').get.call({}));
  typeError(() => Object.getOwnPropertyDescriptor(FontFaceSet.prototype, 'size').get.call({}));
  // WebIDL promise-returning operations reject conversion/arity errors.
  globalThis.fontLoadArityRejected = false;
  FontFaceSet.prototype.load.call(fonts).catch(error => {
    check(error.name === 'TypeError', 'load arity rejection');
    globalThis.fontLoadArityRejected = true;
  });
  typeError(() => FontFaceSet.prototype.check.call(fonts));
  typeError(() => FontFaceSet.prototype.values.call({}));
  typeError(() => FontFaceSet.prototype.forEach.call({}, () => {}));
  typeError(() => Object.getOwnPropertyDescriptor(FontFaceSetLoadEvent.prototype, 'fontfaces').get.call({}));
  for (const name of ['onloading', 'onloadingdone', 'onloadingerror']) {
    const handler = () => {};
    fonts[name] = handler;
    check(fonts[name] === handler, name + ' native event handler');
    fonts[name] = null;
  }
  // Actual native typed constructor and FontFaceSet.load selection. Missing
  // local sources fail normally; both families must participate in selection.
  const a = new FontFace('NativeFamilyA', 'local("oryn-missing-a")', {weight: '400'});
  const b = new FontFace('NativeFamilyB', 'local("oryn-missing-b")', {weight: '700'});
  fonts.add(a); fonts.add(b);
  const loading = fonts.load('400 16px NativeFamilyA, NativeFamilyB', 'x');
  check(a.status === 'loading' && b.status === 'loading',
    'rank independently within each family: ' + JSON.stringify({
      a: a.status, b: b.status, size: fonts.size,
      hasA: fonts.has(a), hasB: fonts.has(b),
      familyA: a.family, familyB: b.family, weightA: a.weight, weightB: b.weight
    }));
  loading.catch(() => {});
  fonts.delete(a); fonts.delete(b);
})();
true;
