-- #460: partitions' PRIMARY KEY and UNIQUE constraints, cloned from the
-- parent's, which pg_restore adds to each partition and attaches.
CREATE TABLE app.observations (
    id bigint NOT NULL,
    tenant_id int NOT NULL,
    probe_run_id bigint NOT NULL,
    observed_on int NOT NULL,
    value double precision,
    PRIMARY KEY (id, observed_on),
    UNIQUE (tenant_id, probe_run_id, observed_on)
) PARTITION BY RANGE (observed_on);
CREATE TABLE app.observations_default PARTITION OF app.observations DEFAULT;
CREATE TABLE app.observations_2026 PARTITION OF app.observations FOR VALUES FROM (20260101) TO (20270101);
