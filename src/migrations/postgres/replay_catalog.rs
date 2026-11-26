//! Group equal definitions within each fresh catalog read. Scratch needs only
//! the stable fingerprint; live checks also compare stored values from older
//! plans. A digest can be reused only if the entire new observation matches.
use super::{
    ambiguous_key, catalog_value, check_catalog_limit, scope, Postgres, RelationalAdapter,
};
use crate::migrations::{
    catalog_sql,
    inventory::{CatalogDigests, Fingerprint, GroupedSnapshot},
    Result,
};
use std::collections::BTreeMap;

/// Observe typically empty classes in one exchange. Use the authoritative
/// SELECTs themselves, so new object sources or cross-schema dependencies
/// cannot escape the census. EXISTS doesn't need the deparsed definition.
/// `schema`, `relation` and `column` go straight to their readers. `server`
/// and `extension` have different parameter types and are database-wide.
static CLASS_PRESENCE: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
    let fields: Vec<_> = catalog_sql::QUERIES
        .iter()
        .filter(|(kind, _)| {
            !matches!(
                *kind,
                "schema" | "relation" | "column" | "extension" | "server"
            )
        })
        .map(|(kind, sql)| format!("EXISTS({sql}) AS \"{kind}\""))
        .collect();
    format!("SELECT {}", fields.join(","))
});

impl Postgres {
    // Bind the native parameter types of QUERIES in one protocol exchange;
    // don't retain server-side statements across authored SQL.
    pub(super) async fn catalog_rows(
        &self,
        kind: &str,
        sql: &str,
        schema: &str,
        scope: &[String],
        outside: &[String],
    ) -> Result<Vec<tokio_postgres::Row>> {
        use tokio_postgres::types::{ToSql, Type};
        let declared: &[String] = &self.extensions;
        let args: [(&(dyn ToSql + Sync), Type); 4] = [
            (
                &schema,
                if matches!(kind, "extension" | "server") {
                    Type::TEXT
                } else {
                    Type::NAME
                },
            ),
            (
                &scope,
                if kind == "server" {
                    Type::TEXT_ARRAY
                } else {
                    Type::NAME_ARRAY
                },
            ),
            (&outside, Type::TEXT_ARRAY),
            (
                &declared,
                if kind == "extension" {
                    Type::NAME_ARRAY
                } else {
                    Type::TEXT_ARRAY
                },
            ),
        ];
        let n = if catalog_sql::WITH_DECLARED_EXTENSIONS.contains(&kind) {
            4
        } else {
            3
        };
        let rows = super::bounded(self.client.query_typed(sql, &args[..n])).await?;
        check_catalog_limit(rows.len())?;
        Ok(rows)
    }
    pub(super) async fn class_presence(&self, schema: &str) -> Result<tokio_postgres::Row> {
        let scope = [schema.to_owned()];
        Ok(self
            .catalog_rows("other", &CLASS_PRESENCE, schema, &scope, &[])
            .await?
            .remove(0))
    }
}

/// A new class not in the census must go through its full reader.
pub(super) fn absent(presence: &Option<tokio_postgres::Row>, kind: &str) -> bool {
    presence
        .as_ref()
        .and_then(|r| {
            r.columns()
                .iter()
                .position(|c| c.name() == kind)
                .map(|i| !r.get::<_, bool>(i))
        })
        .unwrap_or(false)
}

impl Postgres {
    pub(in crate::migrations) async fn replay_catalog_state(
        &self,
        schema: &str,
        extra: &[String],
        outside: &[String],
        role: &str,
        digests: Option<&mut CatalogDigests>,
    ) -> Result<(Vec<String>, Option<String>)> {
        let (uncovered, fp) = self
            .grouped_catalog(schema, extra, outside, role, digests, false)
            .await?;
        Ok((uncovered, fp.map(|fp| fp.stable)))
    }

    pub(in crate::migrations) async fn live_fingerprint(
        &self,
        schema: &str,
        extra: &[String],
        outside: &[String],
        role: &str,
        digests: &mut CatalogDigests,
    ) -> Result<(Vec<String>, Fingerprint)> {
        let (uncovered, fp) = self
            .grouped_catalog(schema, extra, outside, role, Some(digests), true)
            .await?;
        Ok((uncovered, fp.expect("fingerprint requested")))
    }

