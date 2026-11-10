//! Catalog validation for the fixed std/auth PostgreSQL schema, not a migration API.
//! Keep these required shapes in sync with config.rs bootstrap SQL. Object names
//! alone are insufficient: wrong columns or missing uniqueness can break auth safety.
use super::postgres_schema_error;

type Column = (&'static str, &'static str, bool, Option<&'static str>);
const COLUMNS: &[(&str, &[Column])] = &[
    (
        "auth_sessions",
        &[
            ("id", "text", true, None),
            ("user_id", "text", true, None),
            ("provider", "text", true, None),
            ("email", "text", false, None),
            ("name", "text", false, None),
            ("picture", "text", false, None),
            ("raw_json", "text", true, None),
            ("data_json", "text", true, None),
            ("csrf_token", "text", true, Some("''::text")),
            ("access_token", "text", false, None),
            ("refresh_token", "text", false, None),
            ("token_expires_at", "bigint", false, None),
            ("device_name", "text", false, None),
            ("user_agent_hash", "text", false, None),
            ("last_ip_hash", "text", false, None),
            ("created_at", "bigint", true, None),
            ("expires_at", "bigint", true, None),
        ],
    ),
    (
        "auth_oauth_states",
        &[
            ("state", "text", true, None),
            ("nonce", "text", false, None),
            ("pkce_verifier", "text", false, None),
            ("provider", "text", true, None),
            ("redirect_url", "text", true, None),
            ("remember_me", "boolean", true, Some("false")),
            ("device_name", "text", false, None),
            ("user_agent_hash", "text", false, None),
            ("last_ip_hash", "text", false, None),
            ("created_at", "bigint", true, None),
        ],
    ),
    (
        "auth_exchange_tokens",
        &[
            ("token", "text", true, None),
            ("session_id", "text", true, None),
            ("created_at", "bigint", true, None),
        ],
    ),
    (
        "auth_challenges",
        &[
            ("id", "text", true, None),
            ("subject_id", "text", true, None),
            ("provider", "text", true, None),
            ("kind", "text", true, None),
            ("csrf_token", "text", true, Some("''::text")),
            ("data_json", "text", true, None),
            ("created_at", "bigint", true, None),
            ("expires_at", "bigint", true, None),
        ],
    ),
    (
        "auth_rate_limits",
        &[
            ("scope", "text", true, None),
            ("key_hash", "text", true, None),
            ("count", "bigint", true, None),
            ("expires_at", "bigint", true, None),
        ],
    ),
    (
        "auth_local_identities",
        &[
            ("id", "text", true, None),
            ("identifier_kind", "text", true, None),
            ("identifier", "text", true, None),
            ("identifier_normalized", "text", true, None),
            ("created_at", "bigint", true, None),
            ("updated_at", "bigint", true, None),
            ("state", "text", true, None),
            ("metadata_json", "text", true, None),
        ],
    ),
    (
        "auth_local_credentials",
        &[
            ("local_user_id", "text", true, None),
            ("password_hash", "text", true, None),
            ("password_hash_algorithm", "text", true, None),
            ("password_hash_params_json", "text", true, None),
            ("password_changed_at", "bigint", true, None),
            ("must_change_password", "boolean", true, Some("false")),
        ],
    ),
    (
        "auth_local_one_time_tokens",
        &[
            ("purpose", "text", true, None),
            ("selector", "text", true, None),
            ("local_user_id", "text", true, None),
            ("token_hash", "text", true, None),
            ("created_at", "bigint", true, None),
            ("expires_at", "bigint", true, None),
        ],
    ),
];

const CONSTRAINTS: &[(&str, &[&str])] = &[
    ("auth_sessions", &["PRIMARY KEY (id)"]),
    ("auth_oauth_states", &["PRIMARY KEY (state)"]),
    ("auth_exchange_tokens", &["PRIMARY KEY (token)"]),
    ("auth_challenges", &["PRIMARY KEY (id)"]),
    ("auth_rate_limits", &["PRIMARY KEY (scope, key_hash)"]),
    (
        "auth_local_identities",
        &[
            "PRIMARY KEY (id)",
            "UNIQUE (identifier_kind, identifier_normalized)",
        ],
    ),
    (
        "auth_local_credentials",
        &[
            "PRIMARY KEY (local_user_id)",
            "FOREIGN KEY (local_user_id) REFERENCES auth_local_identities(id) ON DELETE CASCADE",
        ],
    ),
    (
        "auth_local_one_time_tokens",
        &[
            "PRIMARY KEY (selector)",
            "FOREIGN KEY (local_user_id) REFERENCES auth_local_identities(id) ON DELETE CASCADE",
            "CHECK ((purpose = ANY (ARRAY['password_reset'::text, 'magic_link'::text])))",
        ],
    ),
];

type Index = (
    &'static str,
    &'static str,
    bool,
    &'static [&'static str],
    Option<&'static str>,
);
const INDEXES: &[Index] = &[
    (
        "auth_sessions",
        "idx_sessions_expires",
        false,
        &["expires_at"],
        None,
    ),
    (
        "auth_challenges",
        "idx_auth_challenges_expires",
        false,
        &["expires_at"],
        None,
    ),
    (
        "auth_rate_limits",
        "idx_auth_rate_limits_expires",
        false,
        &["expires_at"],
        None,
    ),
    (
        "auth_local_one_time_tokens",
        "idx_auth_local_one_time_tokens_expires",
        false,
        &["purpose", "expires_at"],
        None,
    ),
    (
        "auth_local_one_time_tokens",
        "idx_auth_local_one_time_tokens_user_purpose",
        false,
        &["local_user_id", "purpose"],
        None,
    ),
    (
        "auth_local_one_time_tokens",
        "idx_auth_local_one_time_tokens_one_magic_link",
        true,
        &["local_user_id"],
        Some("(purpose = 'magic_link'::text)"),
    ),
];

