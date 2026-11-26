// Visiteurs reconnus : rattachement de la visite anonyme, fiche, fil d'événements, rétention.
import test from 'node:test';
import assert from 'node:assert/strict';
import { BROWSER, call, eventually, login, newProject, newToken, track, dayIn } from './helpers.mjs';

const page = (path, extra = {}) => ({ t: 'pageview', u: `https://demo.test${path}`, ...extra });

test('la visite commencée en anonyme est rattachée au visiteur dès qu\'il est identifié', async () => {
  const cookie = await login();
  const project = await newProject(cookie);
  const key = project.public_key;
  const ua = `${BROWSER} Essai/${Math.random().toString(36).slice(2)}`; // une empreinte anonyme propre à ce test
  await track(key, [page('/')], { ua });
  await track(key, [page('/tarifs')], { ua });
  const today = dayIn('Europe/Paris');
  const summary = () => call('GET', `/api/projects/${project.id}/stats/summary?from=${today}&to=${today}`, { cookie });
  await eventually(summary, (r) => r.body.pageviews === 2);
  assert.equal((await call('GET', `/api/projects/${project.id}/visitors`, { cookie })).body.total, 0, 'un anonyme n\'a pas de fiche');

  // Connexion : l'application identifie l'utilisateur.
  await track(key, [page('/compte'), { t: 'event', n: 'Connexion', u: 'https://demo.test/compte' }], { ua, distinct: 'u:42', extra: { tr: { offre: 'pro', sieges: 3, secret: { a: 1 } } } });
  const after = await eventually(summary, (r) => r.body.pageviews === 3);
  assert.equal(after.body.sessions, 1, 'une seule visite, pas deux');
  assert.equal(after.body.visitors, 1);

  const list = await call('GET', `/api/projects/${project.id}/visitors`, { cookie });
  assert.equal(list.body.total, 1);
  assert.equal(list.body.users, 1);
  const [visitor] = list.body.visitors;
  assert.equal(visitor.user, '42');
  assert.equal(visitor.sessions, 1);
  assert.deepEqual(visitor.traits, { offre: 'pro', sieges: 3 }, 'seules les valeurs simples sont gardées');
  assert.match(visitor.id, /^\d+$/);

  const profile = await call('GET', `/api/projects/${project.id}/visitors/${visitor.id}`, { cookie });
  assert.equal(profile.body.recent_sessions.length, 1);
  assert.deepEqual([profile.body.recent_sessions[0].entry_path, profile.body.recent_sessions[0].pageviews, profile.body.recent_sessions[0].events], ['/', 3, 4]);

  const events = await call('GET', `/api/projects/${project.id}/visitors/${visitor.id}/events`, { cookie });
  assert.deepEqual(events.body.events.map((e) => e.kind === 'page' ? e.path : e.name).reverse(), ['/', '/tarifs', '/compte', 'Connexion'], 'les pages vues avant la connexion font partie de son fil');

  // Une propriété envoyée plus tard complète les précédentes ; la visite continue.
  await track(key, [page('/reglages')], { ua, distinct: 'u:42', extra: { tr: { offre: 'entreprise' } } });
  const updated = await eventually(() => call('GET', `/api/projects/${project.id}/visitors/${visitor.id}`, { cookie }), (r) => r.body.traits.offre === 'entreprise');
  assert.deepEqual(updated.body.traits, { offre: 'entreprise', sieges: 3 });
  assert.equal(updated.body.sessions, 1);

  // Un visiteur anonyme qui passe ensuite avec le même navigateur ouvre sa propre visite.
  await track(key, [page('/')], { ua });
  const final = await eventually(summary, (r) => r.body.sessions === 2);
  assert.equal(final.body.sessions, 2);
});