    async fn grouped_catalog(
        &self,
        schema: &str,
        extra: &[String],
        outside: &[String],
        role: &str,
        mut digests: Option<&mut CatalogDigests>,
        with_stored: bool,
    ) -> Result<(Vec<String>, Option<Fingerprint>)> {
        // The full reader also inspects the dependency tree of each declared
        // extension; keep its ownership checks and limits unchanged.
        if !self.extensions.is_empty() {
            let catalog = self.catalog(schema, extra, outside, role).await?;
            let fingerprint = digests.map(|d| {
                if with_stored {
                    d.fingerprint(&catalog)
                } else {
                    Fingerprint {
                        stable: d.digest(&catalog),
                        stored: None,
                    }
                }
            });
            return Ok((catalog.uncovered, fingerprint));
        }
        self.batch("SET LOCAL search_path=pg_catalog").await?;
        let scope = scope(schema, extra);
        let declared: &[String] = &self.extensions;
        let mut uncovered = Vec::new();
        let mut objects = BTreeMap::new();
        let presence = if with_stored && extra.is_empty() && outside.is_empty() {
            Some(self.class_presence(schema).await?)
        } else {
            None
        };
        let mut values = Vec::new();
        for (kind, sql) in catalog_sql::QUERIES {
            if absent(&presence, kind) {
                continue;
            }
            let params: &[&(dyn tokio_postgres::types::ToSql + Sync)] =
                if catalog_sql::WITH_DECLARED_EXTENSIONS.contains(kind) {
                    &[&schema, &scope, &outside, &declared]
                } else {
                    &[&schema, &scope, &outside]
                };
            // Keep the native row order to report the same first ambiguous
            // key, even when two rows of a group were not adjacent. Bound
            // the input, not just the number of distinct output groups.
            let grouped = format!(
                r#"WITH entries AS (
 SELECT name, v::text COLLATE "C" AS def, covered, qualifier COLLATE "C" AS qual,
        row_number() OVER () AS pos FROM ({sql}) q(name,v,covered,qualifier) LIMIT 10001)
 SELECT array_agg(name ORDER BY pos), def::jsonb, covered, qual, array_agg(pos ORDER BY pos)
 FROM entries GROUP BY def, covered, qual"#
            );
            let mut entries = Vec::new();
            let shape_query = if presence.is_some() {
                match *kind {
                    "relation" => Some(catalog_sql::RELATION_SHAPES),
                    "column" => Some(catalog_sql::COLUMN_SHAPES),
                    _ => None,
                }
            } else {
                None
            };
            let shapes = if let Some(sql) = shape_query {
                let rows = self
                    .catalog_rows(kind, sql, schema, &scope, outside)
                    .await?;
                (rows.len() < 65).then_some(rows)
            } else {
                None
            };
            let is_shape = shapes.is_some();
            let rows = match shapes {
                Some(rows) => {
                    #[cfg(test)]
                    super::LIVE_SHAPE_ROWS.with(|n| n.set(n.get() + rows.len()));
                    rows
                }
                None if with_stored => {
                    self.catalog_rows(kind, &grouped, schema, &scope, outside)
                        .await?
                }
                None => self.rows(&grouped, params).await?,
            };
            for row in rows {
                let names: Vec<String> = row.get(0);
                check_catalog_limit(entries.len() + names.len())?;
                // Single-schema relation and quoted relation/attribute names are
                // unique. No native row order is needed to disambiguate them.
                let pos: Vec<i64> = if is_shape {
                    vec![0; names.len()]
                } else {
                    row.get(4)
                };
                let supported: bool = row.get(2);
                let prefix = match row.get::<_, Option<String>>(3) {
                    Some(qualifier) => format!("{qualifier}.{kind}:"),
                    None => format!("{kind}:"),
                };
                let group = values.len();
                if let Some(d) = digests.as_deref_mut() {
                    let (value, stored) = catalog_value(kind, row.get(1), role);
                    d.stabilized |= stored.is_some();
                    values.push((
                        serde_json::to_vec(&value).expect("catalog values serialize"),
                        stored
                            .filter(|_| with_stored)
                            .map(|v| serde_json::to_vec(&v).expect("catalog values serialize")),
                    ));
                }
                entries.extend(
                    pos.into_iter()
                        .zip(names)
                        .map(|(pos, name)| (pos, format!("{prefix}{name}"), supported, group)),
                );
            }
            entries.sort_unstable_by_key(|e| e.0);
            for (_, key, supported, group) in entries {
                let bytes = if digests.is_some() {
                    let mut bytes = serde_json::to_vec(&key).expect("catalog keys serialize");
                    bytes.push(b':');
                    bytes
                } else {
                    Vec::new()
                };
                if !supported {
                    uncovered.push(key.clone());
                }
                match objects.entry(key) {
                    std::collections::btree_map::Entry::Vacant(e) => {
                        e.insert((bytes, group));
                    }
                    std::collections::btree_map::Entry::Occupied(e) => {
                        return Err(ambiguous_key(e.key().clone()))
                    }
                }
            }
        }
        if !objects.contains_key(&format!("schema:{schema}")) {
            uncovered.push(format!("schema_missing:{schema}"));
        }
        uncovered.sort();
        uncovered.dedup();
        let fp = digests.map(|d| {
            let observed = GroupedSnapshot {
                objects,
                values,
                uncovered: uncovered.clone(),
            };
            if with_stored {
                if let Some((previous, fp)) = d.live.as_deref() {
                    if *previous == observed {
                        return fp.clone();
                    }
                }
            }
            let fp = observed.fingerprint();
            if with_stored {
                d.live = Some(Box::new((observed, fp.clone())));
            }
            fp
        });
        Ok((uncovered, fp))
    }
}
