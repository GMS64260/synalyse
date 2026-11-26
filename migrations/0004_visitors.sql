-- Visiteurs reconnus d'une visite à l'autre : ceux qui ont donné leur consentement et les
-- utilisateurs connectés qu'une application identifie. Les visiteurs anonymes n'y figurent
-- jamais : leur empreinte change chaque jour.
CREATE TABLE visitors (
    project_id   BIGINT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    id           BIGINT NOT NULL,
    -- Identifiant fourni par le site (« u:42 ») ou tiré au hasard par le traceur après consentement.
    distinct_id  TEXT NOT NULL,
    first_seen   TIMESTAMPTZ NOT NULL,
    last_seen    TIMESTAMPTZ NOT NULL,
    -- Jour de la première visite dans le fuseau du projet : c'est lui qui range le visiteur dans une cohorte.
    first_day    DATE NOT NULL,
    sessions     INTEGER NOT NULL DEFAULT 0,
    -- Propriétés transmises par le site avec identify (offre, rôle…).
    traits       JSONB NOT NULL DEFAULT '{}',
    PRIMARY KEY (project_id, id)
);
CREATE INDEX visitors_recent_idx ON visitors (project_id, last_seen DESC);
CREATE INDEX visitors_first_day_idx ON visitors (project_id, first_day);
