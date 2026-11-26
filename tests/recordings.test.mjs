// Enregistrements de session : consentement exigé, réglages, stockage, lecture, cloisonnement, suppression.
import test from 'node:test';
import assert from 'node:assert/strict';
import { gzipSync } from 'node:zlib';
import { BASE, BROWSER, call, login, newProject, newToken, probe } from './helpers.mjs';

const rid = () => [...crypto.getRandomValues(new Uint8Array(16))].map((b) => b.toString(16).padStart(2, '0')).join('');

function sample(start, { host = 'demo.test', path = '/tarifs' } = {}) {
  return [
    { type: 4, timestamp: start, data: { href: `https://${host}${path}`, width: 1280, height: 720 } },
    { type: 2, timestamp: start + 10, data: { node: { type: 0, id: 1, childNodes: [] }, initialOffset: { left: 0, top: 0 } } },
    { type: 3, timestamp: start + 1000, data: { source: 1, positions: [{ x: 5, y: 5, id: 1, timeOffset: 0 }] } },
    { type: 3, timestamp: start + 2500, data: { source: 2, type: 2, id: 1, x: 10, y: 20 } },
  ];
}

function send(key, id, events, { distinct = 'visiteur-consentant-1', gzip = false, ua = BROWSER } = {}) {
  const body = typeof events === 'string' ? events : JSON.stringify({ d: distinct, e: events });
  return fetch(`${BASE}/r?k=${encodeURIComponent(key)}&rid=${encodeURIComponent(id)}${gzip ? '&z=1' : ''}`, {
    method: 'POST',
    headers: { 'content-type': 'text/plain', 'user-agent': ua },
    body: gzip ? gzipSync(body) : body,
  });
}

const enable = (cookie, project, recording = {}) =>
  call('PATCH', `/api/projects/${project.id}`, { cookie, body: { recording: { enabled: true, ...recording } } });

test('les enregistrements sont coupés par défaut et la configuration publique le dit', async () => {
  const cookie = await login();
  const project = await newProject(cookie);
  assert.deepEqual(project.recording, { enabled: false, sample: 100, retention_days: 30, mask_text: false, max_mb: 500 });

  const config = await fetch(`${BASE}/r/config?k=${project.public_key}`);
  assert.equal(config.headers.get('access-control-allow-origin'), '*');
  assert.deepEqual(await config.json(), { record: false, sample: 100, mask_text: false });
  assert.deepEqual(await (await fetch(`${BASE}/r/config?k=cle-inconnue`)).json(), { record: false, sample: 100, mask_text: false });

  assert.equal((await send(project.public_key, rid(), sample(Date.now()))).status, 410, 'le navigateur doit s\'arrêter');
  const list = await call('GET', `/api/projects/${project.id}/recordings`, { cookie });
  assert.equal(list.body.total, 0);
});

test('les réglages sont bornés et réservés aux administrateurs', async () => {
  const cookie = await login();
  const project = await newProject(cookie);
  const res = await enable(cookie, project, { sample: 250, retention_days: 0, max_mb: 1, mask_text: true });
  assert.equal(res.status, 200);
  assert.deepEqual(res.body.recording, { enabled: true, sample: 100, retention_days: 1, mask_text: true, max_mb: 10 });
  assert.equal(res.body.name, project.name, 'les autres champs ne bougent pas');
  const config = await (await fetch(`${BASE}/r/config?k=${project.public_key}`)).json();
  assert.deepEqual(config, { record: true, sample: 100, mask_text: true });

  const reader = await newToken(cookie, project.id, 'read');
  assert.equal((await call('PATCH', `/api/projects/${project.id}`, { token: reader.token, body: { recording: { enabled: false } } })).status, 403);
});

