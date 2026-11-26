// Import du jeu d'essai déterministe, puis comparaison du résumé avec des
// totaux recalculés indépendamment à partir du même jeu.
import test from 'node:test';
import assert from 'node:assert/strict';
import { generateVisits, siteDay, FIRST_DAY, DAYS } from '../dev/fixtures.mjs';
import { call, login, newProject, newToken } from './helpers.mjs';

const visits = generateVisits();
const toEvents = (v) => v.actions.map((a) => ({
  ts: a.ts * 1000,
  visitor: `v${v.id}`,
  ua: v.ua,
  u: a.url,
  r: v.referrer || undefined,
  ...(a.type === 'page' ? { ti: a.title } : { t: 'event', n: `${a.c} › ${a.a}`, p: { produit: a.n } }),
}));

test('le résumé par jour correspond au jeu d\'essai importé', async () => {
  const cookie = await login();
  const project = await newProject(cookie);
  const { token } = await newToken(cookie, project.id, 'ingest');

  const events = visits.flatMap(toEvents);
  for (let i = 0; i < events.length; i += 2000) {
    const res = await call('POST', `/api/projects/${project.id}/import`, { token, body: { events: events.slice(i, i + 2000) } });
    assert.equal(res.status, 200);
    assert.equal(res.body.rejected, 0);
  }

  // Référence : une visite du jeu d'essai est une session, rattachée au jour (heure de Paris) où elle commence.
  const expected = {};
  for (const v of visits) {
    const day = siteDay(v.actions[0].ts);
    const d = (expected[day] ??= { sessions: 0, pageviews: 0, events: 0 });
    d.sessions++;
    d.pageviews += v.actions.filter((a) => a.type === 'page').length;
    d.events += v.actions.length;
  }
  const bounces = visits.filter((v) => v.actions.filter((a) => a.type === 'page').length <= 1).length;
  const seconds = visits.reduce((s, v) => s + (v.actions.at(-1).ts - v.actions[0].ts), 0);

  const res = await call('GET', `/api/projects/${project.id}/stats/summary?from=2026-11-01&to=2026-11-30`, { cookie });
  assert.equal(res.status, 200);
  const s = res.body;
  assert.equal(s.sessions, visits.length);
  assert.equal(s.visitors, visits.length);
  assert.equal(s.pageviews, events.filter((e) => !e.t).length);
  assert.equal(s.events, events.length);
  assert.ok(Math.abs(s.bounce_rate - bounces / visits.length) < 1e-9);
  assert.ok(Math.abs(s.avg_session_seconds - seconds / visits.length) < 1e-6);
  assert.deepEqual(
    Object.fromEntries(s.by_day.map((d) => [d.day, { sessions: d.sessions, pageviews: d.pageviews, events: d.events }])),
    expected,
  );
  // Le jeu d'essai place des visites entre 23 h et minuit UTC : elles comptent pour le lendemain à Paris.
  assert.ok(Object.keys(expected).length === DAYS + 1 && expected['2026-11-23'], 'des visites doivent déborder sur le jour suivant');
  assert.equal(Object.keys(expected).sort()[0], FIRST_DAY);

  // Une journée seule, bornes comprises.
  const one = await call('GET', `/api/projects/${project.id}/stats/summary?from=2026-11-12&to=2026-11-12`, { cookie });
  assert.equal(one.body.sessions, expected['2026-11-12'].sessions);
  assert.equal(one.body.by_day.length, 1);
});

test('des événements espacés de plus de trente minutes ouvrent une nouvelle session', async () => {
  const cookie = await login();
  const project = await newProject(cookie);
  const { token } = await newToken(cookie, project.id, 'ingest');
  const t0 = Date.parse('2026-11-10T09:00:00Z');
  const at = (minutes) => ({ ts: t0 + minutes * 60000, visitor: 'a', ua: 'Mozilla/5.0 Chrome/130', u: 'https://demo.test/' + minutes });
  const res = await call('POST', `/api/projects/${project.id}/import`, { token, body: { events: [at(0), at(29), at(59), at(90), at(95)] } });
  assert.deepEqual(res.body, { accepted: 5, rejected: 0 });
  const s = await call('GET', `/api/projects/${project.id}/stats/summary?from=2026-11-10&to=2026-11-10`, { cookie });
  // 0, 29 et 59 minutes s'enchaînent (écarts de 29 et 30 min) ; 90 est à 31 min de 59.
  assert.equal(s.body.sessions, 2);
  assert.equal(s.body.visitors, 1);
  assert.equal(s.body.pageviews, 5);

  // Un second envoi, plus tard, prolonge la session trouvée en base et non en mémoire seulement.
  await call('POST', `/api/projects/${project.id}/import`, { token, body: { events: [at(110)] } });
  const again = await call('GET', `/api/projects/${project.id}/stats/summary?from=2026-11-10&to=2026-11-10`, { cookie });
  assert.equal(again.body.sessions, 2);
  assert.equal(again.body.pageviews, 6);
});

test('l\'import écarte les événements invalides et les domaines non autorisés', async () => {
  const cookie = await login();
  const project = await newProject(cookie);
  const { token } = await newToken(cookie, project.id, 'ingest');
  const base = { ts: Date.parse('2026-11-10T09:00:00Z'), visitor: 'a', ua: 'Mozilla/5.0 Chrome/130' };
  const res = await call('POST', `/api/projects/${project.id}/import`, { token, body: { events: [
    { ...base, u: 'https://demo.test/ok' },
    { ...base, u: 'https://autre-site.test/' },
    { ...base, u: 'javascript:alert(1)' },
    { ...base, u: 'https://demo.test/', t: 'event' },
  ] } });
  assert.deepEqual(res.body, { accepted: 1, rejected: 3 });
});
