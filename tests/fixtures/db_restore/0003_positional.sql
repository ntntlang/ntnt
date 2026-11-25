-- #464 review: SQL-standard bodies that name an unnamed argument by its
-- position ($1), whose AND chains pg_restore stores merged like any other.
CREATE FUNCTION app.in_range_positional(int) RETURNS boolean LANGUAGE sql IMMUTABLE RETURN $1 BETWEEN 1 AND 5 AND $1 > 0;
CREATE FUNCTION app.in_range_atomic(int, text) RETURNS boolean LANGUAGE sql IMMUTABLE
BEGIN ATOMIC
    SELECT $1 BETWEEN 1 AND 5 AND $1 > 0 AND $2 <> 'x';
END;
