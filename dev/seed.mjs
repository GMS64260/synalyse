// Charge le jeu d'essai (dev/fixtures.mjs) dans l'instance de développement
// par l'API de suivi en lot. Refuse de tourner si le site contient déjà des visites.
import { readFileSync } from 'node:fs';
import { generateVisits } from './fixtures.mjs';

const BASE = process.env.SYN_DEV_URL || 'http://127.0.0.1:8291';
const token = readFileSync(new URL('./.token', import.meta.url), 'utf8').trim();

async function api(method, params = {}) {
  const body = new URLSearchParams({ module: 'API', method, format: 'json', token_auth: token, ...params });
  const res = await fetch(`${BASE}/index.php`, { method: 'POST', body });
  return res.json();
}

const existing = await api('Live.getLastVisitsDetails', { idSite: 1, period: 'range', date: '2026-10-01,2026-12-31', filter_limit: 1 });
if (Array.isArray(existing) && existing.length > 0 && !process.argv.includes('--force')) {
  console.log('Le site 1 contient déjà des visites : rien à faire (bin/dev reset pour repartir de zéro).');
  process.exit(0);
}

const visits = generateVisits();
const toQuery = (v, a) => {
  const p = new URLSearchParams({ idsite: '1', rec: '1', apiv: '1', url: a.url, cdt: String(a.ts), cip: v.ip, ua: v.ua, rand: `${v.id}-${a.ts}` });
  if (v.referrer) p.set('urlref', v.referrer);
  if (a.type === 'page') p.set('action_name', a.title);
  else { p.set('e_c', a.c); p.set('e_a', a.a); p.set('e_n', a.n); }
  return '?' + p.toString();
};

let sent = 0;
const BATCH = 25;
for (let i = 0; i < visits.length; i += BATCH) {
  const requests = visits.slice(i, i + BATCH).flatMap((v) => v.actions.map((a) => toQuery(v, a)));
  const res = await fetch(`${BASE}/matomo.php`, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ requests, token_auth: token }) });
  const out = await res.json().catch(() => ({}));
  if (out.status !== 'success' || out.tracked !== requests.length) {
    console.error('Lot refusé', i, res.status, out);
    process.exit(1);
  }
  sent += requests.length;
}
console.log(`${visits.length} visites, ${sent} actions envoyées.`);
