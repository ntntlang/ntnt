//! Exact-plan relational execution. No plan digest self-authorizes a write.
//! The caller retains progress across cancellation; sessions are never pooled.
use super::adapter::{self, Access, Adapter, Identity};
use super::adoption::{self, Adoption};
use super::inventory::{json_digest, CapturedInputs, CatalogDigests};
use super::model::History;
use super::{Diagnostic, Result};
use serde_json::{json, Value};

fn stale() -> Diagnostic {
    Diagnostic::new("DB_PLAN_STALE", "Execution artifact is incomplete or no longer matches captured inputs, policy, target, history or catalog. Review a new plan; do not blindly retry SQL.")
}

/// Whether the observed target identity is the one a plan bound, for
/// `reconcile`. Plans saved by ntnt 0.6.1 bind no `transport`: that release
/// connected only with sslmode=disable. Such a binding matches a plaintext
/// session whose identity is otherwise equal, so an attempt 0.6.1 left behind
/// can be resolved after upgrading, while `plan` refuses to run until it is.
/// `locked_prefix` (migrate) still compares exactly: it needs a new plan.
fn reconcile_target_matches(observed: &Value, bound: &Value) -> bool {
    if observed == bound {
        return true;
    }
    let (Some(observed), Some(bound)) = (observed.as_object(), bound.as_object()) else {
        return false;
    };
    let plaintext = observed
        .get("transport")
        .and_then(|transport| transport["sslmode"].as_str())
        == Some("disable");
    if bound.contains_key("transport") || !plaintext {
        return false;
    }
    let mut observed = observed.clone();
    observed.remove("transport");
    observed == *bound
}

