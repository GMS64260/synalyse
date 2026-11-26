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

function cairn() {
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
  renderDashboard(project, route.params);
}

const brand = () => h('a', { class: 'brand', href: '#' }, cairn(), h('span', { text: 'Synalyse' }));

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
  h('label', {}, 'Adresse e-mail', h('input', { class: 'field', type: 'email', name: 'email', autocomplete: 'username', required: true, autofocus: true })),
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
  h('label', {}, 'Nom du site', h('input', { class: 'field', name: 'name', required: true, maxlength: 100, placeholder: 'Boutique en ligne', autofocus: true })),
  h('label', {}, 'Domaine', h('input', { class: 'field', name: 'domain', placeholder: 'exemple.fr', autocomplete: 'off', spellcheck: 'false' }),
    h('small', { text: 'Seules les pages de ce domaine et de ses sous-domaines seront comptées. Laissez vide pour tout accepter.' })),
  h('label', {}, 'Fuseau horaire', h('input', { class: 'field', name: 'timezone', value: zone, required: true, spellcheck: 'false' }),
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
    h('pre', { class: 'snippet', tabindex: '0' }, h('code', { text: code })),
    h('div', { class: 'row-actions' }, copy, h('a', { class: 'btn', href: `#p/${project.id}`, text: 'Voir les statistiques' })),
    h('ol', { class: 'steps' },
      h('li', {}, h('strong', { text: 'Aucun bandeau de cookies à ajouter. ' }), "Par défaut, Synalyse ne dépose rien dans le navigateur et ne reconnaît pas un visiteur d'un jour à l'autre."),
      h('li', {}, h('strong', { text: 'Mesurer une action. ' }), "Appelez synalyse('event', 'Inscription') au moment où elle se produit."),
      h('li', {}, h('strong', { text: "Permettre l'opposition. " }), "Reliez un bouton de votre politique de confidentialité à synalyse('optout')."))));
}

// ——— Tableau de bord ———

function renderDashboard(project, params) {
  const token = ++renderToken;
  const period = resolvePeriod(params, project.timezone);
  const metric = METRICS.some((m) => m.chart && m.key === params.get('metric')) ? params.get('metric') : 'visitors';
  const filters = Object.keys(FILTER_NAMES).filter((key) => params.has(key)).map((key) => [key, params.get(key)]);
  const base = `/api/projects/${project.id}/stats`;
  const filterQuery = filters.map(([k, v]) => `&${k}=${encodeURIComponent(v)}`).join('');
  const range = (from, to) => `from=${from}&to=${to}${filterQuery}`;

  /** Navigue vers le même écran avec des paramètres modifiés (null supprime). */
  const update = (changes) => {
    const next = new URLSearchParams(params);
    for (const [key, value] of Object.entries(changes)) {
      if (value === null) next.delete(key); else next.set(key, value);
    }
    go(`p/${project.id}`, next);
  };

  // Barre du haut
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
    ? h('select', { class: 'field site', 'aria-label': 'Site', onchange: (e) => go(`p/${e.target.value}`, params) },
      me.projects.map((p) => h('option', { value: p.id, text: p.name, selected: p.id === project.id })))
    : h('strong', { text: project.name });
  const topbar = h('header', { class: 'topbar' }, brand(), projectPicker, live, h('span', { class: 'spacer' }), ...custom, periodSelect,
    h('details', { class: 'menu' }, h('summary', { class: 'field', text: 'Réglages' }), h('div', { class: 'menu-items' },
      h('a', { href: `#p/${project.id}/installation`, text: 'Installer le code de suivi' }),
      h('a', { href: '#new', text: 'Ajouter un site' }),
      h('button', { type: 'button', text: 'Se déconnecter', onclick: async () => {
        await api('/api/auth/logout', { method: 'POST' }).catch(() => {});
        me = null;
        renderLogin();
      } }))));

  // Filtres actifs
  const chips = h('div', { class: 'filters' }, filters.map(([key, value]) => h('span', { class: 'chip' },
    h('span', { text: FILTER_NAMES[key] }), displayLabel(key, value),
    h('button', { type: 'button', 'aria-label': `Retirer le filtre ${FILTER_NAMES[key]}`, text: '×', onclick: () => update({ [key]: null }) }))));

  // Dalle : chiffres clés et courbe
  const metricEls = METRICS.map((m) => {
    const value = h('span', { class: 'value', text: '–' });
    const delta = h('span', { class: 'delta' });
    const el = m.chart
      ? h('button', { class: 'metric', type: 'button', 'aria-pressed': String(m.key === metric), onclick: () => update({ metric: m.key }) }, h('span', { class: 'label', text: m.label }), value, delta)
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
    const tabs = panel.tabs.length > 1 ? h('div', { class: 'tabs', role: 'tablist' }, panel.tabs.map(([dim, label]) =>
      h('button', { type: 'button', role: 'tab', 'aria-selected': String(dim === active[0]), text: label, onclick: () => update({ [stateKey]: dim }) }))) : null;
    return { dim: active[0], body, el: h('section', { class: 'slab panel' }, h('header', {}, h('h2', { text: panel.title }), tabs), body) };
  });

  mount(topbar, h('main', {}, chips, slab, h('div', { class: 'panels' }, panels.map((p) => p.el))));

  const stale = () => token !== renderToken;
  const fail = (target) => (error) => {
    if (stale()) return;
    if (error.status === 401) { me = null; return renderLogin(); }
    target.replaceChildren(h('p', { class: 'empty', text: error.message }));
  };

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
    api(`${base}/breakdown?dim=${panel.dim}&limit=9&${range(period.from, period.to)}`)
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
              ? h('button', { class: 'row', type: 'button', title: 'Filtrer sur cette valeur', onclick: () => update({ [panel.dim]: row.label }) }, content)
              : h('div', { class: 'row' }, content);
            el.style.setProperty('--part', `${(row.visitors / max) * 100}%`);
            return h('li', {}, el);
          })));
      })
      .catch(fail(panel.body));
  }

  // Temps réel, rafraîchi toutes les trente secondes tant que l'écran est affiché.
  const refreshLive = () => api(`${base}/realtime`).then((data) => {
    if (stale()) return;
    const n = data.last_5_minutes;
    live.classList.toggle('on', n > 0);
    live.lastChild.replaceChildren(h('b', { text: nf.format(n) }), n > 1 ? ' visiteurs en ce moment' : ' visiteur en ce moment');
  }).catch(() => {});
  refreshLive();
  liveTimer = setInterval(() => document.visibilityState === 'visible' && refreshLive(), 30000);
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

window.addEventListener('hashchange', () => me && render());
let resizeTimer = null;
window.addEventListener('resize', () => {
  clearTimeout(resizeTimer);
  resizeTimer = setTimeout(() => me && readRoute().view === 'p' && !readRoute().sub && render(), 200);
});
boot();
