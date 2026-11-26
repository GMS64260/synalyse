// Vérifie les parcours contre une implémentation de référence indépendante.
import test from 'node:test';
import assert from 'node:assert/strict';
import { api, visitsBetween, pathOf, ALL } from './helpers.mjs';

const label = (url, ignoreQuery) => {
  let p = pathOf(url);
  if (ignoreQuery) p = p.replace(/[?#].*$/, '');
  return p.length > 1 ? p.replace(/\/$/, '') : p;
};

/** Référence : comptes exacts par niveau et par transition, sans regroupement. */
function reference(visits, { depth, start = null, ignoreQuery = true }) {
  const nodes = []; const links = {}; let matched = 0;
  for (const v of visits) {
    let seq = [];
    for (const a of v.actions) {
      if (a.type !== 'page') continue;
      const l = label(a.url, ignoreQuery);
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

test('parcours depuis l\'entrée : niveaux et transitions exacts', async () => {
  const visits = visitsBetween(ALL.from, ALL.to);
  const ref = reference(visits, { depth: 4 });
  // maxNodes élevé : aucun regroupement, la comparaison est terme à terme.
  const res = await api('SynInsights.getPaths', { idSite: 1, period: ALL.period, date: ALL.date, depth: 4, maxNodes: 15 });
  assert.equal(res.visits, visits.length);
  assert.equal(res.matched, ref.matched);
  assert.equal(res.levels.length, 4);
  res.levels.forEach((level, i) => {
    assert.deepEqual(Object.fromEntries(level.map((n) => [n.label, n.visits])), ref.nodes[i], `niveau ${i + 1}`);
  });
  assert.deepEqual(Object.fromEntries(res.links.map((l) => [`${l.level}|${l.from}|${l.to}`, l.visits])), ref.links);
  // Les URL à paramètres sont confondues avec leur page.
  assert.ok(!res.levels.flat().some((n) => n.label.includes('?')));
});

test('le regroupement « autres » conserve les totaux de chaque niveau', async () => {
  const visits = visitsBetween(ALL.from, ALL.to);
  const ref = reference(visits, { depth: 5 });
  const res = await api('SynInsights.getPaths', { idSite: 1, period: ALL.period, date: ALL.date, depth: 5, maxNodes: 2 });
  res.levels.forEach((level, i) => {
    const total = Object.values(ref.nodes[i]).reduce((a, b) => a + b, 0);
    assert.equal(level.reduce((a, n) => a + n.visits, 0), total, `total du niveau ${i + 1}`);
    assert.ok(level.filter((n) => n.label !== '__autres__').length <= 2);
    // Ce qui sort d'un niveau ne dépasse jamais ce qui y est entré.
    const out = res.links.filter((l) => l.level === i).reduce((a, l) => a + l.visits, 0);
    assert.ok(out <= total);
  });
});

test('parcours à partir d\'une page donnée, paramètres d\'URL conservés ou non', async () => {
  const visits = visitsBetween(ALL.from, ALL.to);
  for (const ignoreQuery of [true, false]) {
    const ref = reference(visits, { depth: 3, start: '/produits', ignoreQuery });
    const res = await api('SynInsights.getPaths', { idSite: 1, period: ALL.period, date: ALL.date, depth: 3, maxNodes: 15, start: '/produits', ignoreQuery: ignoreQuery ? 1 : 0 });
    assert.equal(res.matched, ref.matched);
    assert.ok(res.matched > 0 && res.matched < res.visits);
    assert.deepEqual(res.levels[0], [{ label: '/produits', visits: ref.matched }]);
    assert.deepEqual(Object.fromEntries(res.links.map((l) => [`${l.level}|${l.from}|${l.to}`, l.visits])), ref.links);
  }
});

test('une période sans visite renvoie un résultat vide, pas une erreur', async () => {
  const res = await api('SynInsights.getPaths', { idSite: 1, period: 'day', date: '2026-10-15' });
  assert.equal(res.visits, 0);
  assert.deepEqual(res.levels, []);
});
