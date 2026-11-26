-- Enregistrements de session. Les événements rejoués vivent hors de la base, dans un
-- fichier compressé par enregistrement (voir src/recording.rs) ; cette table n'en garde
-- que le résumé. L'identifiant est tiré au hasard par le navigateur.
CREATE TABLE recordings (
    project_id  BIGINT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    id          TEXT NOT NULL,
    visitor_id  BIGINT NOT NULL,
    started_at  TIMESTAMPTZ NOT NULL,
    last_at     TIMESTAMPTZ NOT NULL,
    -- Temps pendant lequel le visiteur a réellement agi (souris, clavier, défilement).
    active_ms   BIGINT NOT NULL DEFAULT 0,
    events      INTEGER NOT NULL DEFAULT 0,
    chunks      INTEGER NOT NULL DEFAULT 0,
    -- Taille sur le disque, puis taille une fois décompressé.
    bytes       BIGINT NOT NULL DEFAULT 0,
    raw_bytes   BIGINT NOT NULL DEFAULT 0,
    clicks      INTEGER NOT NULL DEFAULT 0,
    pages       INTEGER NOT NULL DEFAULT 0,
    entry_host  TEXT NOT NULL DEFAULT '',
    entry_path  TEXT NOT NULL DEFAULT '',
    device      SMALLINT NOT NULL DEFAULT 0,
    browser     TEXT NOT NULL DEFAULT '',
    os          TEXT NOT NULL DEFAULT '',
    view_w      INTEGER,
    view_h      INTEGER,
    PRIMARY KEY (project_id, id)
);
CREATE INDEX recordings_recent_idx ON recordings (project_id, started_at DESC);