/// Validate the descriptor, ordered prefix evidence and exact captured source.
/// This is pure: no I/O or approval lookup. Old inspection-only artifacts fail closed.
pub(super) fn validate(inputs: &CapturedInputs, saved: &Value) -> Result<usize> {
    let plan = &saved["plan"];
    let action = Adoption::from_plan(inputs, saved).map_err(|_| stale())?;
    let end = action.as_ref().map_or(inputs.migrations.len(), |a| a.count);
    let b = &plan["binding"];
    let base = b["applied_count"]
        .as_u64()
        .and_then(|value| usize::try_from(value).ok())
        .ok_or_else(stale)?;
    let reference = action.as_ref().map_or(base, |a| a.count);
    let fingerprints = plan["execution"]["prefix_fingerprints"]
        .as_array()
        .ok_or_else(stale)?;
    let pending: Vec<_> = inputs
        .migrations
        .iter()
        .skip(if action.is_some() {
            inputs.migrations.len()
        } else {
            base
        })
        .map(|m| {
            let mut s = m.summary();
            s["analysis"] = json!(super::analysis::analyze_in_scope(
                &inputs.config.engine,
                &m.sql,
                &inputs.config.schema,
                &inputs.history_schemas
            ));
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
        || plan["execution"]["format"]
            != if action.is_some() {
                adoption::format(&inputs.config.engine)
            } else {
                adapter::execution_format(&inputs.config.engine)
            }
        || base > inputs.migrations.len()
        || action.as_ref().is_some_and(|a| base != a.base_count())
        || fingerprints.len() != end + 1
        || fingerprints.iter().any(|v| {
            !v.as_str().is_some_and(|s| {
                s.len() == 64
                    && s.bytes()
                        .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
            })
        })
        || plan["candidate"]["fingerprint"] != fingerprints[end]
        || b["expected_base_digest"] != fingerprints[reference]
        || b["base_catalog_digest"] != fingerprints[reference]
        || b["format_version"] != 1
        || !b["base_history_digest"].as_str().is_some_and(|s| {
            s.len() == 64
                && s.bytes()
                    .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        })
        || b["adapter"] != adapter::catalog_format(&inputs.config.engine)
        || b["canonicalization_version"] != 1
        || b["input_digest"] != inputs.input_digest
        || b["policy_digest"] != inputs.policy_digest
        || b["database"] != inputs.database_name
        || b["target_name"] != inputs.target_name
        || b["managed_schema"] != inputs.config.schema
        || !limits_match(inputs, plan)
        || outside_objects(saved).is_err()
        || plan["pending"] != json!(pending)
        || inputs.migrations.iter().take(end).any(|m| {
            !super::analysis::analyze_for(&inputs.config.engine, &m.sql, &inputs.config.schema)
                .replay_supported
        })
    {
        return Err(stale());
    }
    Ok(base)
}

/// The objects outside the managed scope a plan inspects on the target, by
/// catalog key: everything its replay found the history creates there
/// (`report::Replayed::outside`). A plan without the field inspects none:
/// its history creates nothing there, or it was saved before plans recorded
/// them and inspected none either.
fn outside_objects(saved: &Value) -> Result<Vec<String>> {
    let Some(list) = saved["plan"]["execution"].get("outside_objects") else {
        return Ok(vec![]);
    };
    let keys: Option<Vec<String>> =
        list.as_array()
            .filter(|keys| !keys.is_empty())
            .and_then(|keys| {
                keys.iter()
                    .map(|key| {
                        key.as_str()
                            .filter(|key| !key.is_empty())
                            .map(str::to_owned)
                    })
                    .collect()
            });
    match keys {
        Some(keys) if keys.windows(2).all(|pair| pair[0] < pair[1]) => Ok(keys),
        _ => Err(stale()),
    }
}

/// A plan binds the effective limits. One saved before plans recorded them
/// implies the defaults, so it stays valid while no limits table is declared.
fn limits_match(inputs: &CapturedInputs, plan: &Value) -> bool {
    match plan["execution"].get("limits") {
        Some(saved) => inputs.plan_limits().as_ref() == Some(saved),
        None => inputs.config.limits.is_none(),
    }
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
/// The next pending migration, if it is a lone CREATE INDEX CONCURRENTLY.
fn pending_concurrent_index(
    inputs: &CapturedInputs,
    applied: usize,
) -> Option<super::analysis::ConcurrentIndex> {
    if inputs.config.engine != "postgresql" {
        return None;
    }
    let migration = inputs.migrations.get(applied)?;
    super::analysis::concurrent_index(&migration.sql, &inputs.config.schema)
}

/// Validation result: applied count, history, identity, adoption already recorded.
type Prefix = (usize, History, Identity, bool);

/// `base` is `validate(inputs, saved)`. The plan and captured inputs never
/// change during a run, so callers validate once, not for every migration.
/// `digests` fingerprints the target catalog, reusing unchanged objects.
async fn locked_prefix(
    db: &Adapter,
    inputs: &CapturedInputs,
    saved: &Value,
    base: usize,
    digests: &mut CatalogDigests,
) -> Result<Prefix> {
    let action = Adoption::from_plan(inputs, saved)?;
    db.begin_migration(&inputs.config.schema).await?;
    let identity = db.identity(&inputs.endpoint).await?;
    if json!(&identity) != saved["plan"]["binding"]["target"] {
        return Err(stale());
    }
    let history = if action.is_some() {
        let history = db.history().await?;
        if history.scope.is_some() {
            db.writable_history(&inputs.database_name, &inputs.config.schema)
                .await?
        } else {
            history
        }
    } else {
        db.writable_history(&inputs.database_name, &inputs.config.schema)
            .await?
    };
    let applied = super::report::prefix(&history, inputs)?;
    let outside = outside_objects(saved)?;
    let (uncovered, fingerprint) = db
        .fingerprint(
            &inputs.config.schema,
            &inputs.history_schemas,
            &outside,
            &inputs.endpoint.role,
            digests,
        )
        .await?;
    let adopted = if let Some(action) = &action {
        if !uncovered.is_empty()
            || fingerprint != saved["plan"]["execution"]["prefix_fingerprints"][action.count]
        {
            return Err(stale());
        }
        if history == action.after(inputs)? {
            true
        } else {
            action.before(inputs, &history)?;
            if json_digest(&history) != saved["plan"]["binding"]["base_history_digest"] {
                return Err(stale());
            }
            false
        }
    } else {
        // An index nobody recorded an attempt for is drift, not progress.
        if !uncovered.is_empty()
            || applied < base
            || fingerprint != saved["plan"]["execution"]["prefix_fingerprints"][applied]
            || (applied == base
                && json_digest(&history) != saved["plan"]["binding"]["base_history_digest"])
        {
            return Err(stale());
        }
        false
    };
    db.validate_execution_scope(&inputs.config.schema, &inputs.history_schemas, &outside)
        .await?;
    Ok((applied, history, identity, adopted))
}

fn mark_counts(report: &mut Value, history: &History) {
    for (key, value) in history.counts().as_object().unwrap() {
        report[key] = value.clone();
    }
}
fn mark_prefix(report: &mut Value, count: usize, history: &History) {
    let counts = history.counts();
    let adopted = counts["adopted_count"].as_u64().unwrap_or(0);
    let imported = counts["imported_count"].as_u64().unwrap_or(0);
    for (index, entry) in report["progress"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .take(count)
        .enumerate()
    {
        if entry["state"] != "committed" {
            entry["state"] = json!(if (index as u64) < adopted {
                "baseline_covered"
            } else if (index as u64) < imported {
                "history_imported"
            } else {
                "already_applied"
            });
        }
    }
    mark_counts(report, history);
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
    let base = validate(inputs, saved)?;
    let db = connect(inputs, saved).await?;
    db.own_migrations(inputs.limits().migrator_wait()).await?;
    let (count, history, identity, adopted) =
        locked_prefix(&db, inputs, saved, base, &mut CatalogDigests::default()).await?;
    db.rollback().await?;
    report["target"]["identity"] = json!(identity);
    if Adoption::from_plan(inputs, saved)?.is_some() {
        mark_counts(report, &history);
        report["progress"][0]["state"] = json!(if adopted { "committed" } else { "failed" });
        report["progress"][0]["reconciled"] = json!(true);
        report["reconciliation"] = history.counts();
        report["reconciliation"]["outcome"] = json!("pass");
        return Ok(());
    }
    if count < attempted || count > inputs.migrations.len() {
        return Err(stale());
    }
    mark_prefix(report, count, &history);
    if count > attempted {
        report["progress"][attempted]["state"] = json!("committed");
        report["progress"][attempted]["reconciled"] = json!(true);
    } else {
        report["progress"][attempted]["state"] = json!("failed");
    }
    report["reconciliation"] = history.counts();
    report["reconciliation"]["outcome"] = json!("pass");
    Ok(())
}

async fn connect(inputs: &CapturedInputs, saved: &Value) -> Result<Adapter> {
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
    Adapter::connect(
        &secret,
        &inputs.endpoint,
        &inputs.config.engine,
        Access::Execute(saved["plan"]["binding"]["target"].clone()),
    )
    .await
}

/// Count only checks actually attempted on this target in this invocation.
/// Historical transition checks never become permanent verify assertions.
async fn checks(
    db: &Adapter,
    checks: &std::collections::BTreeMap<String, String>,
    inputs: &CapturedInputs,
    outside: &[String],
    source: &str,
    phase: &str,
    report: &mut Value,
) -> Result<()> {
    if checks.is_empty() {
        return Ok(());
    }
    let schema = inputs.config.schema.as_str();
    let mut tables: Vec<_> = checks
        .values()
        .flat_map(|sql| super::analysis::check_tables(sql, schema))
        .collect();
    tables.sort();
    tables.dedup();
    db.lock_for_checks(&tables).await?;
    db.check_purity(schema, &inputs.history_schemas, outside)
        .await?;
    for (name, sql) in checks {
        let count = report["assertions"]["executed"].as_u64().unwrap_or(0);
        report["assertions"]["executed"] = json!(count + 1);
        report["assertions"]["outcome"] = json!("unknown");
        let result = db.check(sql, schema).await;
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

/// The limits and deadlines this run uses. SQLite keeps its fixed limits.
fn report_limits(inputs: &CapturedInputs, report: &mut Value) {
    if inputs.plan_limits().is_some() {
        report["limits"] = inputs.limits().report();
    }
}

/// Reserve part of the CLI's outer deadline for fresh reconciliation. Dropping
/// the timed-out future drops its owned physical connection before recovery starts.
async fn work_deadline<T>(
    future: impl std::future::Future<Output = Result<T>>,
    budget: std::time::Duration,
) -> Result<T> {
    tokio::time::timeout(budget, future).await.map_err(|_| {
        Diagnostic::new(
            "DB_TIMEOUT",
            "Migration work deadline exceeded; session discarded and committed progress retained.",
        )
    })?
}

fn uncertain_commit(report: &Value) -> Option<usize> {
    report["progress"]
        .as_array()?
        .iter()
        .position(|entry| entry["state"] == "unknown" && entry["phase"] == "commit")
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
    let action = Adoption::from_plan(inputs, saved)?;
    authorize(inputs, saved)?;
    report["database"] = json!(inputs.database_name);
    report["target"] = json!({"name":inputs.target_name,"schema":inputs.config.schema});
    report["plan_digest"] = saved["plan_digest"].clone();
    report["source"] =
        json!({"input_digest":inputs.input_digest,"policy_digest":inputs.policy_digest});
    report_limits(inputs, report);
    report["progress"] = json!(inputs
        .migrations
        .iter()
        .map(|m| json!({"source":m.name,"state":"not_run"}))
        .collect::<Vec<_>>());
    if let Some(action) = &action {
        report["progress"] = json!([{"kind":action.kind,"through_id":action.through_id,"covered_count":action.count,"state":"not_run","migration_sql_executed":false}]);
    }
    let result = work_deadline(
        apply_owned(inputs, saved, report),
        inputs.limits().work_deadline(),
    )
    .await;
    if result.as_ref().is_err_and(|d| d.code == "DB_TIMEOUT") {
        if let Some(attempted) = uncertain_commit(report) {
            if let Err(recovery) = reconcile(inputs, saved, report, attempted).await {
                report["reconciliation"] = json!({"outcome":"unknown","diagnostic":recovery});
            }
        }
    }
    result
}

/// Own the connection inside the cancellable work future, never outside it.
/// Attempts per migration when a lock is not granted within lock_timeout (or
/// this session is a deadlock victim). Each attempt rolls back completely.
const LOCK_ATTEMPTS: u32 = 4;

/// Backoff before a lock retry: 250 ms doubling, plus up to 100 ms of jitter
/// so competing sessions do not retry in lockstep.
fn lock_backoff(attempt: u32) -> std::time::Duration {
    let jitter = rand::random::<u64>() % 100;
    std::time::Duration::from_millis((250u64 << attempt.min(4)) + jitter)
}

async fn apply_owned(inputs: &CapturedInputs, saved: &Value, report: &mut Value) -> Result<()> {
    let action = Adoption::from_plan(inputs, saved)?;
    let base = validate(inputs, saved)?;
    let outside = outside_objects(saved)?;
    let db = connect(inputs, saved).await?;
    db.own_migrations(inputs.limits().migrator_wait()).await?;
    let mut digests = CatalogDigests::default();
    let mut lock_retries = 0u32;
    loop {
        let assertions_before = report["assertions"].clone();
        let (applied, history, identity, adopted) =
            match locked_prefix(&db, inputs, saved, base, &mut digests).await {
                Err(d) if d.code == "DB_LOCK_CONFLICT" && lock_retries + 1 < LOCK_ATTEMPTS => {
                    db.rollback().await?;
                    tokio::time::sleep(lock_backoff(lock_retries)).await;
                    lock_retries += 1;
                    continue;
                }
                other => other?,
            };
        report["capabilities"] = identity.capabilities();
        report["target"]["identity"] = json!(identity);
        if action.is_some() {
            mark_counts(report, &history);
            if adopted {
                db.rollback().await?;
                report["progress"][0]["state"] = json!(if action
                    .as_ref()
                    .is_some_and(|a| a.kind == "import_legacy_v1")
                {
                    "already_imported"
                } else {
                    "already_adopted"
                });
                report["outcome"] = json!("pass");
                return Ok(());
            }
        } else {
            mark_prefix(report, applied, &history);
            if applied == inputs.migrations.len() {
                db.rollback().await?;
                report["outcome"] = json!("pass");
                return Ok(());
            }
        }
        let progress_index = if action.is_some() { 0 } else { applied };
        let reference = action.as_ref().map_or(applied + 1, |a| a.count);
        let concurrent = if action.is_some() {
            None
        } else {
            pending_concurrent_index(inputs, applied)
        };
        if let Some(index) = &concurrent {
            build_concurrent(&db, inputs, saved, report, applied, &history, index).await?;
            report["progress"][progress_index]["lock_retries"] = json!(lock_retries);
            lock_retries = 0;
            report["covered_count"] = json!(applied + 1);
            report["applied_count"] =
                json!(history.counts()["applied_count"].as_u64().unwrap() + 1);
            if applied + 1 == inputs.migrations.len() {
                report["outcome"] = json!("pass");
                return Ok(());
            }
            continue;
        }
        report["progress"][progress_index]["state"] = json!("unknown");
        let mut locks = None;
        let work = async {
            if action.is_some() {
                checks(
                    &db,
                    &inputs.config.adoption_checks,
                    inputs,
                    &outside,
                    "manifest",
                    "adoption",
                    report,
                )
                .await?;
            } else {
                let migration = &inputs.migrations[applied];
                checks(
                    &db,
                    &migration.requires,
                    inputs,
                    &outside,
                    &migration.name,
                    "requires",
                    report,
                )
                .await?;
                db.execute_captured(
                    migration,
                    &inputs.config.schema,
                    inputs.limits().statement(),
                )
                .await?;
                checks(
                    &db,
                    &migration.ensures,
                    inputs,
                    &outside,
                    &migration.name,
                    "ensures",
                    report,
                )
                .await?;
            }
            locks = db.relation_locks(&inputs.config.schema).await?;
            let (uncovered, fingerprint) = db
                .fingerprint(
                    &inputs.config.schema,
                    &inputs.history_schemas,
                    &outside,
                    &inputs.endpoint.role,
                    &mut digests,
                )
                .await?;
            if !uncovered.is_empty()
                || fingerprint != saved["plan"]["execution"]["prefix_fingerprints"][reference]
            {
                return Err(stale());
            }
            if let Some(action) = &action {
                if action.kind == "import_legacy_v1" {
                    db.import_legacy_history(&inputs.database_name, &inputs.config.schema)
                        .await?;
                } else {
                    db.record_checkpoint(
                        &inputs.checkpoint(action.count)?,
                        &inputs.database_name,
                        &inputs.config.schema,
                    )
                    .await?;
                }
                if db
                    .writable_history(&inputs.database_name, &inputs.config.schema)
                    .await?
                    != action.after(inputs)?
                {
                    return Err(stale());
                }
            } else {
                db.record_migration(
                    &inputs.migrations[applied],
                    applied + 1,
                    &inputs.database_name,
                    &inputs.config.schema,
                    history.present,
                )
                .await?;
            }
            Ok::<_, Diagnostic>(())
        }
        .await;
        if let Err(d) = work {
            let cleanup = db.rollback().await;
            if d.code == "DB_LOCK_CONFLICT" && cleanup.is_ok() && lock_retries + 1 < LOCK_ATTEMPTS {
                // Nothing committed. Forget this attempt's check results, wait,
                // and run the whole migration transaction again.
                report["assertions"] = assertions_before;
                report["progress"][progress_index]["state"] = json!("not_run");
                tokio::time::sleep(lock_backoff(lock_retries)).await;
                lock_retries += 1;
                report["progress"][progress_index]["lock_retries"] = json!(lock_retries);
                continue;
            }
            report["progress"][progress_index]["state"] =
                json!(if cleanup.is_ok() { "failed" } else { "unknown" });
            report["cleanup"] = json!(if cleanup.is_ok() { "pass" } else { "unknown" });
            return Err(d);
        }
        report["progress"][progress_index]["phase"] = json!("commit");
        if let Err(d) = db.commit().await {
            drop(db);
            if let Err(recovery) = reconcile(inputs, saved, report, progress_index).await {
                report["reconciliation"] = json!({"outcome":"unknown","diagnostic":recovery});
            }
            return Err(d);
        }
        report["progress"][progress_index]["state"] = json!("committed");
        report["progress"][progress_index]["lock_retries"] = json!(lock_retries);
        if let Some(locks) = locks {
            report["progress"][progress_index]["locks"] = locks;
        }
        lock_retries = 0;
        if let Some(action) = &action {
            mark_counts(report, &action.after(inputs)?);
            report["outcome"] = json!("pass");
            return Ok(());
        }
        report["covered_count"] = json!(applied + 1);
        report["applied_count"] = json!(history.counts()["applied_count"].as_u64().unwrap() + 1);
        // Do not re-lock every managed table only to learn nothing is pending.
        // That extra lock pass can deadlock with live application writers.
        if applied + 1 == inputs.migrations.len() {
            report["outcome"] = json!("pass");
            return Ok(());
        }
    }
}

/// Run one lone CREATE INDEX CONCURRENTLY. The attempt is committed as
/// `running` before the build; the build runs outside any transaction while
/// the advisory lock excludes other migrators; a short transaction then
/// verifies the planned catalog and marks the attempt `applied`. A failed
/// build drops its invalid index and deletes the attempt. Anything this run
/// cannot finish leaves the attempt for `ntnt db reconcile`.
async fn build_concurrent(
    db: &Adapter,
    inputs: &CapturedInputs,
    saved: &Value,
    report: &mut Value,
    applied: usize,
    history: &History,
    index: &super::analysis::ConcurrentIndex,
) -> Result<()> {
    let schema = &inputs.config.schema;
    let migration = &inputs.migrations[applied];
    let progress = &mut report["progress"][applied];
    progress["non_transactional"] = json!(true);
    if let Err(d) = db
        .record_attempt(
            migration,
            applied + 1,
            &inputs.database_name,
            schema,
            history.present,
        )
        .await
    {
        progress["state"] = json!("failed");
        return Err(d);
    }
    progress["state"] = json!("unknown");
    progress["phase"] = json!("record_attempt");
    db.commit().await?;
    progress["attempt_recorded"] = json!(true);
    progress["phase"] = json!("build");
    let limit = inputs.limits().statement();
    if let Err(d) = db.build_concurrent_index(migration, schema, limit).await {
        // Delete the attempt only once the index is confirmed absent. A build
        // can succeed and still return an error (e.g. restoring settings
        // failed); a valid or unknown index keeps the attempt for reconcile.
        let cleanup = async {
            let mut dropped = false;
            if db.index_validity(schema, &index.name).await? == Some(false) {
                db.drop_invalid_index(schema, &index.name, limit).await?;
                dropped = true;
            }
            if db.index_validity(schema, &index.name).await?.is_some() {
                return Ok::<_, Diagnostic>(None);
            }
            db.begin_migration(schema).await?;
            db.resolve_attempt(applied + 1, false).await?;
            db.commit().await?;
            Ok(Some(if dropped {
                "dropped_invalid_index"
            } else {
                "pass"
            }))
        }
        .await;
        match cleanup {
            Ok(Some(done)) => {
                progress["state"] = json!("failed");
                progress["cleanup"] = json!(done);
            }
            Ok(None) => progress["cleanup"] = json!("attempt_retained"),
            Err(_) => progress["cleanup"] = json!("unknown"),
        }
        return Err(d);
    }
    progress["phase"] = json!("record");
    let (position, catalog) = locked_attempt(db, inputs, saved).await?;
    if position != applied + 1
        || !catalog.uncovered.is_empty()
        || json_digest(&catalog) != saved["plan"]["execution"]["prefix_fingerprints"][applied + 1]
    {
        return Err(stale());
    }
    db.resolve_attempt(position, true).await?;
    progress["phase"] = json!("commit");
    db.commit().await?;
    progress["state"] = json!("committed");
    Ok(())
}

/// Open a migration transaction and require history to end in exactly one
/// `running` attempt for the next captured migration, within this plan.
/// Returns its 1-based position and the observed catalog.
async fn locked_attempt(
    db: &Adapter,
    inputs: &CapturedInputs,
    saved: &Value,
) -> Result<(usize, super::model::Catalog)> {
    let base = validate(inputs, saved)?;
    db.begin_migration(&inputs.config.schema).await?;
    let identity = db.identity(&inputs.endpoint).await?;
    // Reconcile accepts a 0.6.1 binding here too. Migrate gets here only after
    // locked_prefix compared the binding exactly on this same session.
    if !reconcile_target_matches(&json!(&identity), &saved["plan"]["binding"]["target"]) {
        return Err(stale());
    }
    let history = db
        .writable_history(&inputs.database_name, &inputs.config.schema)
        .await?;
    let (applied, attempt) = super::report::prefix_with_attempt(&history, inputs)?;
    let position = attempt.ok_or_else(|| {
        Diagnostic::new(
            "DB_RECONCILE_MISMATCH",
            "The running attempt disappeared while it was being resolved; nothing was changed.",
        )
    })?;
    if applied < base || applied + 1 != position {
        return Err(stale());
    }
    let catalog = db
        .catalog(
            &inputs.config.schema,
            &inputs.history_schemas,
            &outside_objects(saved)?,
            &inputs.endpoint.role,
        )
        .await?;
    Ok((position, catalog))
}

/// `ntnt db reconcile`: resolve a `running` attempt left by an interrupted
/// non-transactional migration. It never runs migration SQL. It records the
/// attempt only if the planned index exists and is valid, rolls it back
/// (dropping an invalid index) only if the database is exactly the plan's
/// state before or after it, and otherwise changes nothing.
pub(super) async fn reconcile_attempt(
    inputs: &CapturedInputs,
    saved: &Value,
    report: &mut Value,
) -> Result<()> {
    validate(inputs, saved)?;
    if Adoption::from_plan(inputs, saved)?.is_some() {
        return Err(Diagnostic::new(
            "DB_CONFIG",
            "Adoption plans have no non-transactional attempts to reconcile.",
        ));
    }
    authorize(inputs, saved)?;
    report["database"] = json!(inputs.database_name);
    report["target"] = json!({"name":inputs.target_name,"schema":inputs.config.schema});
    report["plan_digest"] = saved["plan_digest"].clone();
    report_limits(inputs, report);
    work_deadline(
        reconcile_owned(inputs, saved, report),
        inputs.limits().work_deadline(),
    )
    .await
}

async fn reconcile_owned(inputs: &CapturedInputs, saved: &Value, report: &mut Value) -> Result<()> {
    let schema = &inputs.config.schema;
    let fingerprints = &saved["plan"]["execution"]["prefix_fingerprints"];
    let db = connect(inputs, saved).await?;
    db.own_migrations(inputs.limits().migrator_wait()).await?;
    db.begin_migration(schema).await?;
    let identity = db.identity(&inputs.endpoint).await?;
    if !reconcile_target_matches(&json!(&identity), &saved["plan"]["binding"]["target"]) {
        return Err(stale());
    }
    report["target"]["identity"] = json!(identity);
    let history = db.writable_history(&inputs.database_name, schema).await?;
    let (applied, attempt) = super::report::prefix_with_attempt(&history, inputs)?;
    if attempt.is_none() {
        db.rollback().await?;
        report["reconciliation"] = json!({"outcome":"pass","resolution":"none"});
        report["outcome"] = json!("pass");
        return Ok(());
    }
    // Only an attempt this plan can explain is resolved.
    let mismatch = |name: &str| {
        let mut d = Diagnostic::new(
            "DB_RECONCILE_MISMATCH",
            "The database matches neither the plan's state before this migration nor after it.",
        );
        d.source = Some(name.to_string());
        d
    };
    let source = inputs.migrations[applied].name.clone();
    let index = pending_concurrent_index(inputs, applied).ok_or_else(|| mismatch(&source))?;
    locked_attempt_scope(applied, inputs, saved)?;
    if db.index_building(schema, &index.name).await? {
        let mut d = Diagnostic::new(
            "DB_ATTEMPT_ACTIVE",
            "The attempt's index is still being built by another session.",
        );
        d.source = Some(index.name.clone());
        return Err(d);
    }
    let mut catalog = db
        .catalog(
            schema,
            &inputs.history_schemas,
            &outside_objects(saved)?,
            &inputs.endpoint.role,
        )
        .await?;
    if !catalog.uncovered.is_empty() {
        return Err(mismatch(&index.name));
    }
    let validity = db.index_validity(schema, &index.name).await?;
    report["reconciliation"] = json!({
        "source": source,
        "observed_index": match validity { Some(true) => "valid", Some(false) => "invalid", None => "absent" },
    });
    let resolution = match validity {
        Some(true) => {
            if json_digest(&catalog) != fingerprints[applied + 1] {
                return Err(mismatch(&index.name));
            }
            db.resolve_attempt(applied + 1, true).await?;
            pending_write(report, "completed");
            db.commit().await?;
            "completed"
        }
        Some(false) => {
            // Treated as finished, the invalid index must make the catalog
            // exactly the planned one; only then is it ours to drop.
            if let Some(object) = catalog.objects.get_mut(&format!("index:{}", index.name)) {
                for flag in ["valid", "ready", "live"] {
                    object[flag] = json!(true);
                }
            }
            if json_digest(&catalog) != fingerprints[applied + 1] {
                return Err(mismatch(&index.name));
            }
            db.rollback().await?;
            pending_write(report, "dropped_invalid_index");
            db.drop_invalid_index(schema, &index.name, inputs.limits().statement())
                .await?;
            report["reconciliation"]["dropped_invalid_index"] = json!(true);
            report["reconciliation"]["outcome"] = json!("fail");
            let (position, after) = locked_attempt(&db, inputs, saved).await?;
            if position != applied + 1 || json_digest(&after) != fingerprints[applied] {
                let mut d = Diagnostic::new(
                    "DB_RECONCILE_INCOMPLETE",
                    "The invalid index was dropped, but the database then differed from the plan's state before this migration; the attempt was kept.",
                );
                d.source = Some(index.name.clone());
                return Err(d);
            }
            db.resolve_attempt(position, false).await?;
            pending_write(report, "rolled_back");
            db.commit().await?;
            "rolled_back"
        }
        None => {
            if json_digest(&catalog) != fingerprints[applied] {
                return Err(mismatch(&index.name));
            }
            db.resolve_attempt(applied + 1, false).await?;
            pending_write(report, "not_started");
            db.commit().await?;
            "not_started"
        }
    };
    report["reconciliation"]["resolution"] = json!(resolution);
    report["reconciliation"]["outcome"] = json!("pass");
    if let Some(r) = report["reconciliation"].as_object_mut() {
        r.remove("pending_write");
    }
    report["outcome"] = json!("pass");
    Ok(())
}

/// Mark a reconciliation write whose outcome is not yet acknowledged. A lost
/// commit or interrupted DROP then reports `unknown`, not `fail`.
fn pending_write(report: &mut Value, write: &str) {
    report["reconciliation"]["pending_write"] = json!(write);
    report["reconciliation"]["outcome"] = json!("unknown");
}

/// The attempt must lie inside the plan's pending range.
fn locked_attempt_scope(applied: usize, inputs: &CapturedInputs, saved: &Value) -> Result<()> {
    if applied < validate(inputs, saved)? {
        return Err(stale());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Captured inputs for a one-target PostgreSQL manifest plus `extra` TOML.
    fn captured(extra: &str) -> (tempfile::TempDir, CapturedInputs) {
        let project = tempfile::tempdir().unwrap();
        std::fs::create_dir(project.path().join("migrations")).unwrap();
        let manifest = project.path().join("ntnt.toml");
        std::fs::write(
            &manifest,
            format!(
                r#"
[databases.app]
engine = "postgresql"
schema = "app"
migrations_dir = "migrations"
[databases.app.targets.test]
connection_secret = "DB_TARGET"
database = "target"
role = "owner"
[secrets.DB_TARGET]
{extra}"#
            ),
        )
        .unwrap();
        let inputs = CapturedInputs::load(&manifest, "test", None).unwrap();
        (project, inputs)
    }
    /// A descriptor fixture for the pure validator, not database evidence.
    fn descriptor(inputs: &CapturedInputs) -> Value {
        let fingerprint = "a".repeat(64);
        json!({
            "binding": {"format_version":1,"applied_count":0,
                "expected_base_digest":fingerprint,"base_catalog_digest":fingerprint,
                "base_history_digest":fingerprint,"adapter":"postgresql-catalog-v1",
                "canonicalization_version":1,"input_digest":inputs.input_digest,
                "policy_digest":inputs.policy_digest,"database":"app",
                "target_name":"test","managed_schema":"app"},
            "execution":{"format":"postgresql-transactional-v1","prefix_fingerprints":[fingerprint]},
            "candidate":{"outcome":"pass","fingerprint":fingerprint},
            "authorized":false,"executable":false,"classification":"requires_review","pending":[]
        })
    }
    fn saved(plan: &Value) -> Value {
        json!({"format_version":1,"outcome":"pass",
            "plan_digest":json_digest(plan),"plan":plan})
    }

    #[test]
    fn plans_bind_limits_and_plans_without_them_imply_the_defaults() {
        let (_project, inputs) = captured("");
        let mut plan = descriptor(&inputs);
        // Saved before plans recorded limits: the defaults are implied.
        assert_eq!(validate(&inputs, &saved(&plan)).unwrap(), 0);
        for (limits, valid) in [
            (
                json!({"statement_seconds":90,"migrator_wait_seconds":5}),
                true,
            ),
            (
                json!({"statement_seconds":91,"migrator_wait_seconds":5}),
                false,
            ),
            (json!({"statement_seconds":90}), false),
            (Value::Null, false),
        ] {
            plan["execution"]["limits"] = limits.clone();
            assert_eq!(validate(&inputs, &saved(&plan)).is_ok(), valid, "{limits}");
        }
        // Declared limits are bound exactly, never implied.
        let (_project, limited) = captured("[databases.app.limits]\nstatement_seconds = 600\n");
        let mut plan = descriptor(&limited);
        for (limits, valid) in [
            (None, false),
            (
                Some(json!({"statement_seconds":90,"migrator_wait_seconds":5})),
                false,
            ),
            (
                Some(json!({"statement_seconds":600,"migrator_wait_seconds":5})),
                true,
            ),
        ] {
            if let Some(limits) = limits {
                plan["execution"]["limits"] = limits;
            }
            let result = validate(&limited, &saved(&plan));
            assert_eq!(result.is_ok(), valid, "{plan}");
            if !valid {
                assert_eq!(result.unwrap_err().code, "DB_PLAN_STALE");
            }
        }
    }

    #[test]
    fn applied_count_cannot_wrap_into_a_valid_prefix() {
        let (_project, inputs) = captured("");
        let mut plan = descriptor(&inputs);
        assert_eq!(validate(&inputs, &saved(&plan)).unwrap(), 0);
        for value in [
            json!(1),
            json!(u64::from(u32::MAX) + 1),
            json!(u64::MAX),
            json!(-1),
            Value::Null,
        ] {
            plan["binding"]["applied_count"] = value;
            assert_eq!(
                validate(&inputs, &saved(&plan)).unwrap_err().code,
                "DB_PLAN_STALE"
            );
        }
    }

    #[test]
    fn reconcile_accepts_a_0_6_1_binding_only_over_plaintext() {
        let identity = |transport: Value| {
            json!({"cluster":"7","database_oid":16384,"database":"app","role":"owner",
                "version":170004,"settings":{"encoding":"UTF8"},"authority":{"roles":[]},
                "transport":transport})
        };
        let plaintext = identity(
            json!({"sslmode":"disable","roots":null,"tls_version":null,"server_ssl":false}),
        );
        let tls = identity(
            json!({"sslmode":"verify-full","roots":"system","tls_version":"TLSv1.3","server_ssl":true}),
        );
        let unverified = identity(
            json!({"sslmode":"require","roots":null,"tls_version":"TLSv1.3","server_ssl":true}),
        );
        // ntnt 0.6.1 bound the same identity without a transport.
        let mut legacy = plaintext.clone();
        legacy.as_object_mut().unwrap().remove("transport");
        assert!(reconcile_target_matches(&plaintext, &legacy));
        assert!(!reconcile_target_matches(&tls, &legacy));
        assert!(!reconcile_target_matches(&unverified, &legacy));
        // Every other field must still be equal.
        let mut moved = legacy.clone();
        moved["cluster"] = json!("8");
        assert!(!reconcile_target_matches(&plaintext, &moved));
        let mut partial = legacy.clone();
        partial.as_object_mut().unwrap().remove("role");
        assert!(!reconcile_target_matches(&plaintext, &partial));
        // A binding with a transport compares exactly.
        assert!(reconcile_target_matches(&plaintext, &plaintext));
        assert!(reconcile_target_matches(&tls, &tls));
        assert!(!reconcile_target_matches(&plaintext, &tls));
        assert!(!reconcile_target_matches(&tls, &plaintext));
        assert!(!reconcile_target_matches(&unverified, &tls));
    }

    #[test]
    fn work_deadline_retains_prefix_and_identifies_unacknowledged_commit() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let mut progress = json!({"progress":[]});
        let error = runtime
            .block_on(work_deadline(
                async {
                    progress["progress"] = json!([
                        {"state":"committed","phase":"commit"},
                        {"state":"unknown","phase":"commit"},
                        {"state":"not_run"}
                    ]);
                    std::future::pending::<Result<()>>().await
                },
                std::time::Duration::from_millis(5),
            ))
            .unwrap_err();
        assert_eq!(error.code, "DB_TIMEOUT");
        assert_eq!(progress["progress"][0]["state"], "committed");
        assert_eq!(uncertain_commit(&progress), Some(1));
        progress["progress"][1]["state"] = json!("committed");
        assert_eq!(uncertain_commit(&progress), None);
    }

    /// A plan records every outside object its replay found, whatever the
    /// length of its key (a routine's identity arguments can run to
    /// thousands of characters) or the number of keys, so `migrate` must
    /// accept what `plan` wrote (#420, review r4654889050).
    #[test]
    fn plans_keep_every_outside_object_key_planning_records() {
        let long = format!(
            "public.routine:record({})",
            vec!["at timestamp with time zone"; 70].join(", ")
        );
        assert!(long.len() > 1024);
        let many: Vec<String> = (0..10_001)
            .map(|i| format!("public.relation:t{i:05}"))
            .collect();
        for keys in [vec![long], many] {
            let saved = json!({"plan": {"execution": {"outside_objects": keys.clone()}}});
            assert_eq!(outside_objects(&saved).unwrap(), keys);
        }
        // Still stale: an empty list, an empty key, keys out of order or
        // repeated, and a value that is not a list.
        for bad in [
            json!([]),
            json!([""]),
            json!(["b", "a"]),
            json!(["a", "a"]),
            json!("a"),
        ] {
            let saved = json!({"plan": {"execution": {"outside_objects": bad.clone()}}});
            assert!(outside_objects(&saved).is_err(), "{bad}");
        }
    }
}
