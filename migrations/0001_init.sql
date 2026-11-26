-- Socle : organisations, comptes, projets, jetons, sessions de visite et événements.
-- Les tables volumineuses (sessions, events) sont partitionnées par mois ;
-- l'application crée les partitions (voir src/db.rs).

CREATE TABLE organizations (
    id          BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    name        TEXT NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE users (
    id             BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    email          TEXT NOT NULL,
    name           TEXT NOT NULL DEFAULT '',
    password_hash  TEXT NOT NULL,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    disabled_at    TIMESTAMPTZ
);
CREATE UNIQUE INDEX users_email_key ON users (lower(email));

-- Rôles, du plus au moins puissant : owner, admin, member, viewer.
CREATE TABLE memberships (
    org_id      BIGINT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    user_id     BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    role        TEXT NOT NULL CHECK (role IN ('owner', 'admin', 'member', 'viewer')),
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (org_id, user_id)
);
CREATE INDEX memberships_user_idx ON memberships (user_id);

CREATE TABLE projects (
    id           BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    org_id       BIGINT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    name         TEXT NOT NULL,
    -- Clé publique du traceur : identifie le projet, ne donne aucun droit de lecture.
    public_key   TEXT NOT NULL UNIQUE,
    timezone     TEXT NOT NULL DEFAULT 'UTC',
    -- Domaines autorisés à envoyer des événements ; vide = tous.
    domains      TEXT[] NOT NULL DEFAULT '{}',
    settings     JSONB NOT NULL DEFAULT '{}',
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    archived_at  TIMESTAMPTZ
);
CREATE INDEX projects_org_idx ON projects (org_id);

-- Sessions de connexion à l'interface. Seule l'empreinte du jeton est stockée.
CREATE TABLE user_sessions (
    token_hash    BYTEA PRIMARY KEY,
    user_id       BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at    TIMESTAMPTZ NOT NULL,
    last_seen_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX user_sessions_user_idx ON user_sessions (user_id);

-- Jetons d'API, liés à un projet. Portées : read (lecture), ingest (import), admin.
CREATE TABLE api_tokens (
    id            BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    project_id    BIGINT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    name          TEXT NOT NULL,
    scope         TEXT NOT NULL CHECK (scope IN ('read', 'ingest', 'admin')),
    token_hash    BYTEA NOT NULL UNIQUE,
    created_by    BIGINT REFERENCES users(id) ON DELETE SET NULL,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_used_at  TIMESTAMPTZ,
    revoked_at    TIMESTAMPTZ
);
CREATE INDEX api_tokens_project_idx ON api_tokens (project_id);

-- Sels quotidiens de l'empreinte des visiteurs sans consentement. Un sel est
-- supprimé le surlendemain : passé ce délai, une empreinte ne peut plus être recalculée.
CREATE TABLE visitor_salts (
    day   DATE PRIMARY KEY,
    salt  BYTEA NOT NULL
);

-- Canal d'acquisition : 0 direct, 1 recherche, 2 réseau social, 3 site référent, 4 campagne, 5 assistant IA.
-- Appareil : 0 inconnu, 1 ordinateur, 2 mobile, 3 tablette.
CREATE TABLE sessions (
    project_id     BIGINT NOT NULL,
    id             BIGINT NOT NULL,
    visitor_id     BIGINT NOT NULL,
    -- Vrai si le visiteur porte un identifiant stable (consentement ou utilisateur connecté).
    identified     BOOLEAN NOT NULL DEFAULT false,
    started_at     TIMESTAMPTZ NOT NULL,
    last_at        TIMESTAMPTZ NOT NULL,
    -- Jour de début dans le fuseau du projet : toutes les statistiques par jour s'appuient dessus.
    day            DATE NOT NULL,
    entry_host     TEXT NOT NULL DEFAULT '',
    entry_path     TEXT NOT NULL DEFAULT '',
    exit_path      TEXT NOT NULL DEFAULT '',
    pageviews      INTEGER NOT NULL DEFAULT 0,
    events         INTEGER NOT NULL DEFAULT 0,
    channel        SMALLINT NOT NULL DEFAULT 0,
    referrer_host  TEXT,
    utm_source     TEXT,
    utm_medium     TEXT,
    utm_campaign   TEXT,
    device         SMALLINT NOT NULL DEFAULT 0,
    browser        TEXT NOT NULL DEFAULT '',
    os             TEXT NOT NULL DEFAULT '',
    lang           TEXT,
    screen_w       INTEGER,
    country        TEXT,
    PRIMARY KEY (project_id, started_at, id)
) PARTITION BY RANGE (started_at);
CREATE INDEX sessions_visitor_idx ON sessions (project_id, visitor_id, last_at DESC);
CREATE INDEX sessions_day_idx ON sessions (project_id, day);

-- Type d'événement : 0 page vue, 1 événement personnalisé.
CREATE SEQUENCE events_id_seq;
CREATE TABLE events (
    project_id  BIGINT NOT NULL,
    ts          TIMESTAMPTZ NOT NULL,
    id          BIGINT NOT NULL DEFAULT nextval('events_id_seq'),
    session_id  BIGINT NOT NULL,
    visitor_id  BIGINT NOT NULL,
    day         DATE NOT NULL,
    kind        SMALLINT NOT NULL,
    name        TEXT,
    host        TEXT NOT NULL DEFAULT '',
    path        TEXT NOT NULL DEFAULT '',
    title       TEXT,
    props       JSONB,
    PRIMARY KEY (project_id, ts, id)
) PARTITION BY RANGE (ts);
CREATE INDEX events_session_idx ON events (project_id, session_id, ts);
CREATE INDEX events_day_idx ON events (project_id, day);
