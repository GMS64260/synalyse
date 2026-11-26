// Outils communs aux tests d'intégration, lancés contre le serveur de développement.
export const BASE = process.env.SYNALYSE_DEV_URL || 'http://127.0.0.1:8292';
export const PASSWORD = 'dev-password-local-1234'; // comptes d'essai de la base de dev, voir bin/dev test
export const BROWSER = 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36';

/** Appel d'API. Renvoie { status, body } sans lever d'erreur : les tests vérifient aussi les refus. */
export async function call(method, path, { cookie, token, body, headers = {} } = {}) {
  const h = { ...headers };
  if (cookie) h.cookie = cookie;
  if (token) h.authorization = `Bearer ${token}`;
  if (body !== undefined) h['content-type'] = 'application/json';
  const res = await fetch(BASE + path, { method, headers: h, body: body === undefined ? undefined : JSON.stringify(body) });
  const text = await res.text();
  let parsed = null;
  try { parsed = text ? JSON.parse(text) : null; } catch { parsed = text; }
  return { status: res.status, body: parsed, headers: res.headers };
}

/** Requête dont seuls l'état et les en-têtes comptent. Le corps est lu jusqu'au bout : une réponse
 *  volumineuse laissée en plan garde sa connexion ouverte et empêche le programme de test de finir. */
export async function probe(url, init) {
  const res = await fetch(url.startsWith('http') ? url : BASE + url, init);
  await res.arrayBuffer();
  return res;
}

/** Ouvre une session et renvoie la valeur d'en-tête Cookie à réutiliser. */
export async function login(email = 'dev@synalyse.test', password = PASSWORD) {
  const res = await call('POST', '/api/auth/login', { body: { email, password } });
  if (res.status !== 200) throw new Error(`connexion refusée pour ${email} : ${res.status}`);
  return res.headers.get('set-cookie').split(';')[0];
}

/** Crée un projet neuf dans l'organisation du compte : chaque test travaille sur ses propres données. */
export async function newProject(cookie, { name = `Essai ${Date.now()}-${Math.random().toString(36).slice(2, 7)}`, timezone = 'Europe/Paris', domains = ['demo.test'] } = {}) {
  const me = await call('GET', '/api/me', { cookie });
  const org = me.body.organizations[0].id;
  const res = await call('POST', `/api/orgs/${org}/projects`, { cookie, body: { name, timezone, domains } });
  if (res.status !== 200) throw new Error(`création de projet refusée : ${res.status} ${JSON.stringify(res.body)}`);
  return res.body;
}

export async function newToken(cookie, projectId, scope) {
  const res = await call('POST', `/api/projects/${projectId}/tokens`, { cookie, body: { name: `test ${scope}`, scope } });
  if (res.status !== 200) throw new Error(`création de jeton refusée : ${res.status}`);
  return res.body;
}

/** Envoi public du traceur, comme le ferait un navigateur. */
export function track(key, events, { ua = BROWSER, distinct, extra = {} } = {}) {
  return fetch(`${BASE}/i`, {
    method: 'POST',
    headers: { 'content-type': 'text/plain;charset=UTF-8', 'user-agent': ua },
    body: JSON.stringify({ k: key, d: distinct, sw: 1440, l: 'fr-FR', e: events, ...extra }),
  });
}

/** Attend qu'une lecture satisfasse une condition : l'ingestion publique est asynchrone. */
export async function eventually(read, ok, { timeout = 4000 } = {}) {
  const end = Date.now() + timeout;
  let last;
  for (;;) {
    last = await read();
    if (ok(last) || Date.now() > end) return last;
    await new Promise((r) => setTimeout(r, 100));
  }
}

/** Jour (AAAA-MM-JJ) d'un instant dans un fuseau. */
export function dayIn(timezone, date = new Date()) {
  return new Intl.DateTimeFormat('en-CA', { timeZone: timezone, year: 'numeric', month: '2-digit', day: '2-digit' }).format(date);
}
