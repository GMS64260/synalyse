// Vérifie le calcul des entonnoirs contre une implémentation de référence
// indépendante, appliquée au même jeu d'essai.
import test from 'node:test';
import assert from 'node:assert/strict';
import { api, visitsBetween, pathOf, ALL } from './helpers.mjs';

const STEPS = [
  { name: 'Accueil', kind: 'url', match: 'equals', pattern: '/' },
  { name: 'Fiche produit', kind: 'url', match: 'starts_with', pattern: '/produits/' },
  { name: 'Ajout au panier', kind: 'event', category: 'Panier', action: 'Ajout' },
  { name: 'Panier', kind: 'title', match: 'equals', pattern: 'Panier' },
  { name: 'Commande passée', kind: 'url', match: 'regex', pattern: '^/merci$' },
];

function matches(step, a) {
  if (step.kind === 'event') {
    return a.type === 'event' && (!step.category || a.c === step.category) && (!step.action || a.a === step.action) && (!step.event_name || a.n === step.event_name);
  }
  if (a.type !== 'page') return false;
  const value = step.kind === 'url' ? pathOf(a.url) : a.title;
  switch (step.match) {
    case 'equals': return value === step.pattern;
    case 'starts_with': return value.startsWith(step.pattern);
    case 'contains': return value.includes(step.pattern);
    case 'ends_with': return value.endsWith(step.pattern);
    case 'regex': return new RegExp(step.pattern, 'iu').test(value);
  }
}

function reference(visits, steps) {
  const reached = steps.map(() => 0);
  const dropped = steps.map(() => ({ exit: 0, next: {} }));
  const gaps = steps.map(() => []);
  for (const v of visits) {
    let i = 0; let last = 0; let nextPage; let waiting = false;
    for (const a of v.actions) {
      if (i < steps.length && matches(steps[i], a)) {
        reached[i]++;
        if (i > 0) gaps[i].push(a.ts - last);
        last = a.ts; i++; waiting = true; nextPage = undefined;
      } else if (waiting && a.type === 'page') { nextPage = pathOf(a.url); waiting = false; }
    }
    if (i > 0 && i < steps.length) {
      if (nextPage === undefined) dropped[i - 1].exit++;
      else dropped[i - 1].next[nextPage] = (dropped[i - 1].next[nextPage] || 0) + 1;
    }
  }
  const median = (xs) => {
    if (!xs.length) return null;
    const s = [...xs].sort((a, b) => a - b); const m = s.length >> 1;
    return s.length % 2 ? s[m] : Math.round((s[m - 1] + s[m]) / 2);
  };
  return { reached, dropped, medians: gaps.map(median) };
}

test('entonnoir sur tout le jeu d\'essai : visites, abandons, délais médians', async () => {
  const visits = visitsBetween(ALL.from, ALL.to);
  const ref = reference(visits, STEPS);
  const report = await api('SynInsights.getFunnelReport', { idSite: 1, period: ALL.period, date: ALL.date, steps: STEPS });

  assert.equal(report.visits_in_period, visits.length);
  assert.deepEqual(report.steps.map((s) => s.visits), ref.reached);
  assert.ok(ref.reached[4] > 0 && ref.reached[0] > ref.reached[4], 'le jeu d\'essai doit avoir des conversions et des abandons');
  assert.equal(report.entered, ref.reached[0]);
  assert.equal(report.converted, ref.reached[4]);
  assert.deepEqual(report.steps.map((s) => s.median_seconds_from_previous), ref.medians);
  report.steps.forEach((s, i) => {
    assert.equal(s.dropped, i < 4 ? ref.reached[i] - ref.reached[i + 1] : 0);
    const exit = s.next_after_drop.find((n) => n.exit);
    assert.equal(exit ? exit.visits : 0, ref.dropped[i].exit, `sorties après l'étape ${i + 1}`);
    for (const n of s.next_after_drop.filter((x) => !x.exit)) {
      assert.equal(n.visits, ref.dropped[i].next[n.label], `suite « ${n.label} » après l'étape ${i + 1}`);
    }
  });
});

