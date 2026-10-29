//! Exact-plan PostgreSQL execution. No plan digest self-authorizes a write.
//! The caller retains progress across cancellation; sessions are never pooled.
use super::inventory::{json_digest, CapturedInputs};
use super::postgres::{History, Postgres, RelationalAdapter};
use super::{Diagnostic, Result};
use serde_json::{json, Value};

fn stale() -> Diagnostic {
    Diagnostic::new("DB_PLAN_STALE", "Execution artifact is incomplete or no longer matches captured inputs, policy, target, history or catalog. Review a new plan; do not blindly retry SQL.")
}

/// Validate the execution capability, ordered prefix evidence and exact captured
/// source before resolving authority. Old inspection-only artifacts fail closed.
fn validate(inputs: &CapturedInputs, saved: &Value) -> Result<usize> {
    let plan = &saved["plan"];
    let b = &plan["binding"];
    let base = b["applied_count"].as_u64().ok_or_else(stale)? as usize;
    let fingerprints = plan["execution"]["prefix_fingerprints"]
        .as_array()
        .ok_or_else(stale)?;
    let pending: Vec<_> = inputs
        .migrations
        .iter()
        .skip(base)
        .map(|m| {
            let mut s = m.summary();
            s["analysis"] = json!(super::analysis::analyze(&m.sql, &inputs.config.schema));
            s
        })
        .collect();
    if saved["format_version"] != 1
        || saved["outcome"] != "pass"
        || saved["plan_digest"] != json_digest(plan)
        || plan["authorized"] != false
        || plan["executable"] != false
        || plan["classification"] != "requires_review"
        || plan["candidate"]["outcome"] != "pass"
        || plan["execution"]["format"] != "postgresql-transactional-v1"
        || base > inputs.migrations.len()
        || fingerprints.len() != inputs.migrations.len() + 1
        || fingerprints.iter().any(|v| {
            !v.as_str().is_some_and(|s| {
                s.len() == 64
                    && s.bytes()
                        .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
            })
        })
        || plan["candidate"]["fingerprint"] != fingerprints[inputs.migrations.len()]
        || b["expected_base_digest"] != fingerprints[base]
        || b["base_catalog_digest"] != fingerprints[base]
        || b["adapter"] != "postgresql-catalog-v1"
        || b["canonicalization_version"] != 1
        || b["input_digest"] != inputs.input_digest
        || b["policy_digest"] != inputs.policy_digest
        || b["database"] != inputs.database_name
        || b["target_name"] != inputs.target_name
        || b["managed_schema"] != inputs.config.schema
        || plan["pending"] != json!(pending)
        || inputs
            .migrations
            .iter()
            .any(|m| !super::analysis::analyze(&m.sql, &inputs.config.schema).replay_supported)
    {
        return Err(stale());
    }
    Ok(base)
}

/// Resolve exact approval through the existing secret provider. The protected
/// provider and DB owner are trusted boundaries, not adversaries this can sandbox.
fn authorize(inputs: &CapturedInputs, saved: &Value) -> Result<()> {
    let reject = || {
        Diagnostic::new("DB_APPROVAL_REQUIRED", "A separately declared approval_secret must resolve to this exact reviewed plan digest. No target or ledger writes were attempted; provider details suppressed.")
    };
    let name = inputs.config.approval_secret.as_ref().ok_or_else(reject)?;
    let approval = crate::stdlib::secrets::require_from_manifest(name, &inputs.document)
        .map_err(|_| reject())?;
    if saved["plan_digest"].as_str() != Some(approval.expose()) {
        return Err(reject());
    }
    Ok(())
}

