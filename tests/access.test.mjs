// Cloisonnement entre organisations et droits des jetons.
import test from 'node:test';
import assert from 'node:assert/strict';
import { call, login, newProject, newToken, PASSWORD } from './helpers.mjs';

const RANGE = 'from=2026-11-01&to=2026-11-30';

test('sans authentification, l\'API répond 401', async () => {
  for (const [method, path] of [['GET', '/api/me'], ['GET', '/api/orgs/1/projects'], ['GET', `/api/projects/1/stats/summary?${RANGE}`], ['POST', '/api/projects/1/import']]) {
    const res = await call(method, path, { body: method === 'POST' ? { events: [] } : undefined });
    assert.equal(res.status, 401, `${method} ${path}`);
  }
  assert.equal((await call('GET', '/api/me', { cookie: 'synalyse_session=' + 'a'.repeat(64) })).status, 401);
  assert.equal((await call('GET', '/api/me', { token: 'syn_' + 'a'.repeat(64) })).status, 401);
});

test('la connexion refuse un mauvais mot de passe sans dire si le compte existe', async () => {
  const wrong = await call('POST', '/api/auth/login', { body: { email: 'dev@synalyse.test', password: 'mauvais-mot-de-passe' } });
  const unknown = await call('POST', '/api/auth/login', { body: { email: 'personne@synalyse.test', password: 'mauvais-mot-de-passe' } });
  assert.equal(wrong.status, 400);
  assert.deepEqual(wrong.body, unknown.body);
  assert.equal(wrong.headers.get('set-cookie'), null);
  const ok = await call('POST', '/api/auth/login', { body: { email: ' DEV@synalyse.test ', password: PASSWORD } });
  assert.equal(ok.status, 200);
  assert.match(ok.headers.get('set-cookie'), /HttpOnly; SameSite=Lax/);
});

test('une organisation ne voit ni ne modifie les projets d\'une autre', async () => {
  const mine = await login('dev@synalyse.test');
  const other = await login('autre@synalyse.test');
  const project = await newProject(mine);
  const myOrg = project.org_id;

  assert.equal((await call('GET', `/api/projects/${project.id}`, { cookie: mine })).status, 200);
  // Même réponse que pour un projet inexistant : l'existence ne fuit pas.
  for (const [method, path, body] of [
    ['GET', `/api/projects/${project.id}`],
    ['PATCH', `/api/projects/${project.id}`, { name: 'Piraté' }],
    ['GET', `/api/projects/${project.id}/stats/summary?${RANGE}`],
    ['GET', `/api/projects/${project.id}/tokens`],
    ['POST', `/api/projects/${project.id}/tokens`, { name: 'x', scope: 'admin' }],
    ['POST', `/api/projects/${project.id}/import`, { events: [] }],
    ['GET', `/api/orgs/${myOrg}/projects`],
    ['POST', `/api/orgs/${myOrg}/projects`, { name: 'Intrus' }],
  ]) {
    const res = await call(method, path, { cookie: other, body });
    assert.equal(res.status, 404, `${method} ${path}`);
  }
  const missing = await call('GET', '/api/projects/999999999', { cookie: other });
  const foreign = await call('GET', `/api/projects/${project.id}`, { cookie: other });
  assert.deepEqual(foreign.body, missing.body);
  assert.equal((await call('GET', `/api/projects/${project.id}`, { cookie: mine })).body.name, project.name);

  const listed = await call('GET', '/api/me', { cookie: other });
  assert.ok(!listed.body.projects.some((p) => p.id === project.id));
});

test('un jeton n\'ouvre que son projet et que sa portée', async () => {
  const cookie = await login();
  const a = await newProject(cookie);
  const b = await newProject(cookie);
  const read = await newToken(cookie, a.id, 'read');
  const ingest = await newToken(cookie, a.id, 'ingest');
  const stats = (id, token) => call('GET', `/api/projects/${id}/stats/summary?${RANGE}`, { token });
  const imp = (id, token) => call('POST', `/api/projects/${id}/import`, { token, body: { events: [] } });

  assert.equal((await stats(a.id, read.token)).status, 200);
  assert.equal((await imp(a.id, read.token)).status, 403, 'un jeton de lecture n\'importe pas');
  assert.equal((await imp(a.id, ingest.token)).status, 200);
  assert.equal((await stats(a.id, ingest.token)).status, 403, 'un jeton d\'import ne lit pas');
  assert.equal((await stats(b.id, read.token)).status, 404, 'autre projet');
  assert.equal((await call('GET', `/api/projects/${a.id}/tokens`, { token: read.token })).status, 403);
  assert.equal((await call('GET', '/api/me', { token: read.token })).status, 403);

  // La valeur du jeton n'est jamais relisible ; une fois révoqué, il ne passe plus.
  const list = await call('GET', `/api/projects/${a.id}/tokens`, { cookie });
  assert.ok(list.body.tokens.length === 2 && !JSON.stringify(list.body).includes(read.token));
  assert.equal((await call('DELETE', `/api/projects/${b.id}/tokens/${read.id}`, { cookie })).status, 404, 'révocation par le mauvais projet');
  assert.equal((await call('DELETE', `/api/projects/${a.id}/tokens/${read.id}`, { cookie })).status, 200);
  assert.equal((await stats(a.id, read.token)).status, 401);
});

test('la déconnexion invalide la session', async () => {
  const cookie = await login();
  assert.equal((await call('GET', '/api/me', { cookie })).status, 200);
  assert.equal((await call('POST', '/api/auth/logout', { cookie })).status, 200);
  assert.equal((await call('GET', '/api/me', { cookie })).status, 401);
});

test('les saisies invalides sont refusées avec un message', async () => {
  const cookie = await login();
  const me = await call('GET', '/api/me', { cookie });
  const org = me.body.organizations[0].id;
  const bad = async (body) => (await call('POST', `/api/orgs/${org}/projects`, { cookie, body })).status;
  assert.equal(await bad({ name: '   ' }), 400);
  assert.equal(await bad({ name: 'x', timezone: 'Mars/Olympus' }), 400);
  assert.equal(await bad({ name: 'x', domains: ['pas un domaine'] }), 400);
  const project = await newProject(cookie, { domains: ['https://www.Exemple.fr/chemin'] });
  assert.deepEqual(project.domains, ['exemple.fr']);
  assert.equal((await call('POST', `/api/projects/${project.id}/tokens`, { cookie, body: { name: 'x', scope: 'root' } })).status, 400);
  assert.equal((await call('GET', `/api/projects/${project.id}/stats/summary?from=2026-11-30&to=2026-11-01`, { cookie })).status, 400);
  assert.equal((await call('GET', `/api/projects/${project.id}/stats/summary?from=hier&to=demain`, { cookie })).status, 400);
});
