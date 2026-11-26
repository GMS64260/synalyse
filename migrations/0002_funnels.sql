-- Entonnoirs enregistrés d'un projet. Les étapes sont un tableau JSON, voir src/analysis/funnel.rs.
CREATE TABLE funnels (
    id          BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    project_id  BIGINT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    name        TEXT NOT NULL,
    steps       JSONB NOT NULL,
    created_by  BIGINT REFERENCES users(id) ON DELETE SET NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    deleted_at  TIMESTAMPTZ
);
CREATE INDEX funnels_project_idx ON funnels (project_id) WHERE deleted_at IS NULL;
