// Synalyse — interface. JavaScript simple, sans dépendance ni étape de build.
// Tout texte venu des données passe par textContent : jamais de HTML construit à la main.

const app = document.getElementById('app');
const nf = new Intl.NumberFormat('fr-FR');
const pf = new Intl.NumberFormat('fr-FR', { style: 'percent', maximumFractionDigits: 1 });

const CHANNELS = { 0: 'Accès direct', 1: 'Moteurs de recherche', 2: 'Réseaux sociaux', 3: 'Sites référents', 4: 'Campagnes', 5: 'Assistants IA' };
const DEVICES = { 0: 'Inconnu', 1: 'Ordinateur', 2: 'Mobile', 3: 'Tablette' };
const PERIODS = [
  ['today', "Aujourd'hui"], ['yesterday', 'Hier'], ['7d', '7 derniers jours'], ['30d', '30 derniers jours'],
  ['month', 'Ce mois-ci'], ['12m', '12 derniers mois'], ['custom', 'Période personnalisée'],
];
const METRICS = [
  { key: 'visitors', label: 'Visiteurs', chart: true },
  { key: 'sessions', label: 'Visites', chart: true },
  { key: 'pageviews', label: 'Pages vues', chart: true },
  { key: 'bounce_rate', label: 'Taux de rebond', format: (v) => pf.format(v), lowerIsBetter: true },
  { key: 'avg_session_seconds', label: 'Durée moyenne', format: duration },
];
// Chaque onglet : dimension de l'API, libellé, et le filtre posé quand on clique une ligne.
const PANELS = [
  { title: 'Pages', tabs: [['page', 'Pages vues'], ['entry', "Pages d'entrée"], ['exit', 'Pages de sortie']] },
  { title: 'Provenance', tabs: [['channel', 'Canaux'], ['referrer', 'Sites'], ['utm_campaign', 'Campagnes']] },
  { title: 'Appareils', tabs: [['device', 'Type'], ['browser', 'Navigateur'], ['os', 'Système']] },
  { title: 'Événements', tabs: [['event', 'Événements']] },
];
const FILTER_NAMES = {
  page: 'Page', entry: 'Entrée', exit: 'Sortie', channel: 'Canal', referrer: 'Site référent', utm_campaign: 'Campagne',
  utm_source: 'Source', utm_medium: 'Support', device: 'Appareil', browser: 'Navigateur', os: 'Système', lang: 'Langue',
};

// ——— Outils ———

/** Crée un élément. `attrs` : attributs, `on*` pour les écouteurs, `text` pour le contenu texte. */
function h(tag, attrs = {}, ...children) {
  const el = document.createElement(tag);
  for (const [key, value] of Object.entries(attrs)) {
    if (value === null || value === undefined || value === false) continue;
    if (key === 'text') el.textContent = value;
    else if (key.startsWith('on')) el.addEventListener(key.slice(2), value);
    else el.setAttribute(key, value === true ? '' : value);
  }
  for (const child of children.flat()) if (child) el.append(child);
  return el;
}

function svg(tag, attrs = {}, ...children) {
  const el = document.createElementNS('http://www.w3.org/2000/svg', tag);
  for (const [key, value] of Object.entries(attrs)) el.setAttribute(key, value);
  for (const child of children) el.append(child);
  return el;
}

function logoMark() {
  return svg('svg', { viewBox: '0 0 32 32', 'aria-hidden': 'true' },
    svg('rect', { class: 'stone', x: 3, y: 21, width: 26, height: 8, rx: 4 }),
    svg('rect', { class: 'stone', x: 7, y: 12, width: 18, height: 7, rx: 3.5 }),
    svg('rect', { class: 'stone-top', x: 11, y: 4, width: 10, height: 6, rx: 3 }));
}

function duration(seconds) {
  const s = Math.round(seconds);
  if (s < 60) return `${s} s`;
  const m = Math.floor(s / 60);
  return m < 60 ? `${m} min ${String(s % 60).padStart(2, '0')} s` : `${Math.floor(m / 60)} h ${String(m % 60).padStart(2, '0')}`;
}

class ApiError extends Error {
  constructor(status, message) { super(message); this.status = status; }
}

async function api(path, { method = 'GET', body } = {}) {
  const res = await fetch(path, {
    method,
    headers: body === undefined ? {} : { 'content-type': 'application/json' },
    body: body === undefined ? undefined : JSON.stringify(body),
    credentials: 'same-origin',
  });
  const data = await res.json().catch(() => null);
  if (!res.ok) throw new ApiError(res.status, data?.error?.message || 'Le serveur ne répond pas. Réessayez dans un instant.');
  return data;
}

// ——— Dates : toujours des chaînes AAAA-MM-JJ, dans le fuseau du projet ———

const toDay = (date) => date.toISOString().slice(0, 10);
const addDays = (day, n) => toDay(new Date(Date.parse(day + 'T00:00:00Z') + n * 86400000));
const daysBetween = (from, to) => Math.round((Date.parse(to) - Date.parse(from)) / 86400000);
const todayIn = (timezone) => new Intl.DateTimeFormat('en-CA', { timeZone: timezone, year: 'numeric', month: '2-digit', day: '2-digit' }).format(new Date());
const validDay = (value) => /^\d{4}-\d{2}-\d{2}$/.test(value || '') && !Number.isNaN(Date.parse(value));
const shortDay = new Intl.DateTimeFormat('fr-FR', { day: 'numeric', month: 'short', timeZone: 'UTC' });
const longDay = new Intl.DateTimeFormat('fr-FR', { weekday: 'long', day: 'numeric', month: 'long', timeZone: 'UTC' });
const dayLabel = (day, format = shortDay) => format.format(new Date(day + 'T00:00:00Z'));

function resolvePeriod(params, timezone) {
  const today = todayIn(timezone);
  let preset = params.get('period') || '30d';
  let from; let to = today;
  switch (preset) {
    case 'today': from = today; break;
    case 'yesterday': from = to = addDays(today, -1); break;
    case '7d': from = addDays(today, -6); break;
    case 'month': from = today.slice(0, 8) + '01'; break;
    case '12m': from = addDays(today, -364); break;
    case 'custom':
      if (validDay(params.get('from')) && validDay(params.get('to')) && params.get('from') <= params.get('to')) {
        from = params.get('from'); to = params.get('to');
        break;
      }
      preset = '30d'; // période personnalisée incomplète : on retombe sur le défaut
    // falls through
    default: preset = PERIODS.some(([key]) => key === preset) ? preset : '30d'; from = addDays(today, -29);
  }
  const length = daysBetween(from, to) + 1;
  return { preset, from, to, prevFrom: addDays(from, -length), prevTo: addDays(from, -1), hourly: from === to };
}

// ——— Adresse : l'état de l'écran vit dans le fragment d'URL, donc se partage et suit le bouton Retour ———