test('un enregistrement est stocké, résumé et relu tel quel', async () => {
  const cookie = await login();
  const project = await newProject(cookie);
  await enable(cookie, project);
  const id = rid();
  const start = Date.now() - 60000;
  const first = sample(start);
  const second = [
    { type: 3, timestamp: start + 4000, data: { source: 3, id: 1, x: 0, y: 300 } },
    { type: 3, timestamp: start + 4500, data: { source: 0, texts: [{ id: 2, value: 'é'.repeat(5000) }], attributes: [], removes: [], adds: [] } },
    { type: 4, timestamp: start + 9000, data: { href: 'https://demo.test/contact', width: 1280, height: 720 } },
    { type: 2, timestamp: start + 9010, data: { node: { type: 0, id: 1, childNodes: [] }, initialOffset: { left: 0, top: 0 } } },
  ];
  assert.equal((await send(project.public_key, id, first)).status, 204);
  assert.equal((await send(project.public_key, id, second, { gzip: true })).status, 204, 'morceau compressé');

  const list = await call('GET', `/api/projects/${project.id}/recordings`, { cookie });
  assert.equal(list.body.total, 1);
  const [rec] = list.body.recordings;
  assert.equal(rec.id, id);
  assert.deepEqual([rec.events, rec.pages, rec.clicks], [8, 2, 1]);
  assert.deepEqual([rec.entry_host, rec.entry_path, rec.view_w, rec.view_h], ['demo.test', '/tarifs', 1280, 720]);
  assert.equal(rec.active_ms, 3000, 'le temps actif se compte aussi entre deux envois');
  assert.equal(rec.browser, 'Chrome');
  assert.ok(rec.bytes > 0 && rec.bytes === list.body.stored_bytes);
  assert.match(rec.visitor, /^\d+$/);

  const one = await call('GET', `/api/projects/${project.id}/recordings/${id}`, { cookie });
  assert.equal(one.body.id, id);

  const res = await fetch(`${BASE}/api/projects/${project.id}/recordings/${id}/events`, { headers: { cookie } });
  assert.equal(res.status, 200);
  assert.match(res.headers.get('content-type'), /ndjson/);
  const lines = (await res.text()).split('\n').filter(Boolean).map((line) => JSON.parse(line));
  assert.deepEqual(lines, [first, second], 'les événements reviennent intacts, un morceau par ligne');

  // Filtres de la liste.
  const active = await call('GET', `/api/projects/${project.id}/recordings?min_active=10`, { cookie });
  assert.equal(active.body.recordings.length, 0);
  const mine = await call('GET', `/api/projects/${project.id}/recordings?visitor=${rec.visitor}`, { cookie });
  assert.equal(mine.body.recordings.length, 1);
  const other = await call('GET', `/api/projects/${project.id}/recordings?visitor=12345`, { cookie });
  assert.equal(other.body.recordings.length, 0);
});

test('rien n\'est enregistré sans consentement, pour un robot, un autre site ou un envoi invalide', async () => {
  const cookie = await login();
  const project = await newProject(cookie);
  await enable(cookie, project);
  const key = project.public_key;
  const now = Date.now();

  assert.equal((await send(key, rid(), sample(now), { distinct: null })).status, 410, 'visiteur sans identifiant stable');
  assert.equal((await send(key, rid(), sample(now), { ua: 'Googlebot/2.1' })).status, 410);
  assert.equal((await send(key, rid(), sample(now, { host: 'autre-site.test' }))).status, 204, 'ignoré sans le dire');
  assert.equal((await send(key, rid(), sample(now).slice(2))).status, 410, 'pas d\'état de page au départ : injouable');
  assert.equal((await send(key, '../../etc/passwd-00000000', sample(now))).status, 400);
  assert.equal((await send(key, rid(), [])).status, 400);
  assert.equal((await send(key, rid(), [{ type: 42, timestamp: now }])).status, 400);
  assert.equal((await send(key, rid(), 'pas du json')).status, 400);
  assert.equal((await fetch(`${BASE}/r?k=${key}&rid=${rid()}&z=1`, { method: 'POST', headers: { 'user-agent': BROWSER }, body: 'pas du gzip' })).status, 400);
  assert.equal((await send(key, rid(), 'x'.repeat(3 << 20))).status, 413);
  assert.equal((await send(key, rid(), [{ type: 4, timestamp: now, data: { width: 10, height: 10 } }])).status, 400, 'page sans adresse');
  // Site inconnu : écarté avant toute lecture du corps, même illisible.
  assert.equal((await send('cle-inconnue', rid(), 'pas du json')).status, 410);

  // Un autre visiteur ne peut pas écrire dans l'enregistrement de quelqu'un.
  const id = rid();
  assert.equal((await send(key, id, sample(now))).status, 204);
  assert.equal((await send(key, id, sample(now), { distinct: 'un-autre-visiteur' })).status, 410);

  const list = await call('GET', `/api/projects/${project.id}/recordings`, { cookie });
  assert.equal(list.body.total, 1);
  assert.equal(list.body.recordings[0].events, 4);
});