test('les bornes de période suivent le fuseau du site (visites autour de minuit à Paris)', async () => {
  for (const day of ['2026-11-21', '2026-11-22', '2026-11-12']) {
    const visits = visitsBetween(day, day);
    const ref = reference(visits, STEPS);
    const report = await api('SynInsights.getFunnelReport', { idSite: 1, period: 'day', date: day, steps: STEPS });
    assert.equal(report.visits_in_period, visits.length, `visites du ${day}`);
    assert.deepEqual(report.steps.map((s) => s.visits), ref.reached, `étapes du ${day}`);
  }
});

test('le segment restreint les visites comptées', async () => {
  const visits = visitsBetween(ALL.from, ALL.to, (v) => v.ua.includes('Firefox'));
  const ref = reference(visits, STEPS);
  const report = await api('SynInsights.getFunnelReport', { idSite: 1, period: ALL.period, date: ALL.date, segment: 'browserCode==FF', steps: STEPS });
  assert.ok(visits.length > 0);
  assert.equal(report.visits_in_period, visits.length);
  assert.deepEqual(report.steps.map((s) => s.visits), ref.reached);
});

test('un entonnoir s\'enregistre, se relit, se modifie et se supprime par son identifiant', async () => {
  const name = 'Achat « test » <b>&';
  const saved = await api('SynInsights.saveFunnel', { idSite: 1, name, steps: STEPS });
  assert.ok(saved.idfunnel > 0);
  assert.equal(saved.name, name, 'le nom est stocké tel que saisi');
  assert.deepEqual(saved.steps.map((s) => s.name), STEPS.map((s) => s.name));

  const byId = await api('SynInsights.getFunnelReport', { idSite: 1, period: ALL.period, date: ALL.date, idFunnel: saved.idfunnel });
  const adHoc = await api('SynInsights.getFunnelReport', { idSite: 1, period: ALL.period, date: ALL.date, steps: STEPS });
  assert.deepEqual(byId.steps.map((s) => s.visits), adHoc.steps.map((s) => s.visits));

  const renamed = await api('SynInsights.saveFunnel', { idSite: 1, name: 'Achat', steps: STEPS.slice(0, 2), idFunnel: saved.idfunnel });
  assert.equal(renamed.idfunnel, saved.idfunnel);
  assert.equal(renamed.steps.length, 2);

  await api('SynInsights.deleteFunnel', { idSite: 1, idFunnel: saved.idfunnel });
  const list = await api('SynInsights.getFunnels', { idSite: 1 });
  assert.ok(!list.some((f) => f.idfunnel === saved.idfunnel));
  const gone = await api('SynInsights.getFunnelReport', { idSite: 1, period: ALL.period, date: ALL.date, idFunnel: saved.idfunnel }, { raw: true });
  assert.equal(gone.result, 'error');
});

test('les étapes invalides sont refusées avec un message', async () => {
  const cases = [
    [STEPS.slice(0, 1), /au moins deux/],
    [[STEPS[0], { kind: 'url', match: 'regex', pattern: '(' }], /expression régulière/],
    [[STEPS[0], { kind: 'event' }], /événement/],
    [[STEPS[0], { kind: 'url', match: 'equals', pattern: '  ' }], /motif est vide/],
  ];
  for (const [steps, expected] of cases) {
    const res = await api('SynInsights.getFunnelReport', { idSite: 1, period: ALL.period, date: ALL.date, steps }, { raw: true });
    assert.equal(res.result, 'error');
    assert.match(res.message, expected);
  }
});

test('sans jeton, l\'API refuse l\'accès', async () => {
  const body = new URLSearchParams({ module: 'API', method: 'SynInsights.getFunnels', format: 'json', idSite: '1' });
  const res = await (await fetch('http://127.0.0.1:8291/index.php', { method: 'POST', body })).json();
  assert.equal(res.result, 'error');
});
