// Synalyse — lecteur d'enregistrements. Rejoue avec rrweb (MIT) les événements servis par l'API.
(function () {
  'use strict';
  var stage = document.getElementById('stage');
  var status = document.getElementById('status');
  var controls = document.getElementById('controls');
  var toggle = document.getElementById('toggle');
  var clock = document.getElementById('clock');
  var seek = document.getElementById('seek');
  var marks = document.getElementById('marks');
  var speed = document.getElementById('speed');
  var skip = document.getElementById('skip');

  var parts = location.hash.slice(1).split('/');
  var project = parts[0];
  var recording = parts[1];
  var replayer = null;
  var total = 0;
  var playing = false;
  var size = { width: 0, height: 0 };
  var frame = 0;

  function fail(message) {
    status.textContent = message;
    status.hidden = false;
    controls.hidden = true;
  }

  function time(ms) {
    var s = Math.max(0, Math.round(ms / 1000));
    var m = Math.floor(s / 60);
    var rest = String(s % 60).padStart(2, '0');
    return m >= 60 ? Math.floor(m / 60) + ':' + String(m % 60).padStart(2, '0') + ':' + rest : m + ':' + rest;
  }

  function fit() {
    if (!replayer || !size.width || !size.height) return;
    var scale = Math.min(stage.clientWidth / size.width, stage.clientHeight / size.height, 1);
    var wrapper = replayer.wrapper;
    wrapper.style.transform = 'scale(' + scale + ')';
    wrapper.style.left = Math.max(0, (stage.clientWidth - size.width * scale) / 2) + 'px';
    wrapper.style.top = Math.max(0, (stage.clientHeight - size.height * scale) / 2) + 'px';
  }

  function current() {
    return Math.min(total, Math.max(0, replayer.getCurrentTime()));
  }

  function show(ms) {
    seek.value = String(Math.round(ms));
    seek.setAttribute('aria-valuetext', time(ms) + ' sur ' + time(total));
    clock.textContent = time(ms) + ' / ' + time(total);
  }

  function tick() {
    if (!playing) return;
    show(current());
    frame = requestAnimationFrame(tick);
  }

  function setPlaying(on, at) {
    playing = on;
    cancelAnimationFrame(frame);
    toggle.textContent = on ? 'Pause' : 'Lire';
    if (on) { replayer.play(at); tick(); } else { replayer.pause(at); show(at); }
  }

  function parse(text) {
    var events = [];
    text.split('\n').forEach(function (line) {
      if (!line) return;
      // Un morceau abîmé ne doit pas empêcher de rejouer les autres.
      try { Array.prototype.push.apply(events, JSON.parse(line)); } catch (e) {}
    });
    return events.sort(function (a, b) { return a.timestamp - b.timestamp; });
  }

  function start(events) {
    var first = events.findIndex(function (e) { return e.type === 4; });
    if (first < 0 || events.length - first < 2) return fail('Cet enregistrement est trop court pour être rejoué.');
    events = events.slice(first);
    var Replayer = window.rrwebReplay && window.rrwebReplay.Replayer;
    if (!Replayer) return fail('Le lecteur n\'a pas pu être chargé.');

    status.hidden = true;
    replayer = new Replayer(events, {
      root: stage,
      speed: Number(speed.value),
      skipInactive: skip.checked,
      mouseTail: { strokeStyle: '#ab4325', duration: 400 },
      showWarning: false,
      UNSAFE_replayCanvas: false
    });
    total = replayer.getMetaData().totalTime;
    seek.max = String(Math.max(1, Math.round(total)));

    var origin = events[0].timestamp;
    events.forEach(function (event) {
      var page = event.type === 4;
      var click = event.type === 3 && event.data && event.data.source === 2 && event.data.type === 2;
      if (!page && !click) return;
      var mark = document.createElement('span');
      mark.className = page ? 'mark page' : 'mark';
      mark.style.left = (total ? ((event.timestamp - origin) / total) * 100 : 0) + '%';
      marks.appendChild(mark);
    });

    replayer.on('resize', function (dimension) { size = dimension; fit(); });
    replayer.on('finish', function () { playing = false; cancelAnimationFrame(frame); toggle.textContent = 'Revoir'; show(total); });
    window.addEventListener('resize', fit);

    toggle.addEventListener('click', function () {
      var at = current();
      setPlaying(!playing, !playing && at >= total - 50 ? 0 : at);
    });
    seek.addEventListener('input', function () { setPlaying(playing, Number(seek.value)); });
    speed.addEventListener('change', function () { replayer.setConfig({ speed: Number(speed.value) }); });
    skip.addEventListener('change', function () { replayer.setConfig({ skipInactive: skip.checked }); });
    document.addEventListener('keydown', function (event) {
      if (event.target !== document.body && event.target !== toggle) return;
      if (event.key === ' ' && event.target === document.body) { event.preventDefault(); toggle.click(); }
      if (event.key === 'ArrowRight' || event.key === 'ArrowLeft') {
        setPlaying(playing, Math.min(total, Math.max(0, current() + (event.key === 'ArrowRight' ? 5000 : -5000))));
      }
    });

    controls.hidden = false;
    setPlaying(true, 0);
  }

  if (!/^\d+$/.test(project || '') || !/^[0-9a-f-]{16,40}$/i.test(recording || '')) return fail('Enregistrement introuvable.');
  fetch('/api/projects/' + project + '/recordings/' + recording + '/events', { credentials: 'same-origin' })
    .then(function (res) {
      if (res.status === 401) throw new Error('Reconnectez-vous pour lire cet enregistrement.');
      if (!res.ok) throw new Error('Enregistrement introuvable.');
      return res.text();
    })
    .then(function (text) { start(parse(text)); })
    .catch(function (error) { fail(error.message || 'Lecture impossible.'); });
})();
