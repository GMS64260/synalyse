/*! Synalyse — enregistreur de session. AGPL-3.0-or-later. https://github.com/GMS64260/synalyse */
(function (w, d) {
  'use strict';
  var ctx = w.synalyse && w.synalyse.rec;
  var lib = w.rrwebRecord;
  var record = lib && (lib.record || lib);
  if (!ctx || ctx.stop || typeof record !== 'function') return;

  var STATE = 'synalyse_rec';
  var GAP = 30 * 60 * 1000; // au-delà de 30 minutes sans activité, un nouvel enregistrement commence
  var now = Date.now();

  function session(value) {
    try {
      if (value === undefined) return JSON.parse(w.sessionStorage.getItem(STATE) || 'null');
      if (value === null) w.sessionStorage.removeItem(STATE);
      else w.sessionStorage.setItem(STATE, JSON.stringify(value));
    } catch (e) {}
    return null;
  }

  function newId() {
    var bytes = new Uint8Array(16);
    w.crypto.getRandomValues(bytes);
    var hex = '';
    for (var i = 0; i < bytes.length; i++) hex += (bytes[i] + 256).toString(16).slice(1);
    return hex;
  }

  // Le tirage au sort vaut pour toute la visite : on ne rejoue pas les dés à chaque page.
  var state = session();
  if (!state || !state.id || now - state.t > GAP) {
    state = { id: newId(), t: now, on: Math.random() * 100 < ctx.cfg.sample };
  }
  state.t = now;
  session(state);
  if (!state.on) { ctx.stop = function () { session(null); }; return; }

  var buffer = [];
  var stopRrweb = null;
  var timer = null;

  function halt(forget) {
    if (timer) clearInterval(timer);
    timer = null;
    if (stopRrweb) stopRrweb();
    stopRrweb = null;
    buffer = [];
    if (forget) session(null);
    else { state.on = false; session(state); }
  }

  function post(body, compressed, leaving) {
    var url = ctx.url + (compressed ? '?z=1' : '');
    // text/plain : requête simple, sans vérification CORS préalable.
    var blob = new Blob([body], { type: 'text/plain' });
    if (leaving && navigator.sendBeacon && blob.size < 60000 && navigator.sendBeacon(url, blob)) return;
    try {
      fetch(url, { method: 'POST', body: blob, keepalive: blob.size < 60000 }).then(function (res) {
        // 410 : le serveur n'en veut plus (enregistrements coupés, plafond atteint).
        if (res.status === 410) halt(false);
      }).catch(function () {});
    } catch (e) {}
  }

  function flush(leaving) {
    if (!buffer.length) return;
    var id = ctx.id();
    if (!id) return;
    var body = JSON.stringify({ k: ctx.key, d: id, rid: state.id, e: buffer });
    buffer = [];
    state.t = Date.now();
    session(state);
    if (leaving || !w.CompressionStream || body.length < 2000) return post(body, false, leaving);
    try {
      var stream = new Blob([body]).stream().pipeThrough(new w.CompressionStream('gzip'));
      new Response(stream).blob().then(function (zipped) { post(zipped, true, false); }, function () { post(body, false, false); });
    } catch (e) { post(body, false, false); }
  }

  stopRrweb = record({
    emit: function (event) {
      buffer.push(event);
      if (buffer.length >= 2000) flush(false);
    },
    // Ce qui est saisi dans un champ ne quitte jamais le navigateur.
    maskAllInputs: true,
    maskTextSelector: ctx.cfg.mask_text ? '*' : '[data-synalyse-mask], .synalyse-mask',
    blockSelector: '[data-synalyse-block], .synalyse-block',
    sampling: { mousemove: 50, scroll: 150, media: 800, input: 'last' },
    slimDOMOptions: { script: true, comment: true, headFavicon: true, headWhitespace: true, headMetaVerification: true },
    inlineStylesheet: true,
    recordCanvas: false,
    collectFonts: false
  });
  timer = setInterval(function () { flush(false); }, 5000);
  d.addEventListener('visibilitychange', function () { if (d.visibilityState === 'hidden') flush(true); });
  w.addEventListener('pagehide', function () { flush(true); });

  // Appelé par le traceur quand le visiteur retire son consentement ou s'oppose à la mesure.
  ctx.stop = function () { halt(true); };
})(window, document);