test('recherche, pagination et cloisonnement des fiches', async () => {
  const cookie = await login();
  const project = await newProject(cookie);
  const base = Date.UTC(2026, 8, 1, 10);
  const events = ['alice', 'alban', 'bob_50%'].map((user, i) => ({ ts: base + i * 3600000, visitor: `v${i}`, user, ua: BROWSER, t: 'pageview', u: 'https://demo.test/' }));
  events.push({ ts: base + 9 * 3600000, visitor: 'anonyme', ua: BROWSER, t: 'pageview', u: 'https://demo.test/' });
  const imported = await call('POST', `/api/projects/${project.id}/import`, { cookie, body: { events } });
  assert.equal(imported.body.accepted, 4);

  const all = await call('GET', `/api/projects/${project.id}/visitors`, { cookie });
  assert.deepEqual(all.body.visitors.map((v) => v.user), ['bob_50%', 'alban', 'alice'], 'du plus récemment vu au plus ancien');
  const search = await call('GET', `/api/projects/${project.id}/visitors?q=AL`, { cookie });
  assert.deepEqual(search.body.visitors.map((v) => v.user), ['alban', 'alice']);
  const literal = await call('GET', `/api/projects/${project.id}/visitors?q=${encodeURIComponent('50%')}`, { cookie });
  assert.deepEqual(literal.body.visitors.map((v) => v.user), ['bob_50%']);
  assert.equal((await call('GET', `/api/projects/${project.id}/visitors?q=${encodeURIComponent('%')}&limit=1`, { cookie })).body.visitors.length, 1);

  const first = await call('GET', `/api/projects/${project.id}/visitors?limit=2`, { cookie });
  assert.equal(first.body.visitors.length, 2);
  const second = await call('GET', `/api/projects/${project.id}/visitors?limit=2&before=${encodeURIComponent(first.body.next_before)}`, { cookie });
  assert.deepEqual(second.body.visitors.map((v) => v.user), ['alice']);
  assert.equal(second.body.next_before, null);

  const id = all.body.visitors[0].id;
  const stranger = await login('autre@synalyse.test');
  const foreign = await newProject(stranger);
  for (const path of ['/visitors', `/visitors/${id}`, `/visitors/${id}/events`, '/retention?from=2026-09-01&to=2026-09-30']) {
    assert.equal((await call('GET', `/api/projects/${project.id}${path}`, { cookie: stranger })).status, 404, path);
    assert.equal((await call('GET', `/api/projects/${project.id}${path}`)).status, 401, path);
  }
  // Le bon identifiant demandé à travers un autre projet ne donne rien.
  assert.equal((await call('GET', `/api/projects/${foreign.id}/visitors/${id}`, { cookie: stranger })).status, 404);
  assert.deepEqual((await call('GET', `/api/projects/${foreign.id}/visitors/${id}/events`, { cookie: stranger })).body.events, []);
  assert.equal((await call('GET', `/api/projects/${project.id}/visitors/pas-un-nombre`, { cookie })).status, 404);
  const reader = await newToken(cookie, project.id, 'read');
  assert.equal((await call('GET', `/api/projects/${project.id}/visitors/${id}`, { token: reader.token })).status, 200);
});

test('rétention par semaine et par jour, sur des visites datées', async () => {
  const cookie = await login();
  const project = await newProject(cookie);
  // 12 h UTC : le même jour à Paris. Semaines commençant les lundis 31 août, 7 et 14 septembre 2026.
  const at = (day) => Date.UTC(2026, 8, day, 12);
  const visits = { ana: [1, 8, 9, 15], ben: [2, 16], cleo: [6], dan: [8, 14], eve: [10, 11] };
  const events = [];
  for (const [user, days] of Object.entries(visits)) {
    for (const day of days) events.push({ ts: at(day), visitor: user, user, ua: BROWSER, t: 'pageview', u: 'https://demo.test/' });
  }
  events.push({ ts: at(8), visitor: 'passant', ua: BROWSER, t: 'pageview', u: 'https://demo.test/' }); // anonyme : hors rétention
  events.sort((a, b) => a.ts - b.ts);
  assert.equal((await call('POST', `/api/projects/${project.id}/import`, { cookie, body: { events } })).body.accepted, events.length);

  const weekly = await call('GET', `/api/projects/${project.id}/retention?from=2026-08-31&to=2026-09-13&unit=week`, { cookie });
  assert.equal(weekly.status, 200);
  assert.equal(weekly.body.unit, 'week');
  assert.equal(weekly.body.visitors, 5);
  const rows = weekly.body.cohorts.map((c) => [c.start, c.size, c.values.slice(0, 3)]);
  assert.deepEqual(rows, [
    ['2026-08-31', 3, [3, 1, 2]], // ana revient la semaine suivante ; ana et ben celle d'après
    ['2026-09-07', 2, [2, 1, 0]], // dan revient le lundi 14 ; eve ne revient pas
  ]);

  const daily = await call('GET', `/api/projects/${project.id}/retention?from=2026-09-08&to=2026-09-10&unit=day`, { cookie });
  assert.deepEqual(daily.body.cohorts.map((c) => [c.start, c.size, c.values.slice(0, 2)]), [
    ['2026-09-08', 1, [1, 0]],
    ['2026-09-09', 0, [0, 0]],
    ['2026-09-10', 1, [1, 1]],
  ]);
  // Une période plus étroite ne retient que les visiteurs arrivés pendant celle-ci.
  const narrow = await call('GET', `/api/projects/${project.id}/retention?from=2026-09-07&to=2026-09-13&unit=week`, { cookie });
  assert.deepEqual(narrow.body.cohorts.map((c) => [c.start, c.size]), [['2026-09-07', 2]]);

  assert.equal((await call('GET', `/api/projects/${project.id}/retention?from=2026-09-01&to=2026-09-30&unit=siecle`, { cookie })).status, 400);
  assert.equal((await call('GET', `/api/projects/${project.id}/retention?unit=week`, { cookie })).status, 400);
});
