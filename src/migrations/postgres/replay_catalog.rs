//! Intermediate scratch states need coverage and (for plan) a stable digest,
//! not a materialized Catalog. Dictionary-encode equal definitions within
//! this inspection. Nothing is inferred from migration text or reused from a
//! prior snapshot; a change to a dependency, grant or role is observed by
//! exactly the same catalog queries as a full inspection.
use super::{
    ambiguous_key, catalog_value, check_catalog_limit, scope, Postgres, RelationalAdapter,
};
use crate::migrations::{
    catalog_sql,
    inventory::{catalog_digest, CatalogDigests},
    Result,
};
use std::collections::BTreeMap;

impl Postgres {
    pub(in crate::migrations) async fn replay_catalog_state(
        &self,
        schema: &str,
        extra: &[String],
        outside: &[String],
        role: &str,
        mut digests: Option<&mut CatalogDigests>,
    ) -> Result<(Vec<String>, Option<String>)> {
        // The full reader also inspects the dependency tree of each declared
        // extension; keep its ownership checks and limits unchanged.
        if !self.extensions.is_empty() {
            let catalog = self.catalog(schema, extra, outside, role).await?;
            let fingerprint = digests.map(|d| d.digest(&catalog));
            return Ok((catalog.uncovered, fingerprint));
        }
        self.batch("SET LOCAL search_path=pg_catalog").await?;
        let scope = scope(schema, extra);
        let declared: &[String] = &self.extensions;
        let mut uncovered = Vec::new();
        let mut objects = BTreeMap::new();
        for (kind, sql) in catalog_sql::QUERIES {
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
            let mut values = Vec::new();
            for row in self.rows(&grouped, params).await? {
                let names: Vec<String> = row.get(0);
                check_catalog_limit(entries.len() + names.len())?;
                let pos: Vec<i64> = row.get(4);
                let supported: bool = row.get(2);
                let prefix = match row.get::<_, Option<String>>(3) {
                    Some(qualifier) => format!("{qualifier}.{kind}:"),
                    None => format!("{kind}:"),
                };
                let group = values.len();
                if let Some(d) = digests.as_deref_mut() {
                    let (value, stored) = catalog_value(kind, row.get(1), role);
                    d.stabilized |= stored.is_some();
                    values.push(serde_json::to_vec(&value).expect("catalog values serialize"));
                }
                entries.extend(
                    pos.into_iter()
                        .zip(names)
                        .map(|(pos, name)| (pos, format!("{prefix}{name}"), supported, group)),
                );
            }
            entries.sort_unstable_by_key(|e| e.0);
            for (_, key, supported, group) in entries {
                let bytes = values.get(group).map_or_else(Vec::new, |v| {
                    let mut bytes = serde_json::to_vec(&key).expect("catalog keys serialize");
                    bytes.push(b':');
                    bytes.extend_from_slice(v);
                    bytes
                });
                if !supported {
                    uncovered.push(key.clone());
                }
                match objects.entry(key) {
                    std::collections::btree_map::Entry::Vacant(e) => {
                        e.insert(bytes);
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
        let fp = digests.map(|_| catalog_digest(objects.values().map(Vec::as_slice), &uncovered));
        Ok((uncovered, fp))
    }
}
