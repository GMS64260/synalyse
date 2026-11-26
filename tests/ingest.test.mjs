// Point d'entrée public du traceur : ce qu'un navigateur envoie réellement.
import test from 'node:test';
import assert from 'node:assert/strict';
import { BASE, BROWSER, call, login, newProject, track, eventually, dayIn } from './helpers.mjs';

const today = dayIn('Europe/Paris');
const summary = (cookie, id) => call('GET', `/api/projects/${id}/stats/summary?from=${today}&to=${today}`, { cookie }).then((r) => r.body);
const page = (path, extra = {}) => ({ u: `https://demo.test${path}`, ti: `Page ${path}`, ...extra });

test('pages vues et événements d\'un visiteur forment une seule session', async () => {
  const cookie = await login();
  const project = await newProject(cookie);
  for (const events of [[page('/')], [page('/produits')], [{ t: 'event', n: 'Ajout au panier', u: 'https://demo.test/produits', p: { sku: 'lampe' } }]]) {
    const res = await track(project.public_key, events);
    assert.equal(res.status, 204);
    assert.equal(res.headers.get('access-control-allow-origin'), '*');
  }
  const s = await eventually(() => summary(cookie, project.id), (x) => x.events === 3);
  assert.deepEqual({ sessions: s.sessions, visitors: s.visitors, pageviews: s.pageviews, events: s.events }, { sessions: 1, visitors: 1, pageviews: 2, events: 3 });
  assert.equal(s.bounce_rate, 0);
});

test('un autre navigateur est un autre visiteur ; un identifiant stable aussi', async () => {
  const cookie = await login();
  const project = await newProject(cookie);
  await track(project.public_key, [page('/')]);
  await track(project.public_key, [page('/')], { ua: BROWSER.replace('Chrome/130', 'Chrome/131') });
  // Sur un troisième navigateur : avec le même, la visite anonyme en cours lui serait rattachée (voir people.test.mjs).
  const other = { ua: BROWSER.replace('Chrome/130', 'Chrome/132'), distinct: 'b6f1c2e0-0000-4000-8000-000000000001' };
  await track(project.public_key, [page('/')], other);
  await track(project.public_key, [page('/a')], other);
  const s = await eventually(() => summary(cookie, project.id), (x) => x.pageviews === 4);
  assert.deepEqual({ sessions: s.sessions, visitors: s.visitors, pageviews: s.pageviews }, { sessions: 3, visitors: 3, pageviews: 4 });
});

test('robots, clés inconnues et domaines étrangers sont ignorés sans erreur', async () => {
  const cookie = await login();
  const project = await newProject(cookie);
  assert.equal((await track(project.public_key, [page('/')], { ua: 'Mozilla/5.0 (compatible; Googlebot/2.1)' })).status, 204);
  assert.equal((await track('cle-inconnue', [page('/')])).status, 204);
  assert.equal((await track(project.public_key, [{ u: 'https://site-pirate.test/' }])).status, 204);
  assert.equal((await track(project.public_key, [page('/vraie')])).status, 204);
  const s = await eventually(() => summary(cookie, project.id), (x) => x.pageviews >= 1);
  await new Promise((r) => setTimeout(r, 400)); // laisse le temps à un éventuel événement indésirable d'arriver
  const after = await summary(cookie, project.id);
  assert.equal(s.pageviews, 1);
  assert.equal(after.pageviews, 1);
  assert.equal(after.sessions, 1);
});

test('les requêtes mal formées sont refusées', async () => {
  const post = (body) => fetch(`${BASE}/i`, { method: 'POST', headers: { 'user-agent': BROWSER }, body });
  assert.equal((await post('pas du json')).status, 400);
  assert.equal((await post(JSON.stringify({ k: 'x', e: [] }))).status, 400);
  assert.equal((await post(JSON.stringify({ k: 'x', e: Array(51).fill({ u: 'https://demo.test/' }) }))).status, 400);
  assert.equal((await post('x'.repeat(70 * 1024))).status, 413);
  const preflight = await fetch(`${BASE}/i`, { method: 'OPTIONS' });
  assert.equal(preflight.status, 204);
  assert.match(preflight.headers.get('access-control-allow-methods'), /POST/);
});

test('le traceur est servi avec le bon type et peut être mis en cache', async () => {
  const res = await fetch(`${BASE}/c.js`);
  assert.equal(res.status, 200);
  assert.match(res.headers.get('content-type'), /javascript/);
  assert.match(res.headers.get('cache-control'), /max-age/);
  const js = await res.text();
  assert.ok(js.includes('sendBeacon') && js.length < 8000, 'le traceur reste petit');
  new Function(js); // syntaxe valide
});
