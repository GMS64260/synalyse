/*! Synalyse — traceur. AGPL-3.0-or-later. https://github.com/GMS64260/synalyse */
(function (w, d) {
  'use strict';
  var script = d.currentScript;
  if (!script) return;
  var key = script.getAttribute('data-key');
  if (!key) return;
  var origin = new URL(script.src, location.href).origin;
  var api = script.getAttribute('data-api') || origin + '/i';
  var manual = script.hasAttribute('data-manual');

  function store(name, value) {
    // Lecture si value est undefined, suppression si null, écriture sinon. Jamais d'exception.
    try {
      if (value === undefined) return w.localStorage.getItem(name);
      if (value === null) w.localStorage.removeItem(name);
      else w.localStorage.setItem(name, value);
    } catch (e) {}
    return null;
  }

  // Sans consentement, rien n'est écrit dans le navigateur : ni cookie, ni stockage local.
  // L'identifiant stable n'existe qu'après synalyse('consent', true) ou synalyse('identify', id).
  var distinct = store('synalyse_id');
  var lastUrl = null;

  function send(event) {
    if (store('synalyse_optout') === '1') return;
    var body = JSON.stringify({
      k: key,
      d: distinct || undefined,
      sw: w.screen && w.screen.width,
      l: navigator.language,
      e: [event]
    });
    // sendBeacon envoie en text/plain : pas de requête préalable CORS, et l'envoi survit au changement de page.
    if (navigator.sendBeacon && navigator.sendBeacon(api, body)) return;
    try {
      fetch(api, { method: 'POST', body: body, keepalive: true }).catch(function () {});
    } catch (e) {}
  }

  function page() {
    var url = location.href;
    if (url === lastUrl) return;
    var referrer = lastUrl === null ? d.referrer : lastUrl;
    lastUrl = url;
    send({ t: 'pageview', u: url, r: referrer || undefined, ti: d.title || undefined });
  }

  // Enregistrement de session : seulement pour un visiteur qui porte un identifiant stable
  // (consentement donné ou utilisateur connecté), et si le projet l'a activé. L'enregistreur
  // est un fichier à part, chargé à ce moment-là : les autres visiteurs ne le téléchargent jamais.
  var attempt = 0;
  function identity() {
    return store('synalyse_optout') === '1' ? null : distinct;
  }
  function record() {
    if (w.synalyse.rec || !identity() || !w.fetch) return;
    // Chaque tentative porte un numéro : une réponse arrivée après un retrait du consentement est ignorée.
    var mine = ++attempt;
    fetch(origin + '/r/config?k=' + encodeURIComponent(key)).then(function (res) { return res.json(); }).then(function (cfg) {
      if (mine !== attempt || w.synalyse.rec || !cfg || !cfg.record || !identity()) return;
      w.synalyse.rec = { key: key, url: origin + '/r', cfg: cfg, id: identity };
      var el = d.createElement('script');
      el.src = origin + '/r.js';
      el.async = true;
      d.head.appendChild(el);
    }).catch(function () {});
  }
  function stopRecording() {
    attempt++;
    var rec = w.synalyse.rec;
    w.synalyse.rec = null;
    if (rec) {
      if (rec.stop) rec.stop();
      // Enregistreur encore en cours de chargement : il trouvera `stop` posé et ne démarrera pas.
      else rec.stop = function () {};
    }
  }
  // Consentement retiré ou opposition exprimée dans un autre onglet : cet onglet suit.
  w.addEventListener('storage', function (event) {
    if (event.key === 'synalyse_id') { distinct = event.newValue; if (distinct) record(); else stopRecording(); }
    if (event.key === 'synalyse_optout') { if (event.newValue === '1') stopRecording(); else record(); }
  });

  function newId() {
    if (w.crypto && w.crypto.randomUUID) return w.crypto.randomUUID();
    return 'x' + Date.now().toString(36) + Math.random().toString(36).slice(2) + Math.random().toString(36).slice(2);
  }

  var commands = {
    page: page,
    event: function (name, props) {
      if (name) send({ t: 'event', n: String(name), u: location.href, ti: d.title || undefined, p: props || undefined });
    },
    // Consentement du visiteur aux fonctions qui le reconnaissent d'une visite à l'autre.
    consent: function (granted) {
      if (granted) {
        if (!distinct) distinct = newId();
        store('synalyse_id', distinct);
        record();
      } else {
        distinct = null;
        store('synalyse_id', null);
        stopRecording();
      }
    },
    // Utilisateur connecté d'une application : identifiant stable fourni par le site.
    identify: function (id) {
      if (!id) return;
      distinct = 'u:' + String(id);
      store('synalyse_id', distinct);
      record();
    },
    // Opposition du visiteur à toute mesure.
    optout: function (on) {
      store('synalyse_optout', on === false ? null : '1');
      if (on === false) record(); else stopRecording();
    }
  };

  var queued = (w.synalyse && w.synalyse.q) || [];
  w.synalyse = function (name) {
    var fn = commands[name];
    if (fn) return fn.apply(null, Array.prototype.slice.call(arguments, 1));
  };
  for (var i = 0; i < queued.length; i++) w.synalyse.apply(null, queued[i]);
  record();

  if (!manual) {
    // Applications à page unique : chaque changement d'URL compte comme une page vue.
    ['pushState', 'replaceState'].forEach(function (method) {
      var original = history[method];
      if (!original) return;
      history[method] = function () {
        var result = original.apply(this, arguments);
        page();
        return result;
      };
    });
    w.addEventListener('popstate', page);
    if (d.visibilityState === 'prerender') {
      d.addEventListener('visibilitychange', function once() {
        if (d.visibilityState !== 'prerender') { d.removeEventListener('visibilitychange', once); page(); }
      });
    } else {
      page();
    }
  }
})(window, document);