test('un réglage modifié seul laisse les autres en place', async () => {
  const cookie = await login();
  const project = await newProject(cookie);
  await enable(cookie, project, { mask_text: true, retention_days: 365, max_mb: 2000, sample: 40 });
  const res = await call('PATCH', `/api/projects/${project.id}`, { cookie, body: { recording: { enabled: false } } });
  assert.deepEqual(res.body.recording, { enabled: false, sample: 40, retention_days: 365, mask_text: true, max_mb: 2000 });
  const renamed = await call('PATCH', `/api/projects/${project.id}`, { cookie, body: { name: 'Autre nom' } });
  assert.deepEqual(renamed.body.recording, res.body.recording);
});

test('un utilisateur connecté à identifiant court est enregistré, et un envoi sur plusieurs lignes reste lisible', async () => {
  const cookie = await login();
  const project = await newProject(cookie);
  await enable(cookie, project);
  const id = rid();
  const events = sample(Date.now());
  const pretty = JSON.stringify({ d: 'u:42', e: events }, null, 2);
  assert.equal((await send(project.public_key, id, pretty)).status, 204);
  const res = await fetch(`${BASE}/api/projects/${project.id}/recordings/${id}/events`, { headers: { cookie } });
  const lines = (await res.text()).split('\n').filter(Boolean);
  assert.equal(lines.length, 1, 'un morceau, une ligne');
  assert.deepEqual(JSON.parse(lines[0]), events);
});

test('le plafond de stockage du site arrête les nouveaux envois', async () => {
  const cookie = await login();
  const project = await newProject(cookie);
  await enable(cookie, project, { max_mb: 10 });
  const key = project.public_key;
  // Du texte aléatoire, donc incompressible : environ 1,45 Mo stockés par envoi.
  const noise = () => Buffer.from(crypto.getRandomValues(new Uint8Array(60000))).toString('base64');
  const heavy = (start) => [...sample(start), ...Array.from({ length: 24 }, (_, i) => ({ type: 3, timestamp: start + 3000 + i, data: { source: 0, texts: [{ id: 2, value: noise() }], attributes: [], removes: [], adds: [] } }))];
  let stopped = 0;
  for (let i = 0; i < 12 && !stopped; i++) {
    const status = (await send(key, rid(), heavy(Date.now()))).status;
    if (status === 410) stopped = i; else assert.equal(status, 204);
  }
  assert.ok(stopped >= 7 && stopped <= 9, `arrêt autour de 10 Mo, obtenu après ${stopped} envois`);
  const list = await call('GET', `/api/projects/${project.id}/recordings`, { cookie });
  assert.ok(list.body.stored_bytes >= 10 << 20 && list.body.stored_bytes < 12 << 20);
  // Supprimer libère de la place : les envois reprennent.
  for (const rec of list.body.recordings.slice(0, 3)) await call('DELETE', `/api/projects/${project.id}/recordings/${rec.id}`, { cookie });
  assert.equal((await send(key, rid(), sample(Date.now()))).status, 204);
});