/// Validate a fresh snapshot under session ownership and native relation locks.
/// Matching progress is recognized at *every* planned prefix, never by final
/// catalog alone. An unrelated extension, dirty history or changed SQL blocks it.
async fn locked_prefix(
    pg: &Postgres,
    inputs: &CapturedInputs,
    saved: &Value,
) -> Result<(usize, History)> {
    let base = validate(inputs, saved)?;
    pg.begin_migration(&inputs.config.schema).await?;
    if json!(pg.identity(&inputs.endpoint).await?) != saved["plan"]["binding"]["target"] {
        return Err(stale());
    }
    let history = pg
        .writable_history(&inputs.database_name, &inputs.config.schema)
        .await?;
    let applied = super::report::prefix(&history, inputs)?;
    let catalog = pg
        .catalog(&inputs.config.schema, &inputs.endpoint.role)
        .await?;
    if !catalog.uncovered.is_empty()
        || applied < base
        || json_digest(&catalog) != saved["plan"]["execution"]["prefix_fingerprints"][applied]
        || (applied == base
            && json_digest(&history) != saved["plan"]["binding"]["base_history_digest"])
    {
        return Err(stale());
    }
    // A cross-scope dependent is not an owned object. Even non-cascading changes
    // are conservatively rejected rather than promising a transitive effect bound.
    let incoming = pg.rows("SELECT EXISTS(SELECT 1 FROM pg_catalog.pg_constraint k JOIN pg_catalog.pg_class c ON c.oid=k.confrelid JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace JOIN pg_catalog.pg_class f ON f.oid=k.conrelid JOIN pg_catalog.pg_namespace fn ON fn.oid=f.relnamespace WHERE n.nspname=$1 AND fn.nspname<>$1)", &[&inputs.config.schema]).await?;
    if incoming[0].get::<_, bool>(0) {
        return Err(Diagnostic::new(
            "DB_EXTERNAL_DEPENDENCY",
            "Incoming foreign keys from outside the managed schema block execution.",
        ));
    }
    Ok((applied, history))
}

