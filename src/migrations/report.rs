//! Shared report construction: authored history, observed target, expected prefix.
use super::analysis::analyze;
use super::inventory::{json_digest, CapturedInputs};
use super::postgres::{Catalog, History, Postgres, RelationalAdapter};
use super::{Diagnostic, Result};
use serde_json::{json, Value};

fn diagnostic(report: &mut Value, d: Diagnostic) {
    report["diagnostics"].as_array_mut().unwrap().push(json!(d));
}
pub(super) fn failure(command: &str, d: Diagnostic) -> Value {
    json!({"format_version":1,"command":command,"observed_at":chrono::Utc::now().to_rfc3339(),
        "outcome":"unknown","diagnostics":[d],"assertions":{"outcome":"not_run","executed":0}})
}
fn prefix(history: &History, inputs: &CapturedInputs) -> Result<usize> {
    if history.entries.len() > inputs.migrations.len() {
        return Err(Diagnostic::new(
            "DB_HISTORY_MISSING",
            "Applied history contains migrations missing from the captured inventory.",
        ));
    }
    for (i, (entry, migration)) in history.entries.iter().zip(&inputs.migrations).enumerate() {
        if entry.state != "applied" {
            return Err(Diagnostic::new(
                "DB_HISTORY_DIRTY",
                "Ledger contains non-applied work; explicit reconciliation is required.",
            ));
        }
        if entry.position != i as i64 + 1
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
    }
    Ok(history.entries.len())
}
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

pub(super) async fn inspect(inputs: CapturedInputs, command: &str) -> Result<Value> {
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
    let target = Postgres::connect(&secret, &inputs.endpoint).await?;
    let result = inspect_target(&target, &inputs, command).await;
    let cleanup = target.rollback().await;
    match (result, cleanup) {
        (Ok(report), Ok(())) => Ok(report),
        (Err(d), _) => Err(d),
        (_, Err(d)) => Err(d),
    }
}
async fn inspect_target(
    target: &Postgres,
    inputs: &CapturedInputs,
    command: &str,
) -> Result<Value> {
    target.begin_readonly().await?;
    let identity = target.identity(&inputs.endpoint).await?;
    let catalog = target
        .catalog(&inputs.config.schema, &inputs.endpoint.role)
        .await?;
    let inventory: Vec<_> = inputs
        .migrations
        .iter()
        .map(|m| {
            let mut v = m.summary();
            v["analysis"] = json!(analyze(&m.sql, &inputs.config.schema));
            v
        })
        .collect();
    let mut report = json!({"format_version":1,"command":command,"observed_at":chrono::Utc::now().to_rfc3339(),
        "outcome":"unknown","target":{"name":inputs.target_name,"schema":inputs.config.schema,"identity":identity},
        "source":{"input_digest":inputs.input_digest,"policy_digest":inputs.policy_digest,"migrations":inventory},
        "capabilities":Postgres::capabilities(identity.version),"catalog":catalog,
        "history":{"outcome":"unknown"},"drift":{"outcome":"unknown","differences":[]},
        "assertions":{"outcome":"not_run","executed":0,"reason":"SQL contracts and data/business invariants are not executed in PR1."},
        "diagnostics":[],"limitations":["Catalog equivalence is not data correctness or business compatibility.","No production row scans or lock-duration estimates.","No target DDL, adoption, authorization or repair.","Noncooperating concurrent DDL/writers are not excluded by this observation."]});
    let history = match target.history().await {
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
    report["history"] = json!({"outcome":"pass","ledger":history,"applied_count":applied,"pending_count":inputs.migrations.len()-applied});
    if !history.present
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
        diagnostic(&mut report,Diagnostic::new("DB_COVERAGE_INCOMPLETE","Observed objects or privileges exceed declared catalog coverage; equality cannot be asserted."));
    }
    let mut expected = None;
    let mut candidate = None;
    if let Some(scratch_config) = &inputs.config.scratch {
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
            let scratch = Postgres::connect(&secret, &scratch_config.endpoint).await?;
            let work = async {
                let scratch_identity = scratch
                    .prepare_scratch(&identity, &scratch_config.endpoint, &inputs.config.schema)
                    .await?;
                report["scratch"] = json!({"identity":scratch_identity,"disposable":true});
                if applied == 0 {
                    expected = Some(
                        scratch
                            .catalog(&inputs.config.schema, &scratch_config.endpoint.role)
                            .await?,
                    );
                }
                let count = if command == "plan" {
                    inputs.migrations.len()
                } else {
                    applied
                };
                for (index, migration) in inputs.migrations.iter().take(count).enumerate() {
                    scratch
                        .execute_captured(migration, &inputs.config.schema)
                        .await?;
                    if index + 1 == applied {
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
            match (work, cleanup) {
                (Ok(()), Ok(())) => Ok(()),
                (Err(e), Ok(())) => Err(e),
                (_, Err(e)) => {
                    expected = None;
                    candidate = None;
                    Err(e)
                }
            }
        }
        .await;
        if let Err(d) = replay {
            diagnostic(&mut report, d);
        }
    } else {
        diagnostic(&mut report,Diagnostic::new("DB_REFERENCE_UNAVAILABLE","No explicitly disposable matching scratch database is configured; expected state is unverified."));
    }
    if let Some(expected) = &expected {
        report["drift"] = compare(&catalog, expected);
    }
    report["outcome"] = report["drift"]["outcome"].clone();
    if report["drift"]["outcome"] == "fail" {
        diagnostic(&mut report,Diagnostic::new("DB_DRIFT","Observed managed catalog differs from the replayed applied prefix; inspect object-level differences."));
    }
    // Binding excludes observation time, display filters and scratch physical IDs.
    // It includes every authority-relevant source, policy, target and base fact.
    let binding = json!({"format_version":1,"adapter":"postgresql-catalog-v1","canonicalization_version":1,
        "input_digest":inputs.input_digest,"policy_digest":inputs.policy_digest,"target":identity,
        "managed_schema":inputs.config.schema,"base_history_digest":json_digest(&history),
        "base_catalog_digest":json_digest(&catalog),"applied_count":applied,
        "expected_base_digest":expected.as_ref().map(json_digest)});
    let pending: Vec<_> = inputs
        .migrations
        .iter()
        .skip(applied)
        .map(|m| {
            let mut v = m.summary();
            v["analysis"] = json!(analyze(&m.sql, &inputs.config.schema));
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
        let plan = json!({"binding":binding,"pending":pending,"candidate":{"outcome":candidate_outcome,"fingerprint":candidate.as_ref().map(json_digest)},
            "authorized":false,"executable":false,"automatic_safe":false,"classification":if report["outcome"]=="pass" && candidate_outcome=="pass" {"requires_review"} else {"blocked"},"required_authority":"A later executor must revalidate under its migration lock and obtain exact-target/exact-plan approval; this digest grants no authority.",
            "data_checks":{"outcome":"not_run","reason":"Constraints, casts, nulls, duplicates, orphans and deployed-client compatibility require later data/behavior evidence."}});
        report["plan_digest"] = json!(json_digest(&plan));
        report["plan"] = plan;
        if candidate_outcome != "pass" && report["outcome"] == "pass" {
            report["outcome"] = json!("unknown");
        }
    }
    Ok(report)
}
