//! Exact-plan relational execution. No plan digest self-authorizes a write.
//! The caller retains progress across cancellation; sessions are never pooled.
use super::adapter::{self, Access, Adapter, Identity};
use super::adoption::{self, Adoption};
use super::inventory::{json_digest, CapturedInputs};
use super::model::History;
use super::{Diagnostic, Result};
use serde_json::{json, Value};

fn stale() -> Diagnostic {
    Diagnostic::new("DB_PLAN_STALE", "Execution artifact is incomplete or no longer matches captured inputs, policy, target, history or catalog. Review a new plan; do not blindly retry SQL.")
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
            s["analysis"] = json!(super::analysis::analyze_for(
                &inputs.config.engine,
                &m.sql,
                &inputs.config.schema
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

/// Validation result: applied count, history, identity, adoption already
/// recorded, and whether the next concurrent index already exists and is valid
/// (built by an earlier run that stopped before recording it).
type Prefix = (usize, History, Identity, bool, bool);

async fn locked_prefix(db: &Adapter, inputs: &CapturedInputs, saved: &Value) -> Result<Prefix> {
    let base = validate(inputs, saved)?;
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
    let mut prebuilt = false;
    let catalog = db
        .catalog(&inputs.config.schema, &inputs.endpoint.role)
        .await?;
    let adopted = if let Some(action) = &action {
        if !catalog.uncovered.is_empty()
            || json_digest(&catalog)
                != saved["plan"]["execution"]["prefix_fingerprints"][action.count]
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
        // A concurrent build that failed or was interrupted leaves an invalid
        // index; one that finished but was not recorded leaves a valid one.
        let mut reference = applied;
        if let Some(index) = pending_concurrent_index(inputs, applied) {
            match db
                .index_validity(&inputs.config.schema, &index.name)
                .await?
            {
                Some(false) => {
                    let mut d = Diagnostic::new(
                        "DB_INVALID_INDEX",
                        "An earlier concurrent build left an invalid index.",
                    );
                    d.source = Some(index.name);
                    return Err(d);
                }
                Some(true) => {
                    prebuilt = true;
                    reference = applied + 1;
                }
                None => (),
            }
        }
        if !catalog.uncovered.is_empty()
            || applied < base
            || json_digest(&catalog) != saved["plan"]["execution"]["prefix_fingerprints"][reference]
            || (applied == base
                && json_digest(&history) != saved["plan"]["binding"]["base_history_digest"])
        {
            return Err(stale());
        }
        false
    };
    db.validate_execution_scope(&inputs.config.schema).await?;
    Ok((applied, history, identity, adopted, prebuilt))
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
    let db = connect(inputs, saved).await?;
    db.own_migrations().await?;
    let (count, history, identity, adopted, _) = locked_prefix(&db, inputs, saved).await?;
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
    schema: &str,
    source: &str,
    phase: &str,
    report: &mut Value,
) -> Result<()> {
    if checks.is_empty() {
        return Ok(());
    }
    let mut tables: Vec<_> = checks
        .values()
        .flat_map(|sql| super::analysis::check_tables(sql, schema))
        .collect();
    tables.sort();
    tables.dedup();
    db.lock_for_checks(&tables).await?;
    db.check_purity(schema).await?;
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
        std::time::Duration::from_secs(100),
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
    let db = connect(inputs, saved).await?;
    db.own_migrations().await?;
    let mut lock_retries = 0u32;
    // Set when this run built a concurrent index, so the recording pass that
    // follows reports it as built here rather than by an earlier run.
    let mut built_here = false;
    let mut dropped_invalid = false;
    loop {
        let assertions_before = report["assertions"].clone();
        let (applied, history, identity, adopted, prebuilt) =
            match locked_prefix(&db, inputs, saved).await {
                Err(d) if d.code == "DB_LOCK_CONFLICT" && lock_retries + 1 < LOCK_ATTEMPTS => {
                    db.rollback().await?;
                    tokio::time::sleep(lock_backoff(lock_retries)).await;
                    lock_retries += 1;
                    continue;
                }
                Err(d) if d.code == "DB_INVALID_INDEX" && !dropped_invalid => {
                    db.rollback().await?;
                    let name = d.source.clone().unwrap_or_default();
                    db.drop_invalid_index(&inputs.config.schema, &name).await?;
                    dropped_invalid = true;
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
            report["progress"][progress_index]["non_transactional"] = json!(true);
            if dropped_invalid {
                report["progress"][progress_index]["dropped_invalid_index"] = json!(true);
            }
            if !prebuilt {
                // CREATE INDEX CONCURRENTLY cannot run in a transaction. Release
                // the validation snapshot; the advisory lock still excludes
                // other migrators. A crash after the build is recovered by the
                // next run, which finds the valid index and records it.
                db.rollback().await?;
                report["progress"][progress_index]["state"] = json!("unknown");
                if let Err(d) = db
                    .build_concurrent_index(&inputs.migrations[applied], &inputs.config.schema)
                    .await
                {
                    let invalid = db.index_validity(&inputs.config.schema, &index.name).await;
                    let cleanup = match invalid {
                        Ok(Some(false)) => db
                            .drop_invalid_index(&inputs.config.schema, &index.name)
                            .await
                            .map(|_| "dropped_invalid_index"),
                        Ok(_) => Ok("pass"),
                        Err(e) => Err(e),
                    };
                    report["progress"][progress_index]["state"] = json!("failed");
                    report["progress"][progress_index]["cleanup"] =
                        json!(cleanup.unwrap_or("unknown"));
                    return Err(d);
                }
                built_here = true;
                continue;
            }
            if !built_here {
                report["progress"][progress_index]["index_already_built"] = json!(true);
            }
        }
        report["progress"][progress_index]["state"] = json!("unknown");
        let mut locks = None;
        let work = async {
            if action.is_some() {
                checks(
                    &db,
                    &inputs.config.adoption_checks,
                    &inputs.config.schema,
                    "manifest",
                    "adoption",
                    report,
                )
                .await?;
            } else if concurrent.is_none() {
                let migration = &inputs.migrations[applied];
                checks(
                    &db,
                    &migration.requires,
                    &inputs.config.schema,
                    &migration.name,
                    "requires",
                    report,
                )
                .await?;
                db.execute_captured(migration, &inputs.config.schema)
                    .await?;
                checks(
                    &db,
                    &migration.ensures,
                    &inputs.config.schema,
                    &migration.name,
                    "ensures",
                    report,
                )
                .await?;
            }
            if concurrent.is_none() {
                locks = db.relation_locks(&inputs.config.schema).await?;
            }
            let catalog = db
                .catalog(&inputs.config.schema, &inputs.endpoint.role)
                .await?;
            if !catalog.uncovered.is_empty()
                || json_digest(&catalog)
                    != saved["plan"]["execution"]["prefix_fingerprints"][reference]
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
        built_here = false;
        dropped_invalid = false;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn applied_count_cannot_wrap_into_a_valid_prefix() {
        let project = tempfile::tempdir().unwrap();
        std::fs::create_dir(project.path().join("migrations")).unwrap();
        let manifest = project.path().join("ntnt.toml");
        std::fs::write(
            &manifest,
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
"#,
        )
        .unwrap();
        let inputs = CapturedInputs::load(&manifest, "test", None).unwrap();
        // A descriptor fixture for the pure validator, not database evidence.
        let fingerprint = "a".repeat(64);
        let mut plan = json!({
            "binding": {"format_version":1,"applied_count":0,
                "expected_base_digest":fingerprint,"base_catalog_digest":fingerprint,
                "base_history_digest":fingerprint,"adapter":"postgresql-catalog-v1",
                "canonicalization_version":1,"input_digest":inputs.input_digest,
                "policy_digest":inputs.policy_digest,"database":"app",
                "target_name":"test","managed_schema":"app"},
            "execution":{"format":"postgresql-transactional-v1","prefix_fingerprints":[fingerprint]},
            "candidate":{"outcome":"pass","fingerprint":fingerprint},
            "authorized":false,"executable":false,"classification":"requires_review","pending":[]
        });
        let saved = |plan: &Value| {
            json!({"format_version":1,"outcome":"pass",
            "plan_digest":json_digest(plan),"plan":plan})
        };
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
}
