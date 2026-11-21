//! Shared report construction: authored history, observed target, expected prefix.
use super::adapter::{self, Access, Adapter, Identity};
use super::analysis::analyze_for;
use super::inventory::{json_digest, CapturedInputs};
use super::model::{Catalog, History};
use super::{Diagnostic, Result};
use serde_json::{json, Value};

/// A scratch replay's result and its rollback's, combined: a failed replay
/// stays the reported cause, and a cleanup that also failed comes back
/// separately to be reported after it, never instead of it.
fn replay_outcome(work: Result<()>, cleanup: Result<()>) -> (Result<()>, Option<Diagnostic>) {
    match (work, cleanup) {
        (work, Ok(())) => (work, None),
        (Ok(()), Err(cleanup)) => (Err(cleanup), None),
        (Err(cause), Err(cleanup)) => (Err(cause), Some(cleanup)),
    }
}
/// Add a scratch replay's failures to `report`: the replay's own error first,
/// then a cleanup that also failed.
fn record_replay(report: &mut Value, replay: Result<()>, cleanup_failure: Option<Diagnostic>) {
    for d in replay.err().into_iter().chain(cleanup_failure) {
        diagnostic(report, d);
    }
}
fn diagnostic(report: &mut Value, d: Diagnostic) {
    report["diagnostics"].as_array_mut().unwrap().push(json!(d));
}
pub(super) fn failure(command: &str, d: Diagnostic) -> Value {
    json!({"format_version":1,"command":command,"observed_at":chrono::Utc::now().to_rfc3339(),
        "outcome":"unknown","diagnostics":[d],"assertions":{"outcome":"not_run","executed":0}})
}
/// Verify that applied entries are the exact immutable prefix of captured source history.
/// Dirty, missing, reordered or edited entries require reconciliation; none are repaired.
/// Applied prefix; refuses history that ends in an unresolved attempt.
pub(super) fn prefix(history: &History, inputs: &CapturedInputs) -> Result<usize> {
    let (covered, attempt) = prefix_with_attempt(history, inputs)?;
    if let Some(position) = attempt {
        let mut d = Diagnostic::new(
            "DB_ATTEMPT_UNRESOLVED",
            "A non-transactional migration was started but never recorded as applied or rolled back.",
        );
        d.source = inputs.migrations.get(position - 1).map(|m| m.name.clone());
        return Err(d);
    }
    Ok(covered)
}
/// Applied prefix plus the position of a trailing `running` attempt, which is
/// only valid as the last scoped entry for the next captured migration.
pub(super) fn prefix_with_attempt(
    history: &History,
    inputs: &CapturedInputs,
) -> Result<(usize, Option<usize>)> {
    if history.scope.as_ref().is_some_and(|scope| {
        *scope != json!({"database":inputs.database_name,"schema":inputs.config.schema})
    }) {
        return Err(Diagnostic::new(
            "DB_LEDGER_SCOPE",
            "History belongs to a different logical database or managed schema.",
        ));
    }
    if history.entries.len() > inputs.migrations.len() {
        return Err(Diagnostic::new(
            "DB_HISTORY_MISSING",
            "Applied history contains migrations missing from the captured inventory.",
        ));
    }
    let mut covered = 0;
    let mut applied_started = false;
    let has_checkpoint = history
        .entries
        .first()
        .is_some_and(|e| e.state == "adopted");
    let mut attempt = None;
    for (i, entry) in history.entries.iter().enumerate() {
        if attempt.is_some() {
            return Err(Diagnostic::new(
                "DB_HISTORY_DIRTY",
                "Ledger records work after an unresolved attempt; explicit reconciliation is required.",
            ));
        }
        if i == 0 && entry.state == "adopted" {
            if history.scope.is_none() {
                return Err(Diagnostic::new(
                    "DB_LEDGER_SCOPE",
                    "An adopted checkpoint must bind the logical database and managed schema.",
                ));
            }
            let count = usize::try_from(entry.position)
                .ok()
                .filter(|n| *n > 0 && *n <= inputs.migrations.len())
                .ok_or_else(|| {
                    Diagnostic::new(
                        "DB_HISTORY_MISSING",
                        "The adopted source prefix is missing or invalid.",
                    )
                })?;
            let expected = inputs.checkpoint(count)?;
            if entry.id != expected.id || entry.name != expected.name {
                return Err(Diagnostic::new(
                    "DB_HISTORY_DIVERGENT",
                    "Adopted checkpoint does not identify this ordered source prefix.",
                ));
            }
            if entry.checksum != expected.checksum {
                return Err(Diagnostic::new(
                    "DB_HISTORY_EDITED",
                    "Source bytes covered by the adopted checkpoint have changed.",
                ));
            }
            covered = count;
            continue;
        }
        let migration = inputs.migrations.get(covered).ok_or_else(|| {
            Diagnostic::new(
                "DB_HISTORY_MISSING",
                "Applied history exceeds the captured source prefix.",
            )
        })?;
        if entry.state == "imported" {
            if history.scope.is_none() || has_checkpoint || applied_started {
                return Err(Diagnostic::new("DB_HISTORY_DIVERGENT","Imported history must be a leading scoped prefix, never combined with a baseline checkpoint."));
            }
        } else if entry.state == "applied" {
            applied_started = true;
        } else if entry.state == "running"
            && history.scope.is_some()
            && inputs.config.engine == "postgresql"
            && super::analysis::concurrent_index(&migration.sql, &inputs.config.schema).is_some()
        {
            // Only a lone CREATE INDEX CONCURRENTLY commits an attempt first.
            attempt = Some(covered + 1);
        } else {
            return Err(Diagnostic::new(
                "DB_HISTORY_DIRTY",
                "Ledger contains non-applied work; explicit reconciliation is required.",
            ));
        }
        if entry.position != covered as i64 + 1
            || entry.id != migration.id.to_string()
            || entry.name != migration.name
        {
            return Err(Diagnostic::new("DB_HISTORY_DIVERGENT","Applied ledger is not the exact ordered prefix of this inventory (including inserted historical files)."));
        }
        if entry.checksum != migration.checksum {
            return Err(Diagnostic::new(
                "DB_HISTORY_EDITED",
                "Applied SQL or metadata bytes have changed.",
            ));
        }
        if attempt.is_none() {
            covered += 1;
        }
    }
    Ok((covered, attempt))
}
/// Report object-level differences, keeping the verdict unknown if either catalog is partial.
fn compare(observed: &Catalog, expected: &Catalog) -> Value {
    let keys: std::collections::BTreeSet<_> = observed
        .objects
        .keys()
        .chain(expected.objects.keys())
        .collect();
    let differences: Vec<_>=keys.into_iter().filter(|k| observed.objects.get(*k)!=expected.objects.get(*k)).map(|k| json!({
        "object":k,"change":match (expected.objects.contains_key(k),observed.objects.contains_key(k)) {(true,false)=>"missing",(false,true)=>"unexpected",_=>"changed"},
        "expected":expected.objects.get(k),"observed":observed.objects.get(k)})).collect();
    let outcome = if !observed.uncovered.is_empty() || !expected.uncovered.is_empty() {
        "unknown"
    } else if differences.is_empty() {
        "pass"
    } else {
        "fail"
    };
    json!({"outcome":outcome,"differences":differences,"expected_fingerprint":json_digest(expected),"observed_fingerprint":json_digest(observed)})
}