fn mark_prefix(report: &mut Value, count: usize) {
    for entry in report["progress"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .take(count)
    {
        if entry["state"] != "committed" {
            entry["state"] = json!("already_applied");
        }
    }
    report["applied_count"] = json!(count);
}

/// Reconnect after an uncertain COMMIT, reacquire ownership and inspect *without*
/// replaying any SQL. Failure leaves the uncertain item unknown and preserves all
/// earlier acknowledged commits. A later invocation may repeat this inspection.
async fn reconcile(
    inputs: &CapturedInputs,
    saved: &Value,
    report: &mut Value,
    attempted: usize,
) -> Result<()> {
    let pg = connect(inputs).await?;
    pg.own_migrations().await?;
    let (count, _) = locked_prefix(&pg, inputs, saved).await?;
    pg.rollback().await?;
    if count < attempted || count > inputs.migrations.len() {
        return Err(stale());
    }
    mark_prefix(report, count);
    if count > attempted {
        report["progress"][attempted]["state"] = json!("committed");
        report["progress"][attempted]["reconciled"] = json!(true);
    } else {
        report["progress"][attempted]["state"] = json!("failed");
    }
    report["reconciliation"] = json!({"outcome":"pass","applied_count":count});
    Ok(())
}

async fn connect(inputs: &CapturedInputs) -> Result<Postgres> {
    let secret = crate::stdlib::secrets::require_from_manifest(
        &inputs.endpoint.connection_secret,
        &inputs.document,
    )
    .map_err(|_| {
        Diagnostic::new(
            "DB_SECRET",
            "Cannot resolve target connection secret; provider details suppressed.",
        )
    })?;
    Postgres::connect(&secret, &inputs.endpoint).await
}

/// Count only checks actually attempted on this target in this invocation.
/// Historical transition checks never become permanent verify assertions.
async fn checks(
    pg: &Postgres,
    checks: &std::collections::BTreeMap<String, String>,
    schema: &str,
    source: &str,
    phase: &str,
    report: &mut Value,
) -> Result<()> {
    if checks.is_empty() {
        return Ok(());
    }
    pg.check_purity(schema).await?;
    for (name, sql) in checks {
        let count = report["assertions"]["executed"].as_u64().unwrap_or(0);
        report["assertions"]["executed"] = json!(count + 1);
        report["assertions"]["outcome"] = json!("unknown");
        let result = pg.check(sql, schema).await;
        if report["assertions"]["checks"].is_null() {
            report["assertions"]["checks"] = json!([]);
        }
        report["assertions"]["checks"].as_array_mut().unwrap().push(json!({"source":source,"name":name,"phase":phase,"outcome":if result.is_ok() {"pass"} else {"fail"}}));
        report["assertions"]["outcome"] = json!(if result.is_ok() { "pass" } else { "fail" });
        result.map_err(|mut d| {
            d.source = Some(source.into());
            d
        })?;
    }
    Ok(())
}

/// Execute one immutable transition per transaction. No bootstrap precedes
/// authorization and locked revalidation. Any failure discards the physical
/// session; an uncertain commit only triggers fresh inspection, never SQL retry.
pub(super) async fn migrate(
    inputs: &CapturedInputs,
    saved: &Value,
    report: &mut Value,
) -> Result<()> {
    validate(inputs, saved)?;
    authorize(inputs, saved)?;
    report["database"] = json!(inputs.database_name);
    report["target"] = json!({"name":inputs.target_name,"schema":inputs.config.schema});
    report["plan_digest"] = saved["plan_digest"].clone();
    report["progress"] = json!(inputs
        .migrations
        .iter()
        .map(|m| json!({"source":m.name,"state":"not_run"}))
        .collect::<Vec<_>>());
    let pg = connect(inputs).await?;
    pg.own_migrations().await?;
    loop {
        let (applied, history) = locked_prefix(&pg, inputs, saved).await?;
        mark_prefix(report, applied);
        if applied == inputs.migrations.len() {
            pg.rollback().await?;
            report["outcome"] = json!("pass");
            return Ok(());
        }
        let migration = &inputs.migrations[applied];
        // A cancellation before COMMIT cannot commit; dropping this physical
        // socket rolls back. Keep in-flight progress conservative until cleanup.
        report["progress"][applied]["state"] = json!("unknown");
        let work = async {
            checks(
                &pg,
                &migration.requires,
                &inputs.config.schema,
                &migration.name,
                "requires",
                report,
            )
            .await?;
            pg.execute_captured(migration, &inputs.config.schema)
                .await?;
            checks(
                &pg,
                &migration.ensures,
                &inputs.config.schema,
                &migration.name,
                "ensures",
                report,
            )
            .await?;
            let catalog = pg
                .catalog(&inputs.config.schema, &inputs.endpoint.role)
                .await?;
            if !catalog.uncovered.is_empty()
                || json_digest(&catalog)
                    != saved["plan"]["execution"]["prefix_fingerprints"][applied + 1]
            {
                return Err(stale());
            }
            pg.record_migration(
                migration,
                applied + 1,
                &inputs.database_name,
                &inputs.config.schema,
                history.present,
            )
            .await?;
            Ok::<_, Diagnostic>(())
        }
        .await;
        if let Err(d) = work {
            let cleanup = pg.rollback().await;
            report["progress"][applied]["state"] =
                json!(if cleanup.is_ok() { "failed" } else { "unknown" });
            report["cleanup"] = json!(if cleanup.is_ok() { "pass" } else { "unknown" });
            return Err(d);
        }
        report["progress"][applied]["phase"] = json!("commit");
        if let Err(d) = pg.batch("COMMIT").await {
            drop(pg);
            if let Err(recovery) = reconcile(inputs, saved, report, applied).await {
                report["reconciliation"] = json!({"outcome":"unknown","diagnostic":recovery});
            }
            return Err(d);
        }
        report["progress"][applied]["state"] = json!("committed");
        report["applied_count"] = json!(applied + 1);
    }
}