fn incompatible(object: &str) -> String {
    format!("PostgreSQL auth schema incompatible at {object}; bootstrap/migrate or repair with the schema owner before starting the runtime DML role")
}

pub(super) fn validate(client: &mut impl postgres::GenericClient) -> Result<(), String> {
    for &(table, columns) in COLUMNS {
        let table_ok: bool = client.query_one(
            "SELECT EXISTS(SELECT 1 FROM pg_class WHERE oid = to_regclass($1) AND relkind IN ('r', 'p'))", &[&table]
        ).map_err(|e| postgres_schema_error(table, &e))?.get(0);
        if !table_ok {
            return Err(incompatible(table));
        }
        let actual = client.query(
            "SELECT a.attname::text, format_type(a.atttypid, a.atttypmod), a.attnotnull,
                    pg_get_expr(d.adbin, d.adrelid), a.attgenerated::text
             FROM pg_attribute a LEFT JOIN pg_attrdef d ON d.adrelid = a.attrelid AND d.adnum = a.attnum
             WHERE a.attrelid = to_regclass($1) AND a.attnum > 0 AND NOT a.attisdropped", &[&table]
        ).map_err(|e| postgres_schema_error(table, &e))?;
        for &(name, ty, not_null, default) in columns {
            let Some(row) = actual.iter().find(|r| r.get::<_, String>(0) == name) else {
                return Err(incompatible(&format!("{table}.{name} (missing column)")));
            };
            if row.get::<_, String>(1) != ty
                || row.get::<_, bool>(2) != not_null
                || !row.get::<_, String>(4).is_empty()
                || default.is_some_and(|expected| {
                    row.get::<_, Option<String>>(3).as_deref() != Some(expected)
                })
            {
                return Err(incompatible(&format!(
                    "{table}.{name} (type, nullability or default)"
                )));
            }
        }
    }
    for &(table, constraints) in CONSTRAINTS {
        let actual = client
            .query(
                "SELECT pg_get_constraintdef(oid) FROM pg_constraint
             WHERE conrelid = to_regclass($1) AND convalidated AND NOT condeferrable",
                &[&table],
            )
            .map_err(|e| postgres_schema_error(table, &e))?;
        for expected in constraints {
            if !actual.iter().any(|r| r.get::<_, String>(0) == *expected) {
                return Err(incompatible(&format!(
                    "{table} (required constraint {expected})"
                )));
            }
        }
    }
    for &(table, index, unique, keys, predicate) in INDEXES {
        let row = client
            .query_opt(
                "SELECT i.indisvalid AND i.indisready, i.indisunique,
                    ARRAY(SELECT pg_get_indexdef(i.indexrelid, k, true)
                          FROM generate_series(1, i.indnkeyatts) k),
                    pg_get_expr(i.indpred, i.indrelid), am.amname::text, i.indnatts = i.indnkeyatts
             FROM pg_index i JOIN pg_class c ON c.oid = i.indexrelid
             JOIN pg_class t ON t.oid = i.indrelid JOIN pg_am am ON am.oid = c.relam
             WHERE t.oid = to_regclass($1) AND c.relnamespace = t.relnamespace AND c.relname = $2",
                &[&table, &index],
            )
            .map_err(|e| postgres_schema_error(index, &e))?;
        let Some(row) = row else {
            return Err(incompatible(index));
        };
        if !row.get::<_, bool>(0)
            || row.get::<_, bool>(1) != unique
            || row.get::<_, Vec<String>>(2) != keys
            || row.get::<_, Option<String>>(3).as_deref() != predicate
            || row.get::<_, String>(4) != "btree"
            || !row.get::<_, bool>(5)
        {
            return Err(incompatible(index));
        }
    }
    Ok(())
}
