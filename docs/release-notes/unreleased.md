# Unreleased

## `ntnt db` on PostgreSQL

- **Enums, domains and composite types.** A history that uses
  `CREATE TYPE ... AS ENUM`, `CREATE DOMAIN` or a composite type plans,
  migrates and verifies. Each type is compared by its definition (an enum's
  labels in order; a domain's base type, `NOT NULL`, default, collation and
  constraints; a composite type's attributes), in the managed schema and in
  schemas the history creates, and columns of these types are covered. An
  out-of-band `ALTER TYPE ... ADD VALUE` or a changed domain constraint is
  drift on the type. Any of these used to leave every plan `unknown` with
  `DB_COVERAGE_INCOMPLETE` (#430).
- **Trusted extensions in the managed schema.** An extension such as pg_trgm,
  citext, hstore or btree_gist installed in the managed schema is compared by
  its name and version, which cover what it installs: types, functions,
  aggregates, operators, operator classes and families, and casts. A
  `gin_trgm_ops` index or a `citext` column plans and migrates, and citext's
  implicit casts no longer stop every later migration. An out-of-band
  `ALTER EXTENSION ... UPDATE` is drift on the extension. Any other custom
  implicit or assignment cast still stops the next migration, now with
  `DB_CHECK_UNSUPPORTED` naming the cast and the migration (#430).
- **Operators and operator classes.** An operator's catalog key carries its
  argument types (`other:operator.===(integer,integer)`) and an operator
  class's its access method (`other:operator_class.citext_ops USING btree`), so
  overloads, and citext's btree and hash `citext_ops`, no longer collide. A
  user-defined operator whose function is covered is covered. When two objects
  still share a key, `DB_CATALOG_AMBIGUOUS` names it and the migration that
  made them (#430).
- **`EXCLUDE` constraints** plan, migrate and verify, with built-in operator
  classes or an extension's such as btree_gist (#430).
- **Checks on these tables.** `requires`, `ensures` and adoption checks may
  read tables with columns of these types, and run while an extension's casts
  exist. Adopting a history that uses an enum or citext column used to fail at
  `migrate` with `DB_CHECK_UNSUPPORTED`. Checks still run no routine the
  history wrote. Comparing a column with a literal converts the literal to the
  column's type; for a domain that is its base type, so a domain's own
  constraints never run, whatever they call. The elements of an array and the
  attributes of a composite go through their types' input, though, so a check
  that can read `emails app.email[]`, where `app.email`'s constraint calls a
  function the history wrote or a volatile one such as `nextval`, fails with
  `DB_CHECK_UNSUPPORTED` before it runs, naming the table, column, type,
  domain and function. So does a view that converts values to such a domain,
  or that uses an operator whose function, or whose negator's, the history
  wrote, and a view that calls `pg_input_is_valid` or `json_populate_record`
  while such a domain exists. A domain whose
  constraint uses only builtins, such as a regular expression, is allowed at
  any depth (#430).

Catalogs that use none of these keep their fingerprints, so saved plans and
applied history stay valid.

## Security

The lockfile updates `cookie` from 0.18.2 to 0.18.3 for
[RUSTSEC-2026-0594](https://rustsec.org/advisories/RUSTSEC-2026-0594.html)
(#456).

See [NTNT v0.6.4](v0.6.4.md) for the latest release announcement and upgrade
guidance.
