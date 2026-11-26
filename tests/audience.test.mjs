// Mesure d'audience : répartitions et filtres, comparés à des totaux recalculés
// indépendamment sur le jeu d'essai.
import test, { before } from 'node:test';
import assert from 'node:assert/strict';
import { generateVisits, SITE } from '../dev/fixtures.mjs';
import { call, login, newProject, newToken } from './helpers.mjs';

const RANGE = 'from=2026-11-01&to=2026-11-30';
const visits = generateVisits();
const pathOf = (url) => (url.slice(SITE.url.length) || '/').replace(/[?#].*$/, '');
const pages = (v) => v.actions.filter((a) => a.type === 'page');
const channelOf = (v) => (!v.referrer ? 0 : /google|bing/.test(v.referrer) ? 1 : /facebook/.test(v.referrer) ? 2 : 3);
const browserOf = (v) => (v.ua.includes('Firefox') ? 'Firefox' : v.ua.includes('Chrome') ? 'Chrome' : 'Safari');
const deviceOf = (v) => (/iPhone|Mobile/.test(v.ua) ? 2 : 1);

let cookie; let project;
const get = (path) => call('GET', `/api/projects/${project.id}/stats/${path}`, { cookie });

/** Regroupe les visites par libellé : { libellé: { visitors, sessions, pageviews } }. */
function group(list, labelOf) {
  const out = {};
  for (const v of list) {
    const g = (out[labelOf(v)] ??= { visitors: 0, sessions: 0, pageviews: 0 });
    g.visitors++; g.sessions++; g.pageviews += pages(v).length;
  }
  return out;
}
const pick = (rows, keys = ['visitors', 'sessions', 'pageviews']) => Object.fromEntries(rows.map((r) => [r.label, Object.fromEntries(keys.map((k) => [k, r[k]]))]));

before(async () => {
  cookie = await login();
  project = await newProject(cookie);
  const { token } = await newToken(cookie, project.id, 'ingest');
  const events = visits.flatMap((v) => v.actions.map((a) => ({
    ts: a.ts * 1000, visitor: `v${v.id}`, ua: v.ua, u: a.url, r: v.referrer || undefined,
    ...(a.type === 'page' ? { ti: a.title } : { t: 'event', n: `${a.c} › ${a.a}` }),
  })));
  for (let i = 0; i < events.length; i += 2000) {
    const res = await call('POST', `/api/projects/${project.id}/import`, { token, body: { events: events.slice(i, i + 2000) } });
    assert.equal(res.body.rejected, 0);
  }
});

test('répartition par canal, navigateur, appareil et page d\'entrée', async () => {
  const cases = [
    ['channel', (v) => String(channelOf(v))],
    ['browser', browserOf],
    ['device', (v) => String(deviceOf(v))],
    ['entry', (v) => pathOf(pages(v)[0]?.url ?? v.actions[0].url)],
    ['exit', (v) => pathOf(pages(v).at(-1)?.url ?? v.actions[0].url)],
  ];
  for (const [dim, labelOf] of cases) {
    const res = await get(`breakdown?dim=${dim}&limit=100&${RANGE}`);
    assert.equal(res.status, 200, dim);
    assert.deepEqual(pick(res.body.rows), group(visits, labelOf), dim);
  }
  const referrers = await get(`breakdown?dim=referrer&limit=100&${RANGE}`);
  assert.deepEqual(Object.keys(pick(referrers.body.rows)).sort(), ['', 'bing.com', 'exemple-partenaire.fr', 'facebook.com', 'google.com']);
});

test('répartition par page et par événement', async () => {
  const expected = {}; const events = {};
  for (const v of visits) {
    const seen = new Set();
    for (const a of v.actions) {
      const target = a.type === 'page' ? expected : events;
      const label = a.type === 'page' ? pathOf(a.url) : `${a.c} › ${a.a}`;
      const g = (target[label] ??= { visitors: 0, sessions: 0, hits: 0 });
      g.hits++;
      if (!seen.has(a.type + label)) { seen.add(a.type + label); g.visitors++; g.sessions++; }
    }
  }
  const rename = (o, key) => Object.fromEntries(Object.entries(o).map(([k, g]) => [k, { visitors: g.visitors, sessions: g.sessions, [key]: g.hits }]));
  const res = await get(`breakdown?dim=page&limit=100&${RANGE}`);
  assert.deepEqual(pick(res.body.rows, ['visitors', 'sessions', 'pageviews']), rename(expected, 'pageviews'));
  assert.ok(!res.body.rows.some((r) => r.label.includes('?')), 'les paramètres d\'URL ne sont pas conservés');
  const ev = await get(`breakdown?dim=event&limit=100&${RANGE}`);
  assert.deepEqual(pick(ev.body.rows, ['visitors', 'sessions', 'count']), rename(events, 'count'));

  // Classement : visiteurs décroissants, et la limite est respectée.
  const top = await get(`breakdown?dim=page&limit=3&${RANGE}`);
  assert.equal(top.body.rows.length, 3);
  assert.deepEqual(top.body.rows.map((r) => r.visitors), [...top.body.rows.map((r) => r.visitors)].sort((a, b) => b - a));
});

test('un filtre restreint le résumé et les autres répartitions', async () => {
  const search = visits.filter((v) => channelOf(v) === 1);
  const s = await get(`summary?channel=1&${RANGE}`);
  assert.equal(s.body.sessions, search.length);
  assert.equal(s.body.pageviews, search.reduce((n, v) => n + pages(v).length, 0));
  assert.deepEqual(pick((await get(`breakdown?dim=browser&channel=1&limit=100&${RANGE}`)).body.rows), group(search, browserOf));

  // Deux filtres cumulés.
  const both = search.filter((v) => deviceOf(v) === 2);
  assert.equal((await get(`summary?channel=1&device=2&${RANGE}`)).body.sessions, both.length);

  // Filtre « page vue » : les sessions qui sont passées par /panier.
  const cart = visits.filter((v) => pages(v).some((a) => pathOf(a.url) === '/panier'));
  assert.ok(cart.length > 0 && cart.length < visits.length);
  assert.equal((await get(`summary?page=${encodeURIComponent('/panier')}&${RANGE}`)).body.sessions, cart.length);
  assert.deepEqual(pick((await get(`breakdown?dim=channel&page=${encodeURIComponent('/panier')}&limit=100&${RANGE}`)).body.rows), group(cart, (v) => String(channelOf(v))));

  // Filtre sur une valeur vide : les visites sans référent.
  const direct = visits.filter((v) => !v.referrer);
  assert.equal((await get(`summary?referrer=&${RANGE}`)).body.sessions, direct.length);
});

test('détail par heure d\'une journée, dans le fuseau du projet', async () => {
  const day = '2026-11-12';
  const fmt = new Intl.DateTimeFormat('en-GB', { timeZone: SITE.timezone, year: 'numeric', month: '2-digit', day: '2-digit', hour: '2-digit', hourCycle: 'h23' });
  const expected = {};
  for (const v of visits) {
    const parts = Object.fromEntries(fmt.formatToParts(new Date(v.actions[0].ts * 1000)).map((p) => [p.type, p.value]));
    if (`${parts.year}-${parts.month}-${parts.day}` === day) expected[Number(parts.hour)] = (expected[Number(parts.hour)] || 0) + 1;
  }
  const res = await get(`summary?interval=hour&from=${day}&to=${day}`);
  assert.deepEqual(Object.fromEntries(res.body.by_hour.map((h) => [h.hour, h.sessions])), expected);
  assert.equal((await get(`summary?interval=hour&${RANGE}`)).status, 400);
});

test('paramètres invalides et tentative d\'injection', async () => {
  assert.equal((await get(`breakdown?dim=inconnue&${RANGE}`)).status, 400);
  assert.equal((await get(`breakdown?dim=page`)).status, 400);
  assert.equal((await get(`summary?channel=abc&${RANGE}`)).status, 400);
  const inj = await get(`summary?browser=${encodeURIComponent("x' OR '1'='1")}&${RANGE}`);
  assert.equal(inj.status, 200);
  assert.equal(inj.body.sessions, 0, 'la valeur est liée, jamais interprétée');
  const rt = await get('realtime');
  assert.deepEqual(rt.body, { last_5_minutes: 0, last_30_minutes: 0 });
});