function readRoute() {
  const [path, query = ''] = location.hash.replace(/^#/, '').split('?');
  const parts = path.split('/').filter(Boolean);
  return { view: parts[0] || '', id: Number(parts[1]) || 0, sub: parts[2] || '', params: new URLSearchParams(query) };
}

function go(path, params) {
  const query = params ? params.toString() : '';
  location.hash = '#' + path + (query ? '?' + query : '');
}

let me = null;
let liveTimer = null;
let renderToken = 0;

function mount(...nodes) {
  clearInterval(liveTimer);
  app.removeAttribute('aria-busy');
  app.replaceChildren(...nodes);
}

async function boot() {
  try {
    me = await api('/api/me');
  } catch (error) {
    if (error.status === 401) return renderLogin();
    return mount(h('div', { class: 'screen' }, brand(), h('h1', { text: 'Synalyse est injoignable' }), h('p', { class: 'lead', text: error.message })));
  }
  render();
}

function render() {
  if (!me) return renderLogin();
  const route = readRoute();
  const project = me.projects.find((p) => p.id === route.id);
  if (route.view === 'new') return renderNewProject();
  if (!me.projects.length) return renderNewProject(true);
  if (!project) return go(`p/${me.projects[0].id}`);
  if (route.sub === 'installation') return renderInstall(project);
  if (route.sub === 'entonnoirs') return renderFunnels(project, route.params);
  if (route.sub === 'parcours') return renderPaths(project, route.params);
  if (route.sub === 'enregistrements') return renderRecordings(project, route.params);
  renderDashboard(project, route.params);
}

const brand = () => h('a', { class: 'brand', href: '#', translate: 'no' }, logoMark(), h('span', { text: 'Synalyse' }));

// ——— Connexion ———

function renderLogin() {
  const error = h('p', { class: 'error', role: 'alert' });
  const button = h('button', { class: 'btn', type: 'submit', text: 'Se connecter' });
  const form = h('form', { class: 'form', onsubmit: async (event) => {
    event.preventDefault();
    button.disabled = true;
    error.textContent = '';
    const data = new FormData(form);
    try {
      await api('/api/auth/login', { method: 'POST', body: { email: data.get('email'), password: data.get('password') } });
      await boot();
    } catch (e) {
      error.textContent = e.message;
      button.disabled = false;
    }
  } },
  h('label', {}, 'Adresse e-mail', h('input', { class: 'field', type: 'email', name: 'email', autocomplete: 'username', required: true, spellcheck: 'false', autofocus: matchMedia('(pointer: fine)').matches })),
  h('label', {}, 'Mot de passe', h('input', { class: 'field', type: 'password', name: 'password', autocomplete: 'current-password', required: true })),
  error, button);
  mount(h('div', { class: 'screen' }, brand(), h('h1', { text: 'Connexion' }),
    h('p', { class: 'lead', text: "Vos statistiques restent sur votre serveur. Connectez-vous pour les consulter." }), form));
}

// ——— Nouveau projet et installation du traceur ———

function renderNewProject(first = false) {
  const admin = me.organizations.filter((o) => o.role === 'owner' || o.role === 'admin');
  if (!admin.length) {
    return mount(h('div', { class: 'screen' }, brand(), h('h1', { text: 'Aucun site pour le moment' }),
      h('p', { class: 'lead', text: "Demandez à un administrateur de votre organisation d'ajouter un site." })));
  }
  const error = h('p', { class: 'error', role: 'alert' });
  const button = h('button', { class: 'btn', type: 'submit', text: 'Ajouter le site' });
  const zone = Intl.DateTimeFormat().resolvedOptions().timeZone || 'Europe/Paris';
  const form = h('form', { class: 'form', onsubmit: async (event) => {
    event.preventDefault();
    button.disabled = true;
    error.textContent = '';
    const data = new FormData(form);
    try {
      const project = await api(`/api/orgs/${data.get('org')}/projects`, { method: 'POST', body: {
        name: data.get('name'), timezone: data.get('timezone'), domains: [data.get('domain')].filter(Boolean),
      } });
      me = await api('/api/me');
      go(`p/${project.id}/installation`);
    } catch (e) {
      error.textContent = e.message;
      button.disabled = false;
    }
  } },
  admin.length > 1
    ? h('label', {}, 'Organisation', h('select', { class: 'field', name: 'org' }, admin.map((o) => h('option', { value: o.id, text: o.name }))))
    : h('input', { type: 'hidden', name: 'org', value: admin[0].id }),
  h('label', {}, 'Nom du site', h('input', { class: 'field', name: 'name', required: true, maxlength: 100, placeholder: 'Boutique en ligne…', autocomplete: 'off' })),
  h('label', {}, 'Domaine', h('input', { class: 'field', name: 'domain', placeholder: 'exemple.fr…', autocomplete: 'off', spellcheck: 'false', inputmode: 'url' }),
    h('small', { text: 'Seules les pages de ce domaine et de ses sous-domaines seront comptées. Laissez vide pour tout accepter.' })),
  h('label', {}, 'Fuseau horaire', h('input', { class: 'field', name: 'timezone', value: zone, required: true, spellcheck: 'false', autocomplete: 'off' }),
    h('small', { text: 'Une journée de statistiques commence à minuit dans ce fuseau.' })),
  error,
  h('div', { class: 'row-actions' }, button, first ? null : h('a', { class: 'btn quiet', href: '#', text: 'Annuler' })));
  mount(h('div', { class: 'screen' }, brand(), h('h1', { text: first ? 'Ajoutez votre premier site' : 'Ajouter un site' }),
    h('p', { class: 'lead', text: 'Synalyse vous donnera ensuite la ligne de code à poser sur vos pages.' }), form));
}

async function renderInstall(summary) {
  const token = ++renderToken;
  let project;
  try {
    project = await api(`/api/projects/${summary.id}`);
  } catch (error) {
    return mount(h('div', { class: 'screen' }, brand(), h('p', { class: 'error', text: error.message })));
  }
  if (token !== renderToken) return;
  const code = `<script async src="${location.origin}/c.js" data-key="${project.public_key}"></script>`;
  const copy = h('button', { class: 'btn quiet', type: 'button', text: 'Copier le code', onclick: async () => {
    try { await navigator.clipboard.writeText(code); copy.textContent = 'Code copié'; } catch { copy.textContent = 'Sélectionnez le code pour le copier'; }
  } });
  mount(h('div', { class: 'screen wide' }, brand(),
    h('h1', { text: `Installer Synalyse sur ${project.name}` }),
    h('p', { class: 'lead', text: 'Collez cette ligne dans le <head> de toutes les pages à mesurer.' }),
    h('pre', { class: 'snippet', tabindex: '0', translate: 'no' }, h('code', { text: code })),
    h('div', { class: 'row-actions' }, copy, h('a', { class: 'btn', href: `#p/${project.id}`, text: 'Voir les statistiques' })),
    h('ol', { class: 'steps' },
      h('li', {}, h('strong', { text: 'Aucun bandeau de cookies à ajouter. ' }), "Par défaut, Synalyse ne dépose rien dans le navigateur et ne reconnaît pas un visiteur d'un jour à l'autre."),
      h('li', {}, h('strong', { text: 'Mesurer une action. ' }), "Appelez synalyse('event', 'Inscription') au moment où elle se produit."),
      h('li', {}, h('strong', { text: "Permettre l'opposition. " }), "Reliez un bouton de votre politique de confidentialité à synalyse('optout')."))));
}

// ——— Cadre commun aux écrans d'un site : barre du haut, sections, période, filtres ———

const SECTIONS = [['', 'Audience'], ['entonnoirs', 'Entonnoirs'], ['parcours', 'Parcours'], ['enregistrements', 'Enregistrements']];
const SHARED_PARAMS = ['period', 'from', 'to', ...Object.keys(FILTER_NAMES)];

function shell(project, params, section, { dated = true } = {}) {
  const token = ++renderToken;
  const period = resolvePeriod(params, project.timezone);
  const filters = Object.keys(FILTER_NAMES).filter((key) => params.has(key)).map((key) => [key, params.get(key)]);
  const root = `/api/projects/${project.id}`;
  const filterQuery = filters.map(([k, v]) => `&${k}=${encodeURIComponent(v)}`).join('');
  const range = (from = period.from, to = period.to) => `from=${from}&to=${to}${filterQuery}`;
  const pathOf = (name) => `p/${project.id}${name ? '/' + name : ''}`;
  /** Paramètres qui suivent l'utilisateur d'une section à l'autre : période et filtres. */
  const shared = () => new URLSearchParams([...params].filter(([key]) => SHARED_PARAMS.includes(key)));

  /** Adresse du même écran avec des paramètres modifiés (null supprime) : sert aux liens, donc au clic milieu. */
  const href = (changes) => {
    const next = new URLSearchParams(params);
    for (const [key, value] of Object.entries(changes)) {
      if (value === null) next.delete(key); else next.set(key, value);
    }
    const query = next.toString();
    return `#${pathOf(section)}${query ? '?' + query : ''}`;
  };
  const update = (changes) => { location.hash = href(changes); };

  const live = h('span', { class: 'live', role: 'status' }, h('i'), h('span', { text: 'Temps réel…' }));
  const periodSelect = h('select', { class: 'field', 'aria-label': 'Période', onchange: (event) => {
    const value = event.target.value;
    update(value === 'custom' ? { period: 'custom', from: period.from, to: period.to } : { period: value, from: null, to: null });
  } }, PERIODS.map(([key, label]) => h('option', { value: key, text: label, selected: key === period.preset })));
  const custom = period.preset === 'custom' ? [
    h('input', { class: 'field', type: 'date', 'aria-label': 'Du', value: period.from, max: period.to, onchange: (e) => validDay(e.target.value) && update({ from: e.target.value }) }),
    h('input', { class: 'field', type: 'date', 'aria-label': 'Au', value: period.to, min: period.from, onchange: (e) => validDay(e.target.value) && update({ to: e.target.value }) }),
  ] : [];
  const projectPicker = me.projects.length > 1
    ? h('select', { class: 'field site', 'aria-label': 'Site', onchange: (e) => go(`p/${e.target.value}${section ? '/' + section : ''}`, new URLSearchParams([...shared()].filter(([key]) => ['period', 'from', 'to'].includes(key)))) },
      me.projects.map((p) => h('option', { value: p.id, text: p.name, selected: p.id === project.id })))
    : h('strong', { text: project.name });
  const topbar = h('header', { class: 'topbar' }, brand(), projectPicker, live, h('span', { class: 'spacer' }), ...(dated ? [...custom, periodSelect] : []),
    h('details', { class: 'menu' }, h('summary', { class: 'field', text: 'Réglages' }), h('div', { class: 'menu-items' },
      h('a', { href: `#${pathOf('installation')}`, text: 'Installer le code de suivi' }),
      h('a', { href: '#new', text: 'Ajouter un site' }),
      h('button', { type: 'button', text: 'Se déconnecter', onclick: async () => {
        await api('/api/auth/logout', { method: 'POST' }).catch(() => {});
        me = null;
        renderLogin();
      } }))));
  const nav = h('nav', { class: 'sections', 'aria-label': 'Sections' }, SECTIONS.map(([name, label]) => {
    const query = shared().toString();
    return h('a', { href: `#${pathOf(name)}${query ? '?' + query : ''}`, 'aria-current': name === section ? 'page' : null, text: label });
  }));
  const chips = h('div', { class: 'filters' }, (dated ? filters : []).map(([key, value]) => h('span', { class: 'chip' },
    h('span', { text: FILTER_NAMES[key] }), displayLabel(key, value),
    h('button', { type: 'button', 'aria-label': `Retirer le filtre ${FILTER_NAMES[key]}`, text: '×', onclick: () => update({ [key]: null }) }))));

  const stale = () => token !== renderToken;
  const fail = (target) => (error) => {
    if (stale()) return;
    if (error.status === 401) { me = null; return renderLogin(); }
    target.replaceChildren(h('p', { class: 'empty', text: error.message }));
  };
  // Temps réel, rafraîchi toutes les trente secondes tant que l'écran est affiché.
  const refreshLive = () => api(`${root}/stats/realtime`).then((data) => {
    if (stale()) return;
    const n = data.last_5_minutes;
    live.classList.toggle('on', n > 0);
    live.lastChild.replaceChildren(h('b', { text: nf.format(n) }), n > 1 ? ' visiteurs en ce moment' : ' visiteur en ce moment');
  }).catch(() => {});
  const show = (...nodes) => {
    const title = `${SECTIONS.find(([name]) => name === section)[1]} de ${project.name}`;
    document.title = `${title} · Synalyse`;
    mount(h('a', { class: 'skip', href: '#contenu', text: 'Aller au contenu', onclick: (e) => { e.preventDefault(); document.getElementById('contenu').focus(); } }),
      topbar, h('main', { id: 'contenu', tabindex: '-1' }, h('h1', { class: 'sr-only', text: title }), nav, chips, ...nodes));
    refreshLive();
    liveTimer = setInterval(() => document.visibilityState === 'visible' && refreshLive(), 30000);
  };
  return { project, params, period, root, range, update, href, stale, fail, show };
}

// ——— Audience ———

function renderDashboard(project, params) {
  const ctx = shell(project, params, '');
  const { period, href, stale, fail, range } = ctx;
  const base = `${ctx.root}/stats`;
  const metric = METRICS.some((m) => m.chart && m.key === params.get('metric')) ? params.get('metric') : 'visitors';

  // Dalle : chiffres clés et courbe
  const metricEls = METRICS.map((m) => {
    const value = h('span', { class: 'value', text: '–' });
    const delta = h('span', { class: 'delta' });
    const el = m.chart
      ? h('a', { class: 'metric', href: href({ metric: m.key }), 'aria-current': m.key === metric ? 'true' : null, title: `Afficher la courbe : ${m.label.toLowerCase()}` }, h('span', { class: 'label', text: m.label }), value, delta)
      : h('div', { class: 'metric' }, h('span', { class: 'label', text: m.label }), value, delta);
    return { ...m, el, value, delta };
  });
  const chart = h('div', { class: 'chart' });
  const slab = h('section', { class: 'slab', 'aria-label': 'Chiffres clés' }, h('div', { class: 'metrics' }, metricEls.map((m) => m.el)), chart);

  // Panneaux
  const panels = PANELS.map((panel) => {
    const stateKey = 'tab_' + panel.tabs[0][0];
    const active = panel.tabs.find(([dim]) => dim === params.get(stateKey)) || panel.tabs[0];
    const body = h('div', {}, h('p', { class: 'empty', text: 'Chargement…' }));
    const tabs = panel.tabs.length > 1 ? h('div', { class: 'tabs' }, panel.tabs.map(([dim, label]) =>
      h('a', { href: href({ [stateKey]: dim }), 'aria-current': dim === active[0] ? 'true' : null, text: label }))) : null;
    return { dim: active[0], body, el: h('section', { class: 'slab panel' }, h('header', {}, h('h2', { text: panel.title }), tabs), body) };
  });

  ctx.show(slab, h('div', { class: 'panels' }, panels.map((p) => p.el)));

  // Chiffres clés : période demandée et période précédente de même longueur.
  const interval = period.hourly ? '&interval=hour' : '';
  Promise.all([api(`${base}/summary?${range(period.from, period.to)}${interval}`), api(`${base}/summary?${range(period.prevFrom, period.prevTo)}${interval}`)])
    .then(([current, previous]) => {
      if (stale()) return;
      for (const m of metricEls) {
        const format = m.format || ((v) => nf.format(v));
        m.value.textContent = format(current[m.key]);
        const before = previous[m.key];
        if (before > 0 && current.sessions > 0) {
          const change = (current[m.key] - before) / before;
          const good = m.lowerIsBetter ? change < 0 : change > 0;
          m.delta.textContent = `${change > 0 ? '+' : change < 0 ? '−' : ''}${pf.format(Math.abs(change))}`;
          m.delta.title = 'Par rapport à la période précédente de même durée';
          m.delta.className = 'delta' + (Math.abs(change) < 0.001 ? '' : good ? ' up' : ' down');
        } else {
          m.delta.textContent = '';
        }
      }
      drawChart(chart, seriesOf(current, period, metric, period.from), seriesOf(previous, period, metric, period.prevFrom), METRICS.find((m) => m.key === metric).label, current.sessions === 0);
      for (const panel of panels) loadPanel(panel, current.visitors);
    })
    .catch(fail(chart));

  function loadPanel(panel, totalVisitors) {
    api(`${base}/breakdown?dim=${panel.dim}&limit=9&${range()}`)
      .then((data) => {
        if (stale()) return;
        if (!data.rows.length) {
          return panel.body.replaceChildren(h('p', { class: 'empty', text: panel.dim === 'event' ? "Aucun événement sur cette période. Envoyez-en avec synalyse('event', 'Nom')." : 'Aucune donnée sur cette période.' }));
        }
        const max = Math.max(...data.rows.map((r) => r.visitors));
        const canFilter = panel.dim in FILTER_NAMES;
        panel.body.replaceChildren(h('ul', { class: 'rows' },
          h('li', { class: 'head' }, h('span', { text: '' }), h('span', { text: 'Visiteurs' })),
          data.rows.map((row) => {
            const content = [h('span', { class: 'name' }, displayLabel(panel.dim, row.label)), h('span', { class: 'num', text: nf.format(row.visitors) }),
              h('span', { class: 'pct', text: totalVisitors ? pf.format(row.visitors / totalVisitors) : '' })];
            const el = canFilter
              ? h('a', { class: 'row', href: href({ [panel.dim]: row.label }), title: 'Filtrer sur cette valeur' }, content)
              : h('div', { class: 'row' }, content);
            el.style.setProperty('--part', `${(row.visitors / max) * 100}%`);
            return h('li', {}, el);
          })));
      })
      .catch(fail(panel.body));
  }
}

function displayLabel(dim, label) {
  if (dim === 'channel') return CHANNELS[label] ?? label;
  if (dim === 'device') return DEVICES[label] ?? label;
  if (label === '') return dim === 'referrer' ? 'Aucun site référent' : dim === 'utm_campaign' ? 'Hors campagne' : '(non renseigné)';
  return label;
}

/** Série complète de la période : les jours (ou heures) sans visite valent zéro. */
function seriesOf(summary, period, metric, from) {
  if (period.hourly) {
    const byHour = new Map((summary.by_hour || []).map((p) => [p.hour, p[metric]]));
    return Array.from({ length: 24 }, (_, hour) => ({ label: `${hour} h`, title: `${dayLabel(from, longDay)}, ${hour} h`, value: byHour.get(hour) || 0 }));
  }
  const byDay = new Map((summary.by_day || []).map((p) => [p.day, p[metric]]));
  const length = daysBetween(period.from, period.to) + 1;
  return Array.from({ length }, (_, i) => {
    const day = addDays(from, i);
    return { label: dayLabel(day), title: dayLabel(day, longDay), value: byDay.get(day) || 0 };
  });
}

/** Plafond de l'axe vertical : quatre intervalles égaux, toujours entiers. */
function niceMax(value) {
  const target = Math.max(1, value / 4);
  const pow = 10 ** Math.floor(Math.log10(target));
  const step = [1, 2, 5, 10].map((m) => m * pow).find((candidate) => candidate >= target);
  return Math.max(1, Math.ceil(step)) * 4;
}

function drawChart(container, points, previous, metricLabel, empty) {
  if (empty) {
    return container.replaceChildren(h('p', { class: 'empty', text: "Aucune visite sur cette période. Si le site vient d'être ajouté, vérifiez que le code est bien posé sur vos pages." }));
  }
  const W = Math.max(320, container.clientWidth - 24); const H = 260;
  const pad = { top: 10, right: 12, bottom: 26, left: 44 };
  const max = niceMax(Math.max(1, ...points.map((p) => p.value), ...previous.map((p) => p.value)));
  const x = (i) => pad.left + (points.length === 1 ? (W - pad.left - pad.right) / 2 : (i * (W - pad.left - pad.right)) / (points.length - 1));
  const y = (v) => pad.top + (1 - v / max) * (H - pad.top - pad.bottom);
  const path = (list) => list.map((p, i) => `${i ? 'L' : 'M'}${x(i).toFixed(1)},${y(p.value).toFixed(1)}`).join('');

  const root = svg('svg', { viewBox: `0 0 ${W} ${H}`, role: 'img', 'aria-label': `${metricLabel} sur la période : ${nf.format(points.reduce((s, p) => s + p.value, 0))} au total, maximum ${nf.format(Math.max(...points.map((p) => p.value)))}.` });
  for (let i = 0; i <= 4; i++) {
    const value = (max / 4) * i;
    root.append(svg('line', { class: 'grid', x1: pad.left, x2: W - pad.right, y1: y(value), y2: y(value) }));
    const text = svg('text', { class: 'axis', x: pad.left - 8, y: y(value) + 4, 'text-anchor': 'end' });
    text.textContent = nf.format(value);
    root.append(text);
  }
  const every = Math.max(1, Math.ceil(points.length / Math.max(2, Math.floor(W / 90))));
  points.forEach((p, i) => {
    if (i % every !== 0 || (points.length - 1 - i < every / 2 && i !== points.length - 1 && points.length > every)) return;
    const text = svg('text', { class: 'axis', x: x(i), y: H - 6, 'text-anchor': i === 0 ? 'start' : 'middle' });
    text.textContent = p.label;
    root.append(text);
  });
  if (previous.some((p) => p.value > 0)) root.append(svg('path', { class: 'prev', d: path(previous.slice(0, points.length)) }));
  const base = y(0);
  root.append(svg('path', { class: 'area', d: `${path(points)}L${x(points.length - 1).toFixed(1)},${base}L${x(0).toFixed(1)},${base}Z` }));
  root.append(svg('path', { class: 'line', d: path(points) }));
  const cursor = svg('line', { class: 'cursor', y1: pad.top, y2: base, visibility: 'hidden' });
  const dot = svg('circle', { class: 'dot', r: 4, visibility: 'hidden' });
  root.append(cursor, dot);

  const tip = h('div', { class: 'tip', hidden: true });
  const show = (clientX) => {
    const box = root.getBoundingClientRect();
    const ratio = (clientX - box.left) / box.width;
    const px = ratio * W;
    const i = points.length === 1 ? 0 : Math.max(0, Math.min(points.length - 1, Math.round(((px - pad.left) / (W - pad.left - pad.right)) * (points.length - 1))));
    cursor.setAttribute('x1', x(i)); cursor.setAttribute('x2', x(i));
    dot.setAttribute('cx', x(i)); dot.setAttribute('cy', y(points[i].value));
    cursor.setAttribute('visibility', 'visible'); dot.setAttribute('visibility', 'visible');
    tip.replaceChildren(h('span', { text: points[i].title }), h('strong', { text: `${nf.format(points[i].value)} ${metricLabel.toLowerCase()}` }),
      previous[i] && previous.some((p) => p.value > 0) ? h('span', { text: `${nf.format(previous[i].value)} sur la période précédente` }) : null);
    tip.hidden = false;
    const left = Math.max(90, Math.min(box.width - 90, (x(i) / W) * box.width));
    tip.style.left = `${left + 12}px`;
    tip.style.top = `${(y(points[i].value) / H) * box.height + 6}px`;
  };
  const hide = () => { tip.hidden = true; cursor.setAttribute('visibility', 'hidden'); dot.setAttribute('visibility', 'hidden'); };
  root.addEventListener('pointermove', (e) => show(e.clientX));
  root.addEventListener('pointerleave', hide);

  const legend = h('div', { class: 'legend' }, h('span', {}, h('i'), 'Période affichée'),
    previous.some((p) => p.value > 0) ? h('span', {}, h('i', { class: 'dashed' }), 'Période précédente') : null);
  container.replaceChildren(legend, root, tip);
}

// ——— Entonnoirs ———

const STEP_KINDS = [['page', 'Page vue'], ['event', 'Événement']];
const STEP_MATCHES = [['equals', 'est'], ['starts_with', 'commence par'], ['contains', 'contient'], ['ends_with', 'finit par'], ['regex', 'correspond à (regex)']];
/** Brouillon en cours d'édition : survit aux rechargements de l'écran, pas à celui de la page. */
let funnelDraft = null;

async function renderFunnels(project, params) {
  const ctx = shell(project, params, 'entonnoirs');
  const body = h('div', {}, h('p', { class: 'empty', text: 'Chargement…' }));
  ctx.show(body);
  let funnels;
  try {
    funnels = (await api(`${ctx.root}/funnels`)).funnels;
  } catch (error) {
    return ctx.fail(body)(error);
  }
  if (ctx.stale()) return;

  const editing = params.get('edit');
  if (editing) return funnelEditor(ctx, body, funnels, editing === 'new' ? null : funnels.find((f) => String(f.id) === editing));
  funnelDraft = null;

  if (!funnels.length) {
    return body.replaceChildren(h('section', { class: 'slab blank' },
      h('h1', { text: 'Voyez où vos visiteurs abandonnent' }),
      h('p', { text: "Un entonnoir suit une suite d'étapes, par exemple accueil, fiche produit, panier, commande, et compte les visites qui franchissent chacune." }),
      h('button', { class: 'btn', type: 'button', text: 'Créer un entonnoir', onclick: () => ctx.update({ edit: 'new' }) })));
  }
  const current = funnels.find((f) => String(f.id) === params.get('funnel')) || funnels[0];
  const report = h('div', {}, h('p', { class: 'empty', text: 'Calcul…' }));
  body.replaceChildren(
    h('div', { class: 'toolbar' },
      h('select', { class: 'field', 'aria-label': 'Entonnoir', onchange: (e) => ctx.update({ funnel: e.target.value }) },
        funnels.map((f) => h('option', { value: f.id, text: f.name, selected: f.id === current.id }))),
      h('button', { class: 'btn quiet', type: 'button', text: 'Modifier', onclick: () => ctx.update({ edit: String(current.id) }) }),
      h('button', { class: 'btn quiet', type: 'button', text: 'Nouvel entonnoir', onclick: () => ctx.update({ edit: 'new' }) })),
    report);
  api(`${ctx.root}/funnels/${current.id}/report?${ctx.range()}`)
    .then((data) => !ctx.stale() && report.replaceChildren(funnelReport(data)))
    .catch(ctx.fail(report));
}

function funnelReport(data) {
  if (!data.entered) {
    return h('section', { class: 'slab blank' }, h('p', { text: data.sessions_in_period
      ? `Aucune des ${nf.format(data.sessions_in_period)} visites de la période n'a franchi la première étape. Vérifiez son motif.`
      : 'Aucune visite sur cette période.' }));
  }
  const steps = data.steps.map((step, i) => {
    const bar = h('div', { class: 'funnel-bar' }, h('i'));
    bar.firstChild.style.width = `${Math.max(0.5, step.rate_from_start * 100)}%`;
    if (i > 0) {
      const lost = h('b');
      lost.style.left = `${step.rate_from_start * 100}%`;
      lost.style.width = `${(data.steps[i - 1].rate_from_start - step.rate_from_start) * 100}%`;
      bar.append(lost);
    }
    const drop = step.dropped > 0 ? h('p', { class: 'funnel-drop' },
      h('strong', { text: `${nf.format(step.dropped)} ${step.dropped > 1 ? 'abandons' : 'abandon'}` }),
      ` (${pf.format(step.dropped / step.sessions)}) après cette étape : `,
      step.next_after_drop.map((n) => n.exit ? `${nf.format(n.sessions)} quittent le site` : `${nf.format(n.sessions)} vont sur ${n.path}`).join(', '), '.') : null;
    return h('li', { class: 'funnel-step' },
      h('div', { class: 'funnel-head' },
        h('span', { class: 'funnel-name' }, h('span', { class: 'funnel-index', text: String(i + 1) }), step.name),
        h('span', { class: 'funnel-figures' }, h('strong', { text: nf.format(step.sessions) }), step.sessions > 1 ? ' visites' : ' visite',
          i > 0 ? h('span', { text: ` · ${pf.format(step.rate_from_previous)} de l'étape précédente` }) : null,
          i > 0 && step.median_seconds_from_previous !== null ? h('span', { text: ` · ${duration(step.median_seconds_from_previous)} en médiane` }) : null)),
      bar, drop);
  });
  return h('section', { class: 'slab funnel' },
    h('p', { class: 'funnel-summary' }, h('strong', { text: pf.format(data.conversion_rate) }),
      ` des visites entrées dans l'entonnoir vont au bout : ${nf.format(data.converted)} sur ${nf.format(data.entered)}.`),
    h('ol', { class: 'funnel-steps' }, steps));
}

async function funnelEditor(ctx, body, funnels, existing) {
  if (!funnelDraft || funnelDraft.id !== (existing?.id ?? null)) {
    funnelDraft = existing
      ? { id: existing.id, name: existing.name, steps: existing.steps.map((s) => ({ ...s })) }
      : { id: null, name: '', steps: [{ name: '', kind: 'page', match: 'equals', pattern: '' }, { name: '', kind: 'page', match: 'equals', pattern: '' }] };
  }
  const draft = funnelDraft;
  const error = h('p', { class: 'error', role: 'alert' });
  const preview = h('div', {});
  const list = h('ol', { class: 'step-editor' });
  const pagesList = h('datalist', { id: 'known-pages' });
  const eventsList = h('datalist', { id: 'known-events' });
  // Aide à la saisie : pages et événements réellement vus sur la période.
  for (const [dim, target] of [['page', pagesList], ['event', eventsList]]) {
    api(`${ctx.root}/stats/breakdown?dim=${dim}&limit=100&${ctx.range()}`)
      .then((data) => target.replaceChildren(...data.rows.map((r) => h('option', { value: r.label }))))
      .catch(() => {});
  }
  const drawSteps = () => list.replaceChildren(...draft.steps.map((step, i) => h('li', {},
    h('span', { class: 'funnel-index', text: String(i + 1) }),
    h('select', { class: 'field', 'aria-label': `Type de l'étape ${i + 1}`, onchange: (e) => { step.kind = e.target.value; drawSteps(); } },
      STEP_KINDS.map(([value, label]) => h('option', { value, text: label, selected: step.kind === value }))),
    h('select', { class: 'field', 'aria-label': `Comparaison de l'étape ${i + 1}`, onchange: (e) => { step.match = e.target.value; } },
      STEP_MATCHES.map(([value, label]) => h('option', { value, text: label, selected: step.match === value }))),
    h('input', { class: 'field grow', 'aria-label': `Motif de l'étape ${i + 1}`, value: step.pattern, placeholder: step.kind === 'page' ? '/panier…' : "Nom de l'événement…", autocomplete: 'off', name: `motif-${i + 1}`,
      list: step.kind === 'page' ? 'known-pages' : 'known-events', spellcheck: 'false', oninput: (e) => { step.pattern = e.target.value; } }),
    h('input', { class: 'field', 'aria-label': `Nom de l'étape ${i + 1}`, value: step.name, placeholder: 'Nom affiché, facultatif…', autocomplete: 'off', name: `nom-${i + 1}`, maxlength: 80, oninput: (e) => { step.name = e.target.value; } }),
    h('button', { class: 'btn quiet', type: 'button', disabled: draft.steps.length <= 2, 'aria-label': `Retirer l'étape ${i + 1}`, text: 'Retirer',
      onclick: () => { draft.steps.splice(i, 1); drawSteps(); } }))));
  drawSteps();

  const guard = (action) => async () => {
    error.textContent = '';
    try { await action(); } catch (e) { if (e.status === 401) { me = null; return renderLogin(); } error.textContent = e.message; }
  };
  const close = (changes) => { funnelDraft = null; ctx.update({ edit: null, ...changes }); };
  const payload = () => ({ name: draft.name, steps: draft.steps });

  body.replaceChildren(h('section', { class: 'slab editor' },
    h('h1', { text: existing ? `Modifier « ${existing.name} »` : 'Nouvel entonnoir' }),
    h('label', { class: 'editor-name' }, 'Nom', h('input', { class: 'field', name: 'nom-entonnoir', autocomplete: 'off', value: draft.name, maxlength: 100, placeholder: 'Achat en ligne…', oninput: (e) => { draft.name = e.target.value; } })),
    h('p', { class: 'hint', text: "Une visite franchit une étape si elle la rencontre après la précédente ; d'autres pages peuvent s'intercaler." }),
    list, pagesList, eventsList,
    h('button', { class: 'btn quiet', type: 'button', text: 'Ajouter une étape', disabled: draft.steps.length >= 12,
      onclick: () => { draft.steps.push({ name: '', kind: 'page', match: 'equals', pattern: '' }); drawSteps(); } }),
    error,
    h('div', { class: 'row-actions' },
      h('button', { class: 'btn', type: 'button', text: 'Enregistrer', onclick: guard(async () => {
        const saved = existing
          ? await api(`${ctx.root}/funnels/${existing.id}`, { method: 'PUT', body: payload() })
          : await api(`${ctx.root}/funnels`, { method: 'POST', body: payload() });
        close({ funnel: String(saved.id) });
      }) }),
      h('button', { class: 'btn quiet', type: 'button', text: 'Essayer sans enregistrer', onclick: guard(async () => {
        preview.replaceChildren(h('p', { class: 'empty', text: 'Calcul…' }));
        try {
          preview.replaceChildren(funnelReport(await api(`${ctx.root}/funnels/preview?${ctx.range()}`, { method: 'POST', body: { steps: draft.steps } })));
        } catch (e) { preview.replaceChildren(); throw e; }
      }) }),
      h('button', { class: 'btn quiet', type: 'button', text: 'Annuler', onclick: () => close({}) }),
      existing ? h('button', { class: 'btn quiet danger', type: 'button', text: 'Supprimer', onclick: guard(async () => {
        if (!confirm(`Supprimer l'entonnoir « ${existing.name} » ?`)) return;
        await api(`${ctx.root}/funnels/${existing.id}`, { method: 'DELETE' });
        close({ funnel: null });
      }) }) : null)),
  preview);
}

// ——— Parcours ———

const EXIT = '__sortie__';
const OTHERS = '__autres__';
const pathLabel = (label) => (label === EXIT ? 'Sortie du site' : label === OTHERS ? 'Autres pages' : label);

async function renderPaths(project, params) {
  const ctx = shell(project, params, 'parcours');
  const depth = Math.min(8, Math.max(2, Number(params.get('depth')) || 4));
  const start = params.get('start') || '';
  const startSelect = h('select', { class: 'field', 'aria-label': 'Point de départ', onchange: (e) => ctx.update({ start: e.target.value || null }) },
    h('option', { value: '', text: "Depuis l'entrée sur le site" }), start ? h('option', { value: start, text: `Depuis ${start}`, selected: true }) : null);
  const depthSelect = h('select', { class: 'field', 'aria-label': "Nombre d'étapes", onchange: (e) => ctx.update({ depth: e.target.value }) },
    [3, 4, 5, 6].map((n) => h('option', { value: n, text: `${n} étapes`, selected: n === depth })));
  const diagram = h('div', { class: 'paths' }, h('p', { class: 'empty', text: 'Calcul…' }));
  const note = h('p', { class: 'hint' });
  ctx.show(h('div', { class: 'toolbar' }, startSelect, depthSelect), h('section', { class: 'slab' }, diagram), note);

  api(`${ctx.root}/stats/breakdown?dim=page&limit=40&${ctx.range()}`).then((data) => {
    if (ctx.stale()) return;
    for (const row of data.rows) if (row.label !== start) startSelect.append(h('option', { value: row.label, text: `Depuis ${row.label}` }));
  }).catch(() => {});

  api(`${ctx.root}/paths?depth=${depth}&max_nodes=6&start=${encodeURIComponent(start)}&${ctx.range()}`).then((data) => {
    if (ctx.stale()) return;
    if (!data.matched) {
      return diagram.replaceChildren(h('p', { class: 'empty', text: start ? `Aucune visite n'est passée par ${start} sur cette période.` : 'Aucune visite sur cette période.' }));
    }
    note.textContent = start
      ? `${nf.format(data.matched)} visites sur ${nf.format(data.sessions_in_period)} sont passées par ${start}. Cliquez sur une page pour repartir de là.`
      : `${nf.format(data.matched)} visites. L'épaisseur d'un lien est proportionnelle au nombre de visites ; cliquez sur une page pour repartir de là.`;
    drawPaths(diagram, data, (label) => ctx.update({ start: label }));
  }).catch(ctx.fail(diagram));
}

function drawPaths(container, data, onPick) {
  // Colonnes : les pages de chaque niveau, puis les sorties venues du niveau précédent.
  const columns = data.levels.map((level) => level.map((n) => ({ label: n.label, sessions: n.sessions })));
  columns.push([]);
  for (const link of data.links) {
    if (link.to !== EXIT) continue;
    const column = columns[link.level + 1];
    const node = column.find((n) => n.label === EXIT) || column[column.push({ label: EXIT, sessions: 0 }) - 1];
    node.sessions += link.sessions;
  }
  while (columns.length && !columns.at(-1).length) columns.pop();

  const nodeW = 10; const gap = 14; const labelW = 170;
  const W = Math.max(columns.length * (nodeW + labelW), container.clientWidth - 32);
  const H = 420; const top = 8;
  const tallest = Math.max(...columns.map((c) => c.reduce((s, n) => s + n.sessions, 0)));
  const scale = (H - top - gap * (Math.max(...columns.map((c) => c.length)) - 1)) / tallest;
  const colX = (i) => (columns.length === 1 ? 0 : (i * (W - nodeW - labelW)) / (columns.length - 1));
  columns.forEach((column, i) => {
    let y = top;
    for (const node of column) {
      Object.assign(node, { x: colX(i), y, h: Math.max(2, node.sessions * scale), out: 0, in: 0 });
      y += node.h + gap;
    }
  });

  const root = svg('svg', { viewBox: `0 0 ${W} ${H}`, width: W, height: H, role: 'img', 'aria-label': 'Diagramme des parcours de pages, niveau par niveau.' });
  const find = (level, label) => columns[level]?.find((n) => n.label === label);
  // Liens triés selon l'ordre des pages d'arrivée : moins de croisements.
  const links = [...data.links].sort((a, b) => a.level - b.level
    || columns[a.level].findIndex((n) => n.label === a.from) - columns[b.level].findIndex((n) => n.label === b.from)
    || columns[a.level + 1].findIndex((n) => n.label === a.to) - columns[b.level + 1].findIndex((n) => n.label === b.to));
  for (const link of links) {
    const from = find(link.level, link.from); const to = find(link.level + 1, link.to);
    if (!from || !to) continue;
    const t = Math.max(1, link.sessions * scale);
    const x1 = from.x + nodeW; const x2 = to.x; const mid = (x1 + x2) / 2;
    const y1 = from.y + from.out; const y2 = to.y + to.in;
    from.out += t; to.in += t;
    const band = svg('path', { class: link.to === EXIT ? 'flow exit' : 'flow',
      d: `M${x1},${y1}C${mid},${y1} ${mid},${y2} ${x2},${y2}L${x2},${y2 + t}C${mid},${y2 + t} ${mid},${y1 + t} ${x1},${y1 + t}Z` });
    const title = svg('title');
    title.textContent = `${pathLabel(link.from)} → ${pathLabel(link.to)} : ${nf.format(link.sessions)} ${link.sessions > 1 ? 'visites' : 'visite'}`;
    band.append(title);
    root.append(band);
  }
  columns.forEach((column) => {
    for (const node of column) {
      const special = node.label === EXIT || node.label === OTHERS;
      const group = svg('g', { class: `node${node.label === EXIT ? ' exit' : ''}${special ? '' : ' pick'}` });
      group.append(svg('rect', { x: node.x, y: node.y, width: nodeW, height: node.h, rx: 2 }));
      const name = svg('text', { x: node.x + nodeW + 8, y: node.y + Math.min(node.h / 2, 14) + 4, class: 'node-name' });
      const text = pathLabel(node.label);
      name.textContent = text.length > 22 ? text.slice(0, 21) + '…' : text;
      const count = svg('text', { x: node.x + nodeW + 8, y: node.y + Math.min(node.h / 2, 14) + 20, class: 'node-count' });
      count.textContent = nf.format(node.sessions);
      const title = svg('title');
      title.textContent = `${text} : ${nf.format(node.sessions)} ${node.sessions > 1 ? 'visites' : 'visite'}`;
      group.append(title, name);
      if (node.h >= 26) group.append(count);
      if (!special) {
        group.setAttribute('tabindex', '0');
        group.setAttribute('role', 'button');
        group.setAttribute('aria-label', `Repartir de ${text}`);
        group.addEventListener('click', () => onPick(node.label));
        group.addEventListener('keydown', (e) => (e.key === 'Enter' || e.key === ' ') && (e.preventDefault(), onPick(node.label)));
      }
      root.append(group);
    }
  });
  container.replaceChildren(root);
}

// ——— Enregistrements de session ———

const fullDate = new Intl.DateTimeFormat('fr-FR', { day: 'numeric', month: 'short', hour: '2-digit', minute: '2-digit' });
const megabytes = (bytes) => (bytes < 104858 ? `${nf.format(Math.ceil(bytes / 1024))} Ko` : `${new Intl.NumberFormat('fr-FR', { maximumFractionDigits: bytes < 10485760 ? 1 : 0 }).format(bytes / 1048576)} Mo`);
const recordingPlace = (rec) => [DEVICES[rec.device], rec.browser, rec.os].filter((part) => part && part !== 'Inconnu').join(', ');

async function renderRecordings(project, params) {
  const ctx = shell(project, params, 'enregistrements', { dated: false });
  const body = h('div', {}, h('p', { class: 'empty', text: 'Chargement…' }));
  ctx.show(body);
  const selected = params.get('rec');
  try {
    if (selected) {
      const rec = await api(`${ctx.root}/recordings/${encodeURIComponent(selected)}`);
      if (!ctx.stale()) body.replaceChildren(...recordingPlayer(ctx, rec).filter(Boolean));
      return;
    }
    const minActive = [0, 10, 60].includes(Number(params.get('actif'))) ? Number(params.get('actif')) : 0;
    const visitor = /^\d+$/.test(params.get('visiteur') || '') ? params.get('visiteur') : '';
    const [settings, list] = await Promise.all([
      api(ctx.root),
      api(`${ctx.root}/recordings?limit=50&min_active=${minActive}${visitor ? '&visitor=' + visitor : ''}`),
    ]);
    if (ctx.stale()) return;
    body.replaceChildren(...recordingList(ctx, settings.recording, list, minActive, visitor).filter(Boolean));
  } catch (error) {
    ctx.fail(body)(error);
  }
}

function recordingList(ctx, settings, list, minActive, visitor) {
  const form = recordingSettings(ctx, settings);
  if (!list.total) {
    return [h('section', { class: 'slab blank' },
      h('h2', { class: 'blank-title', text: settings.enabled ? 'Aucun enregistrement pour l\'instant' : 'Revoyez une visite comme si vous y étiez' }),
      h('p', { text: settings.enabled
        ? 'Les enregistrements sont activés. Une visite apparaît ici dès que le visiteur a donné son accord, ou qu\'un utilisateur connecté est identifié par votre application.'
        : 'Un enregistrement rejoue ce que le visiteur a vu et fait : souris, clics, défilement, navigation. Ce qu\'il saisit n\'est jamais capté : champs masqués, cases à cocher et champs cachés retirés.' }),
      h('p', { text: 'Seuls sont enregistrés les visiteurs qui y ont consenti : votre site appelle synalyse(\'consent\', true) après leur accord, ou synalyse(\'identify\', identifiant) pour un utilisateur connecté.' })), form];
  }
  const filter = h('select', { class: 'field', 'aria-label': 'Durée d\'activité', onchange: (e) => ctx.update({ actif: e.target.value === '0' ? null : e.target.value }) },
    [[0, 'Toutes les durées'], [10, 'Au moins 10 s d\'activité'], [60, 'Au moins 1 min d\'activité']].map(([value, label]) => h('option', { value, text: label, selected: value === minActive })));
  const rows = list.recordings.map((rec) => h('li', {}, h('a', { class: 'rec', href: ctx.href({ rec: rec.id }) },
    h('span', { class: 'rec-when', text: fullDate.format(new Date(rec.started_at)) }),
    h('span', { class: 'rec-page', text: rec.entry_path || '/', title: rec.entry_host + rec.entry_path }),
    h('span', { class: 'rec-fig' }, h('strong', { text: duration(rec.active_ms / 1000) }), ` actif sur ${duration(rec.duration_ms / 1000)}`),
    h('span', { class: 'rec-fig', text: `${nf.format(rec.pages)} ${rec.pages > 1 ? 'pages' : 'page'}, ${nf.format(rec.clicks)} ${rec.clicks > 1 ? 'clics' : 'clic'}` }),
    h('span', { class: 'rec-place', text: recordingPlace(rec) }))));
  return [
    h('div', { class: 'toolbar' }, filter,
      visitor ? h('a', { class: 'btn quiet', href: ctx.href({ visiteur: null }), text: 'Tous les visiteurs' }) : null,
      h('span', { class: 'hint inline', text: `${nf.format(list.total)} ${list.total > 1 ? 'enregistrements' : 'enregistrement'}, ${megabytes(list.stored_bytes)} sur ${nf.format(settings.max_mb)} Mo` })),
    h('section', { class: 'slab' }, rows.length
      ? h('ul', { class: 'recs' }, rows)
      : h('p', { class: 'empty', text: 'Aucun enregistrement ne correspond à ce filtre.' })),
    list.next_before ? h('p', { class: 'hint', text: 'Seuls les 50 enregistrements les plus récents sont affichés.' }) : null,
    form,
  ];
}

function recordingSettings(ctx, settings) {
  const error = h('p', { class: 'error', role: 'alert' });
  const enabled = h('input', { type: 'checkbox', name: 'enabled', checked: settings.enabled });
  const mask = h('input', { type: 'checkbox', name: 'mask_text', checked: settings.mask_text });
  const number = (name, value, min, max) => h('input', { class: 'field', type: 'number', name, value, min, max, step: 1, required: true, inputmode: 'numeric', autocomplete: 'off' });
  const sample = number('sample', settings.sample, 1, 100);
  const retention = number('retention_days', settings.retention_days, 1, 365);
  const cap = number('max_mb', settings.max_mb, 10, 100000);
  const save = h('button', { class: 'btn', type: 'submit', text: 'Enregistrer les réglages' });
  return h('details', { class: 'slab settings', open: !settings.enabled }, h('summary', { text: 'Réglages des enregistrements' }),
    h('form', { class: 'form', onsubmit: async (event) => {
      event.preventDefault();
      save.disabled = true;
      error.textContent = '';
      try {
        await api(ctx.root, { method: 'PATCH', body: { recording: {
          enabled: enabled.checked, mask_text: mask.checked,
          sample: Number(sample.value), retention_days: Number(retention.value), max_mb: Number(cap.value),
        } } });
        render();
      } catch (failure) {
        error.textContent = failure.status === 403 ? 'Seul un administrateur du site peut modifier ces réglages.' : failure.message;
        save.disabled = false;
      }
    } },
      h('label', { class: 'check' }, enabled, h('span', {}, 'Enregistrer les visites', h('small', { text: 'Uniquement celles des visiteurs qui y ont consenti ou des utilisateurs identifiés.' }))),
      h('label', { class: 'check' }, mask, h('span', {}, 'Masquer tout le texte des pages', h('small', { text: 'Les champs de saisie et les zones éditables sont toujours masqués, les cases à cocher retirées. Pour masquer une zone précise, ajoutez-lui l\'attribut data-synalyse-mask ; data-synalyse-block la retire entièrement.' }))),
      h('div', { class: 'settings-grid' },
        h('label', {}, 'Part des visites enregistrées (%)', sample),
        h('label', {}, 'Conservation (jours)', retention),
        h('label', {}, 'Plafond de stockage (Mo)', cap)),
      error, save));
}

function recordingPlayer(ctx, rec) {
  const error = h('p', { class: 'error', role: 'alert' });
  const remove = h('button', { class: 'btn quiet danger', type: 'button', text: 'Supprimer', onclick: async () => {
    if (!confirm('Supprimer définitivement cet enregistrement ?')) return;
    remove.disabled = true;
    try {
      await api(`${ctx.root}/recordings/${rec.id}`, { method: 'DELETE' });
      location.hash = ctx.href({ rec: null });
    } catch (failure) {
      error.textContent = failure.status === 403 ? 'Votre rôle ne permet pas de supprimer un enregistrement.' : failure.message;
      remove.disabled = false;
    }
  } });
  const frame = h('iframe', { src: `/player#${ctx.project.id}/${rec.id}`, title: 'Lecture de l\'enregistrement', allow: 'fullscreen', referrerpolicy: 'no-referrer' });
  // Le cadre prend les proportions de la fenêtre du visiteur, plus la place des commandes : pas de bandes vides.
  const fit = () => {
    if (!frame.isConnected) return window.removeEventListener('resize', fit);
    if (!rec.view_w || !rec.view_h) return;
    const width = frame.clientWidth;
    const controls = width < 620 ? 136 : 58;
    frame.style.height = `${Math.round(Math.max(260, Math.min(window.innerHeight * 0.78, width * rec.view_h / rec.view_w + controls)))}px`;
  };
  window.addEventListener('resize', fit);
  requestAnimationFrame(fit);
  return [
    h('div', { class: 'toolbar' },
      h('a', { class: 'btn quiet', href: ctx.href({ rec: null }), text: 'Tous les enregistrements' }),
      h('a', { class: 'btn quiet', href: ctx.href({ rec: null, visiteur: rec.visitor }), text: 'Autres visites de ce visiteur' }),
      h('span', { class: 'spacer' }), remove),
    error,
    h('section', { class: 'slab player' }, frame),
    h('dl', { class: 'facts' },
      h('div', {}, h('dt', { text: 'Début' }), h('dd', { text: fullDate.format(new Date(rec.started_at)) })),
      h('div', {}, h('dt', { text: 'Page d\'entrée' }), h('dd', { text: rec.entry_host + rec.entry_path })),
      h('div', {}, h('dt', { text: 'Activité' }), h('dd', { text: `${duration(rec.active_ms / 1000)} sur ${duration(rec.duration_ms / 1000)}` })),
      h('div', {}, h('dt', { text: 'Pages et clics' }), h('dd', { text: `${nf.format(rec.pages)} et ${nf.format(rec.clicks)}` })),
      h('div', {}, h('dt', { text: 'Appareil' }), h('dd', { text: recordingPlace(rec) || 'Inconnu' })),
      h('div', {}, h('dt', { text: 'Fenêtre' }), h('dd', { text: rec.view_w ? `${rec.view_w} × ${rec.view_h}` : 'Inconnue' }))),
  ];
}

window.addEventListener('hashchange', () => me && render());
// Un entonnoir en cours de saisie n'est pas perdu sur une fermeture d'onglet accidentelle.
window.addEventListener('beforeunload', (event) => { if (funnelDraft) event.preventDefault(); });
let resizeTimer = null;
window.addEventListener('resize', () => {
  clearTimeout(resizeTimer);
  resizeTimer = setTimeout(() => me && readRoute().view === 'p' && ['', 'parcours'].includes(readRoute().sub) && render(), 200);
});
boot();
