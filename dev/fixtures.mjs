// Jeu d'essai déterministe pour l'instance de développement.
// Le même générateur sert à remplir l'instance de dev et à calculer les
// résultats attendus dans les tests : aucun chiffre n'est écrit en dur.

export const SITE = { name: 'Boutique démo', url: 'https://demo.test', timezone: 'Europe/Paris' };
export const FIRST_DAY = '2026-11-02'; // lundi
export const DAYS = 21;

function mulberry32(seed) {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

const UAS = [
  'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36',
  'Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Mobile/15E148 Safari/604.1',
  'Mozilla/5.0 (Macintosh; Intel Mac OS X 10.15; rv:131.0) Gecko/20100101 Firefox/131.0',
  'Mozilla/5.0 (Linux; Android 14; Pixel 8) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Mobile Safari/537.36',
];
const REFERRERS = ['', '', 'https://www.google.com/', 'https://www.bing.com/', 'https://www.facebook.com/', 'https://exemple-partenaire.fr/article'];
const PRODUCTS = ['lampe-atelier', 'chaise-bistrot', 'table-ferme', 'tapis-laine'];
const BLOG = ['entretenir-le-bois', 'choisir-une-lampe'];

const page = (path, title) => ({ type: 'page', url: SITE.url + path, title });
const event = (path, c, a, n) => ({ type: 'event', url: SITE.url + path, c, a, n });

// Construit le parcours d'une visite : suite d'actions avec un délai (s) avant chacune.
function buildJourney(rnd) {
  const acts = [];
  const push = (a) => acts.push({ ...a, wait: 5 + Math.floor(rnd() * 90) });
  const r = rnd();
  if (r < 0.2) {
    // Lecteur du blog, parfois curieux du catalogue.
    const b = BLOG[Math.floor(rnd() * BLOG.length)];
    push(page(`/blog/${b}`, `Blog : ${b}`));
    if (rnd() < 0.4) push(page('/produits', 'Catalogue'));
    return acts;
  }
  push(page('/', 'Accueil'));
  if (rnd() < 0.25) return acts; // rebond
  if (rnd() < 0.15) { push(page('/contact', 'Contact')); return acts; }
  push(page('/produits', 'Catalogue'));
  let inCart = false;
  const looks = 1 + Math.floor(rnd() * 3);
  for (let i = 0; i < looks; i++) {
    if (rnd() < 0.2) break;
    const p = PRODUCTS[Math.floor(rnd() * PRODUCTS.length)];
    // Paramètre d'URL variable : les règles « contient » et « commence par » doivent le tolérer.
    const q = rnd() < 0.3 ? '?couleur=noir' : '';
    push(page(`/produits/${p}${q}`, `Produit : ${p}`));
    if (rnd() < 0.45) { push(event(`/produits/${p}${q}`, 'Panier', 'Ajout', p)); inCart = true; }
    if (rnd() < 0.3) push(page('/produits', 'Catalogue'));
  }
  if (!inCart || rnd() < 0.3) return acts;
  push(page('/panier', 'Panier'));
  if (rnd() < 0.35) return acts;
  push(page('/commande', 'Commande'));
  if (rnd() < 0.2) push(page('/panier', 'Panier')); // retour en arrière
  if (rnd() < 0.4) return acts;
  push(page('/merci', 'Merci'));
  return acts;
}

/**
 * @returns {Array<{id:number, ip:string, ua:string, referrer:string, start:number, returning:boolean,
 *   actions:Array<{type:string,url:string,title?:string,c?:string,a?:string,n?:string,ts:number}>}>}
 * `start` et `ts` sont des horodatages Unix (UTC).
 */
export function generateVisits() {
  const rnd = mulberry32(20261126);
  const visits = [];
  const day0 = Date.parse(FIRST_DAY + 'T00:00:00Z') / 1000;
  let id = 0;
  for (let d = 0; d < DAYS; d++) {
    const n = 18 + Math.floor(rnd() * 14);
    for (let i = 0; i < n; i++) {
      id++;
      // Heures réparties sur toute la journée UTC : certaines visites tombent
      // le lendemain en heure de Paris (UTC+1), ce qui éprouve les bornes de période.
      const start = day0 + d * 86400 + Math.floor(rnd() * 86400);
      const ip = `10.${1 + (id >> 16)}.${(id >> 8) & 255}.${id & 255}`;
      const ua = UAS[Math.floor(rnd() * UAS.length)];
      const referrer = REFERRERS[Math.floor(rnd() * REFERRERS.length)];
      let ts = start;
      const actions = buildJourney(rnd).map((a) => {
        ts += a.wait;
        const { wait, ...rest } = a;
        return { ...rest, ts };
      });
      visits.push({ id, ip, ua, referrer, start, actions });
    }
  }
  return visits;
}

/** Jour calendaire (AAAA-MM-JJ) d'un horodatage dans le fuseau du site. */
export function siteDay(ts) {
  return new Intl.DateTimeFormat('en-CA', { timeZone: SITE.timezone, year: 'numeric', month: '2-digit', day: '2-digit' }).format(new Date(ts * 1000));
}