test('les enregistrements d\'un site ne sont visibles que de son organisation', async () => {
  const cookie = await login();
  const project = await newProject(cookie);
  await enable(cookie, project);
  const id = rid();
  assert.equal((await send(project.public_key, id, sample(Date.now()))).status, 204);

  const stranger = await login('autre@synalyse.test');
  const foreign = await newProject(stranger);
  for (const path of ['', `/${id}`, `/${id}/events`]) {
    assert.equal((await call('GET', `/api/projects/${project.id}/recordings${path}`, { cookie: stranger })).status, 404, path);
    assert.equal((await call('GET', `/api/projects/${project.id}/recordings${path}`)).status, 401, path);
  }
  // Le bon identifiant demandé à travers un autre projet ne donne rien.
  assert.equal((await call('GET', `/api/projects/${foreign.id}/recordings/${id}`, { cookie: stranger })).status, 404);
  assert.equal((await call('GET', `/api/projects/${foreign.id}/recordings/${id}/events`, { cookie: stranger })).status, 404);
  assert.equal((await call('DELETE', `/api/projects/${project.id}/recordings/${id}`, { cookie: stranger })).status, 404);

  const reader = await newToken(cookie, project.id, 'read');
  assert.equal((await call('GET', `/api/projects/${project.id}/recordings/${id}`, { token: reader.token })).status, 200);
  assert.equal((await call('DELETE', `/api/projects/${project.id}/recordings/${id}`, { token: reader.token })).status, 403);

  assert.equal((await call('DELETE', `/api/projects/${project.id}/recordings/${id}`, { cookie })).status, 200);
  assert.equal((await call('GET', `/api/projects/${project.id}/recordings/${id}`, { cookie })).status, 404);
  assert.equal((await call('GET', `/api/projects/${project.id}/recordings/${id}/events`, { cookie })).status, 404);
  assert.equal((await call('GET', `/api/projects/${project.id}/recordings`, { cookie })).body.total, 0);
});

test('le lecteur est une page à part, seule à pouvoir charger des contenus du site rejoué', async () => {
  const res = await fetch(`${BASE}/player`);
  assert.equal(res.status, 200);
  const csp = res.headers.get('content-security-policy');
  assert.match(csp, /script-src 'self';/, 'aucun script hors des nôtres');
  assert.match(csp, /frame-ancestors 'self'/);
  assert.match(csp, /img-src[^;]*https:/);
  assert.ok(!/http:/.test(csp), 'pas de contenu en clair : le réseau local de la personne qui regarde reste hors d\'atteinte');
  assert.match(csp, /form-action 'none'/);
  assert.match(csp, /base-uri 'none'/);
  const html = await res.text();
  assert.ok(!/<script(?![^>]*src=)/.test(html), 'aucun script en ligne');
  const app = (await probe('/')).headers.get('content-security-policy');
  assert.match(app, /frame-src 'self'/);
  assert.ok(!/img-src[^;]*https:/.test(app), 'l\'interface reste fermée aux contenus tiers');
  for (const name of ['player.js', 'player.css', 'replay.min.js', 'replay.min.css']) {
    assert.equal((await probe(`/assets/${name}`)).status, 200, name);
  }
  assert.equal((await probe('/assets/player.html')).status, 404);

  const recorder = await fetch(`${BASE}/r.js`);
  assert.equal(recorder.status, 200);
  assert.match(recorder.headers.get('content-type'), /javascript/);
  const code = await recorder.text();
  assert.match(code, /maskAllInputs: true/);
  assert.ok(code.length > 50000, 'rrweb est embarqué');
  // Le traceur de base ne contient pas l'enregistreur : il n'est téléchargé qu'après consentement.
  assert.ok((await (await fetch(`${BASE}/c.js`)).text()).length < 8000);
});