/// Capture target facts in a read-only snapshot and release that session before replay.
/// Scratch work uses separate identity/credentials; the result is schema evidence, not
/// data-preservation proof, executed SQL assertions, or permission to migrate.
pub(super) async fn inspect(
    inputs: CapturedInputs,
    command: &str,
    adopt_through: Option<u64>,
) -> Result<Value> {
    if let Some(id) = adopt_through {
        super::adoption::Adoption::selected(&inputs, id)?;
    }
    let secret = crate::stdlib::secrets::require_from_manifest(
        &inputs.endpoint.connection_secret,
        &inputs.document,
    )
    .map_err(|_| {
        Diagnostic::new(
            "DB_SECRET",
            "Cannot resolve declared target connection secret; provider details suppressed.",
        )
    })?;
    let target = Adapter::connect(
        &secret,
        &inputs.endpoint,
        &inputs.config.engine,
        Access::Inspect,
    )
    .await?;
    let observed = async {
        target.begin_readonly().await?;
        let identity = target.identity(&inputs.endpoint).await?;
        let catalog = target
            .catalog(&inputs.config.schema, &inputs.endpoint.role)
            .await?;
        // Retain a history error as a report diagnostic alongside the catalog.
        let history = target.history().await;
        Ok::<_, Diagnostic>((identity, catalog, history))
    }
    .await;
    let cleanup = target.rollback().await;
    // No target lock or idle transaction survives into scratch replay. Closing
    // now also avoids an idle-session disconnect erasing a completed report.
    drop(target);
    let (identity, catalog, history) = match (observed, cleanup) {
        (Ok(observed), Ok(())) => observed,
        (Err(d), _) | (_, Err(d)) => return Err(d),
    };
    inspect_observed(&inputs, command, identity, catalog, history, adopt_through).await
}
/// Combine captured target facts with applied-prefix and optional candidate replay.
/// Plan bindings retain logical labels, physical identity and exact source/policy digests.
/// Missing or incomplete evidence stays unknown; no target connection remains open here.
async fn inspect_observed(
    inputs: &CapturedInputs,
    command: &str,
    identity: Identity,
    catalog: Catalog,
    history: Result<History>,
    adopt_through: Option<u64>,
) -> Result<Value> {
    let inventory: Vec<_> = inputs
        .migrations
        .iter()
        .map(|m| {
            let mut v = m.summary();
            v["analysis"] = json!(analyze_for(
                &inputs.config.engine,
                &m.sql,
                &inputs.config.schema
            ));
            v
        })
        .collect();
    let mut report = json!({"format_version":1,"command":command,"observed_at":chrono::Utc::now().to_rfc3339(),
        "outcome":"unknown","database":inputs.database_name,"target":{"name":inputs.target_name,"schema":inputs.config.schema,"identity":identity},
        "source":{"input_digest":inputs.input_digest,"policy_digest":inputs.policy_digest,"migrations":inventory},
        "capabilities":identity.capabilities(),"catalog":catalog,
        "history":{"outcome":"unknown"},"drift":{"outcome":"unknown","differences":[]},
        "assertions":{"outcome":"not_run","executed":0,"reason":"Transition contracts are not executed by catalog replay or read-only inspection."},
        "diagnostics":[],"limitations":["Catalog equivalence is not data correctness or business compatibility.","No production row scans or lock-duration estimates.","No target DDL, adoption, authorization or repair.","Noncooperating concurrent DDL/writers are not excluded by this observation."]});
    let history = match history {
        Ok(h) => h,
        Err(d) => {
            diagnostic(&mut report, d);
            return Ok(report);
        }
    };
    let applied = match prefix(&history, inputs) {
        Ok(n) => n,
        Err(d) => {
            report["history"] = json!({"outcome":"fail","ledger_present":history.present,"observed_entry_count":history.entries.len(),"ledger_digest":json_digest(&history)});
            report["outcome"] = json!("fail");
            diagnostic(&mut report, d);
            return Ok(report);
        }
    };
    let adoption = adopt_through
        .map(|id| super::adoption::Adoption::request(inputs, id, &history))
        .transpose()?;
    let reference = adoption.as_ref().map_or(applied, |a| a.count);
    report["history"] =
        json!({"outcome":"pass","ledger":history,"pending_count":inputs.migrations.len()-applied});
    for (key, value) in history.counts().as_object().unwrap() {
        report["history"][key] = value.clone();
    }
    if let Some(action) = &adoption {
        report["adoption"] =
            json!({"action":action,"outcome":"not_run","migration_sql_executed":false});
    }
    if adoption.is_none()
        && !history.present
        && catalog
            .objects
            .iter()
            .any(|(k, v)| !k.starts_with("schema:") && v["external"] != true)
    {
        report["history"]["outcome"] = json!("fail");
        report["outcome"] = json!("fail");
        diagnostic(&mut report,Diagnostic::new("DB_UNRECOGNIZED_HISTORY","Nonempty managed scope has no recognized ledger. No baseline is inferred or adopted."));
        return Ok(report);
    }
    if !catalog.uncovered.is_empty() {
        diagnostic(&mut report,Diagnostic::new("DB_COVERAGE_INCOMPLETE","Observed objects or privileges exceed declared catalog coverage; equality cannot be asserted.").with_objects(&catalog.uncovered));
    }
    let mut expected = None;
    let mut candidate = None;
    let mut fingerprints = Vec::new();
    if let Some(scratch_config) = &inputs.config.scratch {
        let mut cleanup_failure = None;
        let replay = async {
            let secret = crate::stdlib::secrets::require_from_manifest(
                &scratch_config.endpoint.connection_secret,
                &inputs.document,
            )
            .map_err(|_| {
                Diagnostic::new(
                    "DB_SECRET",
                    "Cannot resolve separately declared scratch connection secret.",
                )
            })?;
            let scratch = Adapter::connect(
                &secret,
                &scratch_config.endpoint,
                &inputs.config.engine,
                Access::Scratch(json!(&identity)),
            )
            .await?;
            let work = async {
                let scratch_identity = scratch
                    .prepare_scratch(&identity, &scratch_config.endpoint, &inputs.config.schema)
                    .await?;
                report["scratch"] = json!({"identity":scratch_identity,"disposable":true});
                fingerprints.push(json_digest(
                    &scratch
                        .catalog(&inputs.config.schema, &scratch_config.endpoint.role)
                        .await?,
                ));
                if reference == 0 {
                    expected = Some(
                        scratch
                            .catalog(&inputs.config.schema, &scratch_config.endpoint.role)
                            .await?,
                    );
                }
                let count = if let Some(action) = &adoption {
                    action.count
                } else if command == "plan" {
                    inputs.migrations.len()
                } else {
                    applied
                };
                for (index, migration) in inputs.migrations.iter().take(count).enumerate() {
                    // Configured limits size work on the target's data. Replay
                    // runs on an empty database and keeps the default limit.
                    scratch
                        .execute_captured(
                            migration,
                            &inputs.config.schema,
                            super::inventory::Limits::default().statement(),
                        )
                        .await?;
                    let step = scratch
                        .catalog(&inputs.config.schema, &scratch_config.endpoint.role)
                        .await?;
                    if !step.uncovered.is_empty() {
                        // The target's catalog.uncovered cannot name these:
                        // they exist only in the replayed intermediate catalog.
                        let mut d = Diagnostic::new(
                            "DB_COVERAGE_INCOMPLETE",
                            "Intermediate catalog exceeds execution coverage.",
                        )
                        .with_objects(&step.uncovered);
                        d.source = Some(migration.name.clone());
                        return Err(d);
                    }
                    fingerprints.push(json_digest(&step));
                    if index + 1 == reference {
                        expected = Some(
                            scratch
                                .catalog(&inputs.config.schema, &scratch_config.endpoint.role)
                                .await?,
                        );
                    }
                }
                if command == "plan" {
                    candidate = Some(
                        scratch
                            .catalog(&inputs.config.schema, &scratch_config.endpoint.role)
                            .await?,
                    );
                }
                Ok::<_, Diagnostic>(())
            }
            .await;
            let cleanup = scratch.rollback().await;
            report["scratch"]["cleanup"] = json!(if cleanup.is_ok() { "pass" } else { "unknown" });
            if cleanup.is_err() {
                expected = None;
                candidate = None;
            }
            let (outcome, failed_cleanup) = replay_outcome(work, cleanup);
            cleanup_failure = failed_cleanup;
            outcome
        }
        .await;
        record_replay(&mut report, replay, cleanup_failure);
    } else {
        diagnostic(&mut report,Diagnostic::new("DB_REFERENCE_UNAVAILABLE","No explicitly disposable matching scratch database/file is configured; expected state is unverified."));
    }
    if let Some(expected) = &expected {
        report["drift"] = compare(&catalog, expected);
        if adoption.is_some() {
            report["drift"]["reference"] = json!("requested_baseline");
        }
    }
    report["outcome"] = report["drift"]["outcome"].clone();
    if report["drift"]["outcome"] == "fail" {
        diagnostic(&mut report,Diagnostic::new("DB_DRIFT","Observed managed catalog differs from the replayed applied prefix; inspect object-level differences."));
    }
    // Binding excludes observation time, display filters and scratch physical IDs.
    // It includes every authority-relevant source, policy, target and base fact.
    let binding = json!({"format_version":1,"adapter":adapter::catalog_format(&inputs.config.engine),"canonicalization_version":1,
        "input_digest":inputs.input_digest,"policy_digest":inputs.policy_digest,"database":inputs.database_name,"target_name":inputs.target_name,"target":identity,
        "managed_schema":inputs.config.schema,"base_history_digest":json_digest(&history),
        "base_catalog_digest":json_digest(&catalog),"applied_count":applied,
        "expected_base_digest":expected.as_ref().map(json_digest)});
    let pending: Vec<_> = inputs
        .migrations
        .iter()
        .skip(if adoption.is_some() {
            inputs.migrations.len()
        } else {
            applied
        })
        .map(|m| {
            let mut v = m.summary();
            v["analysis"] = json!(analyze_for(
                &inputs.config.engine,
                &m.sql,
                &inputs.config.schema
            ));
            v
        })
        .collect();
    let candidate_outcome = if command != "plan" {
        "not_run"
    } else if candidate.as_ref().is_some_and(|c| c.uncovered.is_empty()) {
        "pass"
    } else {
        "unknown"
    };
    report["binding"] = binding.clone();
    if command == "plan" {
        let mut execution = json!({"format":if adoption.is_some() {super::adoption::format(&inputs.config.engine)} else {adapter::execution_format(&inputs.config.engine)},"prefix_fingerprints":fingerprints});
        // The limits migrate will use are part of what is reviewed and approved.
        if let Some(limits) = inputs.plan_limits() {
            execution["limits"] = limits;
        }
        let mut plan = json!({"execution":execution,"binding":binding,"pending":pending,"candidate":{"outcome":candidate_outcome,"fingerprint":candidate.as_ref().map(json_digest)},
            "authorized":false,"executable":false,"automatic_safe":false,"classification":if report["outcome"]=="pass" && candidate_outcome=="pass" {"requires_review"} else {"blocked"},"required_authority":"migrate requires this exact digest from the separately protected approval_secret provider, then revalidates under ownership. This digest alone grants no authority.",
            "data_checks":{"outcome":"not_run","reason":"Constraints, casts, nulls, duplicates, orphans and deployed-client compatibility require later data/behavior evidence."}});
        if let Some(action) = adoption {
            plan["action"] = json!(action);
        }
        report["plan_digest"] = json!(json_digest(&plan));
        report["plan"] = plan;
        if candidate_outcome != "pass" && report["outcome"] == "pass" {
            report["outcome"] = json!("unknown");
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn failure(code: &str) -> Diagnostic {
        Diagnostic::new(code, "test")
    }

    #[test]
    fn a_failed_cleanup_never_replaces_the_replay_failure() {
        let (outcome, cleanup) =
            replay_outcome(Err(failure("DB_TIMEOUT")), Err(failure("DB_CANCELLED")));
        assert_eq!(outcome.unwrap_err().code, "DB_TIMEOUT");
        assert_eq!(cleanup.unwrap().code, "DB_CANCELLED");
        let (outcome, cleanup) = replay_outcome(Ok(()), Err(failure("DB_SQLITE")));
        assert_eq!(outcome.unwrap_err().code, "DB_SQLITE");
        assert!(cleanup.is_none());
        let (outcome, cleanup) = replay_outcome(Err(failure("DB_SCRATCH_MISMATCH")), Ok(()));
        assert_eq!(outcome.unwrap_err().code, "DB_SCRATCH_MISMATCH");
        assert!(cleanup.is_none());
        let (outcome, cleanup) = replay_outcome(Ok(()), Ok(()));
        assert!(outcome.is_ok() && cleanup.is_none());
    }

    #[test]
    fn a_failed_cleanup_is_reported_after_the_replay_failure() {
        let codes = |report: &Value| -> Vec<String> {
            report["diagnostics"]
                .as_array()
                .unwrap()
                .iter()
                .map(|d| d["code"].as_str().unwrap().to_owned())
                .collect()
        };
        let mut report = json!({ "diagnostics": [] });
        let (outcome, cleanup) =
            replay_outcome(Err(failure("DB_TIMEOUT")), Err(failure("DB_CANCELLED")));
        record_replay(&mut report, outcome, cleanup);
        assert_eq!(codes(&report), ["DB_TIMEOUT", "DB_CANCELLED"]);
        let mut report = json!({ "diagnostics": [] });
        let (outcome, cleanup) = replay_outcome(Ok(()), Ok(()));
        record_replay(&mut report, outcome, cleanup);
        assert!(codes(&report).is_empty());
    }
}
