import { readFileSync } from 'node:fs';
import { storedVisits, siteDay, SITE } from '../dev/fixtures.mjs';

export const BASE = process.env.SYN_DEV_URL || 'http://127.0.0.1:8291';
const token = readFileSync(new URL('../dev/.token', import.meta.url), 'utf8').trim();

/** Appelle l'API de l'instance de développement. Lève une erreur si Matomo en renvoie une. */
export async function api(method, params = {}, { raw = false } = {}) {
  const body = new URLSearchParams({ module: 'API', method, format: 'json', token_auth: token });
  for (const [k, v] of Object.entries(params)) body.set(k, typeof v === 'string' ? v : JSON.stringify(v));
  const res = await fetch(`${BASE}/index.php`, { method: 'POST', body });
  const json = await res.json();
  if (!raw && json && json.result === 'error') throw new Error(json.message);
  return json;
}

/** Visites du jeu d'essai commencées entre deux jours inclus (fuseau du site). */
export function visitsBetween(from, to, filter = () => true) {
  return storedVisits().filter((v) => {
    const day = siteDay(v.actions[0].ts);
    return day >= from && day <= to && filter(v);
  });
}

export const pathOf = (url) => url.slice(SITE.url.length) || '/';
export const ALL = { period: 'range', date: '2026-11-01,2026-11-30', from: '2026-11-01', to: '2026-11-30' };
