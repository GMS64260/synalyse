// Interface embarquée : fichiers servis, en-têtes de sécurité, cache.
import test from 'node:test';
import assert from 'node:assert/strict';
import { BASE } from './helpers.mjs';

test('la page d\'accueil est servie avec une politique de sécurité stricte', async () => {
  const res = await fetch(BASE + '/');
  assert.equal(res.status, 200);
  assert.match(res.headers.get('content-type'), /text\/html/);
  const csp = res.headers.get('content-security-policy');
  assert.match(csp, /default-src 'none'/);
  assert.match(csp, /script-src 'self'/);
  assert.match(csp, /frame-ancestors 'none'/);
  assert.ok(!csp.includes('unsafe-inline'));
  assert.equal(res.headers.get('x-content-type-options'), 'nosniff');
  const html = await res.text();
  assert.ok(!/<script(?![^>]*src=)/.test(html), 'aucun script en ligne');
  assert.ok(!/https?:\/\//.test(html), 'aucune ressource tierce');
});

test('les fichiers de l\'interface sont revalidés par ETag', async () => {
  for (const [name, type] of [['app.js', /javascript/], ['app.css', /css/], ['manrope.woff2', /font\/woff2/], ['logo.svg', /svg/]]) {
    const res = await fetch(`${BASE}/assets/${name}`);
    assert.equal(res.status, 200, name);
    assert.match(res.headers.get('content-type'), type);
    const etag = res.headers.get('etag');
    const again = await fetch(`${BASE}/assets/${name}`, { headers: { 'if-none-match': etag } });
    assert.equal(again.status, 304, name);
  }
  // Le CSS et le JS ne chargent rien depuis un tiers.
  const css = await (await fetch(`${BASE}/assets/app.css`)).text();
  assert.ok(!/url\(["']?https?:/.test(css));
});

test('seuls les fichiers prévus sont servis', async () => {
  for (const name of ['index.html', 'inconnu.js', '..%2fCargo.toml', '%2e%2e%2f%2e%2e%2fetc%2fpasswd', 'fonts%2fOFL.txt']) {
    assert.equal((await fetch(`${BASE}/assets/${name}`)).status, 404, name);
  }
  assert.equal((await fetch(`${BASE}/n-importe-quoi`)).status, 404);
});
