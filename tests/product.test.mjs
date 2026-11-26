// Entonnoirs et parcours, comparés à des implémentations de référence indépendantes
// appliquées au même jeu d'essai.
import test, { before } from 'node:test';
import assert from 'node:assert/strict';
import { generateVisits, siteDay, SITE } from '../dev/fixtures.mjs';
import { call, login, newProject, newToken } from './helpers.mjs';

const RANGE = 'from=2026-11-01&to=2026-11-30';
const visits = generateVisits();
const pathOf = (url) => (url.slice(SITE.url.length) || '/').replace(/[?#].*$/, '');
const eventName = (a) => `${a.c} › ${a.a}`;

const STEPS = [
  { name: 'Accueil', kind: 'page', match: 'equals', pattern: '/' },
  { name: 'Fiche produit', kind: 'page', match: 'starts_with', pattern: '/produits/' },
  { name: 'Ajout au panier', kind: 'event', match: 'equals', pattern: 'Panier › Ajout' },
  { name: 'Panier', kind: 'page', match: 'regex', pattern: '^/panier$' },
  { name: 'Commande passée', kind: 'page', match: 'ends_with', pattern: '/merci' },
];

let cookie; let project; let other;
const url = (path) => `/api/projects/${project.id}/${path}`;

before(async () => {
  cookie = await login();
  other = await login('autre@synalyse.test');
  project = await newProject(cookie);
  const { token } = await newToken(cookie, project.id, 'ingest');
  const events = visits.flatMap((v) => v.actions.map((a) => ({
    ts: a.ts * 1000, visitor: `v${v.id}`, ua: v.ua, u: a.url, r: v.referrer || undefined,
    ...(a.type === 'page' ? { ti: a.title } : { t: 'event', n: eventName(a) }),
  })));
  for (let i = 0; i < events.length; i += 2000) {
    const res = await call('POST', url('import'), { token, body: { events: events.slice(i, i + 2000) } });
    assert.equal(res.body.rejected, 0);
  }
});

function matches(step, a) {
  if ((step.kind === 'event') !== (a.type === 'event')) return false;
  const value = a.type === 'event' ? eventName(a) : pathOf(a.url);
  switch (step.match) {
    case 'equals': return value === step.pattern;
    case 'starts_with': return value.startsWith(step.pattern);
    case 'contains': return value.includes(step.pattern);
    case 'ends_with': return value.endsWith(step.pattern);
    case 'regex': return new RegExp(step.pattern, 'u').test(value);
  }
}

function funnelReference(list, steps) {
  const reached = steps.map(() => 0);
  const drops = steps.map(() => ({ exit: 0, next: {} }));
  const gaps = steps.map(() => []);
  for (const v of list) {
    let i = 0; let last = 0; let nextPage; let waiting = false;
    for (const a of v.actions) {
      if (i < steps.length && matches(steps[i], a)) {
        reached[i]++;
        if (i > 0) gaps[i].push(a.ts - last);
        last = a.ts; i++; waiting = true; nextPage = undefined;
      } else if (waiting && a.type === 'page') { nextPage = pathOf(a.url); waiting = false; }
    }
    if (i > 0 && i < steps.length) {
      if (nextPage === undefined) drops[i - 1].exit++;
      else drops[i - 1].next[nextPage] = (drops[i - 1].next[nextPage] || 0) + 1;
    }
  }
  const median = (xs) => {
    if (!xs.length) return null;
    const s = [...xs].sort((a, b) => a - b); const m = s.length >> 1;
    return s.length % 2 ? s[m] : Math.ceil((s[m - 1] + s[m]) / 2);
  };
  return { reached, drops, medians: gaps.map(median) };
}

test('entonnoir à la volée : visites, abandons, suites et délais médians', async () => {
  const ref = funnelReference(visits, STEPS);
  const res = await call('POST', url(`funnels/preview?${RANGE}`), { cookie, body: { steps: STEPS } });
  assert.equal(res.status, 200);
  const r = res.body;
  assert.equal(r.sessions_in_period, visits.length);
  assert.deepEqual(r.steps.map((s) => s.sessions), ref.reached);
  assert.ok(ref.reached[4] > 0 && ref.reached[0] > ref.reached[4], 'le jeu d\'essai doit contenir des conversions et des abandons');
  assert.equal(r.entered, ref.reached[0]);
  assert.equal(r.converted, ref.reached[4]);
  assert.ok(Math.abs(r.conversion_rate - ref.reached[4] / ref.reached[0]) < 1e-12);
  assert.deepEqual(r.steps.map((s) => s.median_seconds_from_previous), ref.medians);
  r.steps.forEach((s, i) => {
    assert.equal(s.dropped, i < 4 ? ref.reached[i] - ref.reached[i + 1] : 0);
    const exit = s.next_after_drop.find((n) => n.exit);
    const top = Object.values(ref.drops[i].next).concat(ref.drops[i].exit).sort((a, b) => b - a).slice(0, 5);
    assert.deepEqual(s.next_after_drop.map((n) => n.sessions), top.filter((n) => n > 0), `suites après l'étape ${i + 1}`);
    if (exit) assert.equal(exit.sessions, ref.drops[i].exit);
    for (const n of s.next_after_drop.filter((x) => !x.exit)) assert.equal(n.sessions, ref.drops[i].next[n.path]);
    assert.ok(s.dropped_sessions.every((id) => typeof id === 'string' && /^\d+$/.test(id)));
  });
});

test('la période et les filtres restreignent l\'entonnoir', async () => {
  const day = '2026-11-12';
  const ofDay = visits.filter((v) => siteDay(v.actions[0].ts) === day);
  const one = await call('POST', url(`funnels/preview?from=${day}&to=${day}`), { cookie, body: { steps: STEPS } });
  assert.equal(one.body.sessions_in_period, ofDay.length);
  assert.deepEqual(one.body.steps.map((s) => s.sessions), funnelReference(ofDay, STEPS).reached);

  const firefox = visits.filter((v) => v.ua.includes('Firefox'));
  const filtered = await call('POST', url(`funnels/preview?browser=Firefox&${RANGE}`), { cookie, body: { steps: STEPS } });
  assert.equal(filtered.body.sessions_in_period, firefox.length);
  assert.deepEqual(filtered.body.steps.map((s) => s.sessions), funnelReference(firefox, STEPS).reached);
});

test('un entonnoir s\'enregistre, se relit, se modifie et se supprime par son identifiant', async () => {
  const name = 'Achat « test » <b>&';
  const created = await call('POST', url('funnels'), { cookie, body: { name, steps: STEPS } });
  assert.equal(created.status, 200);
  const id = created.body.id;
  assert.equal(created.body.name, name, 'le nom est stocké tel que saisi');
  assert.deepEqual(created.body.steps.map((s) => s.name), STEPS.map((s) => s.name));

  const report = await call('GET', url(`funnels/${id}/report?${RANGE}`), { cookie });
  const preview = await call('POST', url(`funnels/preview?${RANGE}`), { cookie, body: { steps: STEPS } });
  assert.equal(report.body.name, name);
  assert.deepEqual(report.body.steps, preview.body.steps);

  const updated = await call('PUT', url(`funnels/${id}`), { cookie, body: { name: 'Achat', steps: STEPS.slice(0, 2) } });
  assert.equal(updated.body.id, id);
  assert.equal(updated.body.steps.length, 2);
  assert.ok((await call('GET', url('funnels'), { cookie })).body.funnels.some((f) => f.id === id && f.name === 'Achat'));

  // Une autre organisation ne voit, ne lit, ne modifie ni ne supprime cet entonnoir.
  assert.equal((await call('GET', url('funnels'), { cookie: other })).status, 404);
  assert.equal((await call('GET', url(`funnels/${id}/report?${RANGE}`), { cookie: other })).status, 404);
  assert.equal((await call('PUT', url(`funnels/${id}`), { cookie: other, body: { name: 'x', steps: STEPS } })).status, 404);
  assert.equal((await call('DELETE', url(`funnels/${id}`), { cookie: other })).status, 404);
  // Un entonnoir ne s'adresse pas par un autre projet du même compte.
  const second = await newProject(cookie);
  assert.equal((await call('GET', `/api/projects/${second.id}/funnels/${id}/report?${RANGE}`, { cookie })).status, 404);
  assert.equal((await call('DELETE', `/api/projects/${second.id}/funnels/${id}`, { cookie })).status, 404);

  // Un jeton de lecture lit le rapport mais ne modifie rien.
  const read = await newToken(cookie, project.id, 'read');
  assert.equal((await call('GET', url(`funnels/${id}/report?${RANGE}`), { token: read.token })).status, 200);
  assert.equal((await call('DELETE', url(`funnels/${id}`), { token: read.token })).status, 403);
  assert.equal((await call('POST', url('funnels'), { token: read.token, body: { name: 'x', steps: STEPS } })).status, 403);

  assert.equal((await call('DELETE', url(`funnels/${id}`), { cookie })).status, 200);
  assert.equal((await call('GET', url(`funnels/${id}/report?${RANGE}`), { cookie })).status, 404);
  assert.equal((await call('DELETE', url(`funnels/${id}`), { cookie })).status, 404);
  assert.ok(!(await call('GET', url('funnels'), { cookie })).body.funnels.some((f) => f.id === id));
});

test('les étapes invalides sont refusées avec un message', async () => {
  const cases = [
    [STEPS.slice(0, 1), /au moins deux/],
    [[STEPS[0], { kind: 'page', match: 'regex', pattern: '(' }], /expression régulière/],
    [[STEPS[0], { kind: 'page', match: 'equals', pattern: '  ' }], /motif est vide/],
    [Array(13).fill(STEPS[0]), /au plus/],
  ];
  for (const [steps, expected] of cases) {
    const res = await call('POST', url(`funnels/preview?${RANGE}`), { cookie, body: { steps } });
    assert.equal(res.status, 400);
    assert.match(res.body.error.message, expected);
  }
  assert.equal((await call('POST', url('funnels'), { cookie, body: { name: ' ', steps: STEPS } })).status, 400);
  const unknown = await call('POST', url(`funnels/preview?${RANGE}`), { cookie, body: { steps: [STEPS[0], { kind: 'autre', pattern: 'x' }] } });
  assert.ok(unknown.status >= 400 && unknown.status < 500);
  // Une expression régulière coûteuse ne bloque pas le serveur.
  const started = Date.now();
  const heavy = await call('POST', url(`funnels/preview?${RANGE}`), { cookie, body: { steps: [STEPS[0], { kind: 'page', match: 'regex', pattern: '(a+)+$' }] } });
  assert.equal(heavy.status, 200);
  assert.ok(Date.now() - started < 3000);
});

/** Référence des parcours : comptes exacts par niveau et par transition, sans regroupement. */
function pathsReference(list, { depth, start = null }) {
  const nodes = []; const links = {}; let matched = 0;
  for (const v of list) {
    let seq = [];
    for (const a of v.actions) {
      if (a.type !== 'page') continue;
      const l = pathOf(a.url);
      if (seq.at(-1) !== l) seq.push(l);
    }
    if (start !== null) {
      const pos = seq.indexOf(start);
      if (pos < 0) continue;
      seq = seq.slice(pos);
    }
    if (!seq.length) continue;
    matched++;
    const ended = seq.length <= depth;
    seq = seq.slice(0, depth);
    seq.forEach((l, i) => {
      nodes[i] ??= {};
      nodes[i][l] = (nodes[i][l] || 0) + 1;
      let to = null;
      if (i + 1 < seq.length) to = seq[i + 1];
      else if (ended && i + 1 < depth) to = '__sortie__';
      if (to !== null) { const k = `${i}|${l}|${to}`; links[k] = (links[k] || 0) + 1; }
    });
  }
  return { nodes, links, matched };
}
const linkMap = (links) => Object.fromEntries(links.map((l) => [`${l.level}|${l.from}|${l.to}`, l.sessions]));

test('parcours depuis l\'entrée : niveaux et transitions exacts', async () => {
  const ref = pathsReference(visits, { depth: 4 });
  const res = await call('GET', url(`paths?depth=4&max_nodes=15&${RANGE}`), { cookie });
  assert.equal(res.status, 200);
  // Toutes les visites du jeu d'essai comptent au moins une page vue.
  assert.equal(res.body.sessions_in_period, visits.length);
  assert.equal(res.body.matched, ref.matched);
  assert.equal(res.body.levels.length, 4);
  res.body.levels.forEach((level, i) => {
    assert.deepEqual(Object.fromEntries(level.map((n) => [n.label, n.sessions])), ref.nodes[i], `niveau ${i + 1}`);
  });
  assert.deepEqual(linkMap(res.body.links), ref.links);
});

test('le regroupement « autres » conserve les totaux de chaque niveau', async () => {
  const ref = pathsReference(visits, { depth: 5 });
  const res = await call('GET', url(`paths?depth=5&max_nodes=2&${RANGE}`), { cookie });
  res.body.levels.forEach((level, i) => {
    const total = Object.values(ref.nodes[i]).reduce((a, b) => a + b, 0);
    assert.equal(level.reduce((a, n) => a + n.sessions, 0), total, `total du niveau ${i + 1}`);
    assert.ok(level.filter((n) => n.label !== '__autres__').length <= 2);
    const out = res.body.links.filter((l) => l.level === i).reduce((a, l) => a + l.sessions, 0);
    assert.ok(out <= total, 'ce qui sort d\'un niveau ne dépasse pas ce qui y est entré');
  });
});

test('parcours à partir d\'une page, avec filtre, et période vide', async () => {
  const ref = pathsReference(visits, { depth: 3, start: '/produits' });
  const res = await call('GET', url(`paths?depth=3&max_nodes=15&start=${encodeURIComponent('/produits')}&${RANGE}`), { cookie });
  assert.equal(res.body.matched, ref.matched);
  assert.ok(res.body.matched > 0 && res.body.matched < res.body.sessions_in_period);
  assert.deepEqual(res.body.levels[0], [{ label: '/produits', sessions: ref.matched }]);
  assert.deepEqual(linkMap(res.body.links), ref.links);

  const mobile = visits.filter((v) => /iPhone|Mobile/.test(v.ua));
  const filtered = await call('GET', url(`paths?depth=3&max_nodes=15&device=2&${RANGE}`), { cookie });
  assert.deepEqual(linkMap(filtered.body.links), pathsReference(mobile, { depth: 3 }).links);

  const empty = await call('GET', url('paths?from=2026-10-01&to=2026-10-02'), { cookie });
  assert.deepEqual([empty.body.sessions_in_period, empty.body.levels, empty.body.links], [0, [], []]);
  assert.equal((await call('GET', url(`paths?${RANGE}`), { cookie: other })).status, 404);
});
