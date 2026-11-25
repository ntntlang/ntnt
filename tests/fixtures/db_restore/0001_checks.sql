-- #460: every kind of expression PostgreSQL stores with an AND (OR) nested
-- as the first operand of another, which pg_restore stores merged, and
-- controls a restore leaves as they are.
CREATE TABLE app.checks (
    id int PRIMARY KEY,
    check_type text NOT NULL,
    timeout_ms int NOT NULL,
    options jsonb NOT NULL DEFAULT '{}',
    n int,
    CONSTRAINT checks_executable_timeout CHECK (timeout_ms BETWEEN 1000 AND 15000 AND timeout_ms % 1000 = 0),
    CONSTRAINT checks_between_only CHECK (timeout_ms BETWEEN 1 AND 100000),
    CONSTRAINT checks_plain_and CHECK (timeout_ms > 0 AND timeout_ms < 100000 AND n IS DISTINCT FROM 0),
    CONSTRAINT checks_nested_last CHECK (n > 0 AND n BETWEEN 1 AND 9),
    CONSTRAINT checks_not_between_or CHECK (n NOT BETWEEN 5 AND 9 OR n IS NULL),
    CONSTRAINT checks_ping_execution_budget CHECK (
        check_type <> 'ping' OR (
            jsonb_typeof(options) = 'object'
            AND CASE
            WHEN (options->>'count' IS NULL OR (length(options->>'count') <= 100 AND options->>'count' ~ '^[0-9]+$'))
             AND (options->>'interval_ms' IS NULL OR (length(options->>'interval_ms') <= 100 AND options->>'interval_ms' ~ '^[0-9]+$'))
            THEN
                COALESCE(options->>'count', '1')::numeric BETWEEN 1 AND 10
                AND COALESCE(options->>'interval_ms', '0')::numeric BETWEEN 0 AND 5000
                AND timeout_ms > COALESCE(options->>'interval_ms', '0')::numeric * (COALESCE(options->>'count', '1')::numeric - 1)
            ELSE false
            END
        )
    ),
    CONSTRAINT checks_one_fast EXCLUDE USING btree (id WITH =) WHERE (timeout_ms BETWEEN 1 AND 5 AND n > 0)
);
CREATE TABLE app.proposals (id int PRIMARY KEY, timeout_ms int NOT NULL CHECK (timeout_ms BETWEEN 1000 AND 15000 AND timeout_ms % 1000 = 0));
CREATE INDEX checks_fast_idx ON app.checks (id) WHERE timeout_ms BETWEEN 1000 AND 5000 AND check_type = 'ping';
CREATE INDEX checks_fast_expr_idx ON app.checks ((timeout_ms BETWEEN 1000 AND 5000 AND n > 0));
CREATE VIEW app.fast_checks AS SELECT id FROM app.checks WHERE timeout_ms BETWEEN 1000 AND 5000 AND check_type = 'ping';
ALTER TABLE app.checks ENABLE ROW LEVEL SECURITY;
CREATE POLICY checks_window ON app.checks USING (timeout_ms BETWEEN 1000 AND 15000 AND check_type <> 'x')
    WITH CHECK (n NOT BETWEEN 1 AND 3 OR n IS NULL);
ALTER TABLE app.checks ADD COLUMN in_window boolean DEFAULT (now()::date BETWEEN '2026-01-01' AND '2026-12-31' AND true);
ALTER TABLE app.checks ADD COLUMN fast boolean GENERATED ALWAYS AS (timeout_ms BETWEEN 1000 AND 5000 AND n > 0) STORED;
CREATE DOMAIN app.even_small AS int CHECK (VALUE BETWEEN 1 AND 100 AND VALUE % 2 = 0);
CREATE DOMAIN app.window_flag AS boolean DEFAULT (1 BETWEEN 0 AND 2 AND true);
CREATE FUNCTION app.in_range(x int) RETURNS boolean LANGUAGE sql IMMUTABLE RETURN x BETWEEN 1 AND 5 AND x > 0;
CREATE FUNCTION app.touch() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RETURN NEW; END $$;
CREATE TRIGGER checks_touch BEFORE UPDATE ON app.checks FOR EACH ROW
    WHEN (NEW.timeout_ms BETWEEN 1000 AND 5000 AND NEW.n > 0) EXECUTE FUNCTION app.touch();
CREATE TABLE app.flags (id int NOT NULL, k int NOT NULL) PARTITION BY LIST ((k BETWEEN 1 AND 5 AND id > 0));
