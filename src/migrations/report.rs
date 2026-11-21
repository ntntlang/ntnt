//! Shared report construction: authored history, observed target, expected prefix.
use super::adapter::{self, Access, Adapter, Identity};
use super::analysis::analyze_for;
use super::inventory::{json_digest, CapturedInputs, CatalogDigests};
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

/// What replaying a prefix of the history in scratch established. A replay
/// that fails keeps what it established before the failure.
#[derive(Default)]
struct Replayed {
    /// Fingerprint of the catalog before the first and after each replayed
    /// migration; only plans record them.
    fingerprints: Vec<String>,
    /// The catalog after `reference` migrations.
    expected: Option<Catalog>,
    /// For plans, the catalog after the last replayed migration.
    candidate: Option<Catalog>,
}

/// Replay the first `count` migrations on a prepared scratch, inspecting the
/// catalog before the first and after each one: an object outside coverage
/// that exists only between two migrations is reported against the migration
/// that made it. Each state is inspected once and fingerprinted only for a
/// plan; the catalog itself is kept only at `reference` and, for a plan, at
/// the end.
async fn replay_prefix(
    scratch: &Adapter,
    inputs: &CapturedInputs,
    role: &str,
    (count, reference, plan): (usize, usize, bool),
    replayed: &mut Replayed,
) -> Result<()> {
    let (schema, extra) = (&inputs.config.schema, &inputs.history_schemas);
    let mut digests = CatalogDigests::default();
    for index in 0..=count {
        let migration = index.checked_sub(1).map(|i| &inputs.migrations[i]);
        if let Some(migration) = migration {
            // Configured limits size work on the target's data. Replay
            // runs on an empty database and keeps the default limit. A
            // PostgreSQL scratch also refuses a migration that acts outside
            // the managed scope here (DB_OUTSIDE_SCOPE).
            scratch
                .execute_captured(
                    migration,
                    schema,
                    extra,
                    super::inventory::Limits::default().statement(),
                )
                .await?;
        }
        let keep = index == reference || (plan && index == count);
        let (uncovered, catalog, fingerprint) = if keep {
            let catalog = scratch.catalog(schema, extra, role).await?;
            let fingerprint = plan.then(|| digests.digest(&catalog));
            (catalog.uncovered.clone(), Some(catalog), fingerprint)
        } else if plan {
            let (uncovered, fingerprint) = scratch
                .fingerprint(schema, extra, role, &mut digests)
                .await?;
            (uncovered, None, Some(fingerprint))
        } else {
            (scratch.coverage(schema, extra, role).await?, None, None)
        };
        if let (Some(migration), false) = (migration, uncovered.is_empty()) {
            // The target's catalog.uncovered cannot name these:
            // they exist only in the replayed intermediate catalog.
            let mut d = Diagnostic::new(
                "DB_COVERAGE_INCOMPLETE",
                "Intermediate catalog exceeds execution coverage.",
            )
            .with_objects(&uncovered);
            d.source = Some(migration.name.clone());
            return Err(d);
        }
        replayed.fingerprints.extend(fingerprint);
        if index == reference {
            replayed.expected = catalog.clone();
        }
        if plan && index == count {
            replayed.candidate = catalog;
        }
    }
    Ok(())
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
            .catalog(
                &inputs.config.schema,
                &inputs.history_schemas,
                &inputs.endpoint.role,
            )
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
    if !inputs.history_schemas.is_empty() {
        report["target"]["history_schemas"] = json!(inputs.history_schemas);
    }
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
    let mut replayed = Replayed::default();
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
                    .prepare_scratch(
                        &identity,
                        &scratch_config.endpoint,
                        &inputs.config.schema,
                        &inputs.history_schemas,
                    )
                    .await?;
                report["scratch"] = json!({"identity":scratch_identity,"disposable":true});
                let count = if let Some(action) = &adoption {
                    action.count
                } else if command == "plan" {
                    inputs.migrations.len()
                } else {
                    applied
                };
                replay_prefix(
                    &scratch,
                    inputs,
                    &scratch_config.endpoint.role,
                    (count, reference, command == "plan"),
                    &mut replayed,
                )
                .await
            }
            .await;
            let cleanup = scratch.rollback().await;
            report["scratch"]["cleanup"] = json!(if cleanup.is_ok() { "pass" } else { "unknown" });
            if cleanup.is_err() {
                replayed.expected = None;
                replayed.candidate = None;
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
    let Replayed {
        fingerprints,
        expected,
        candidate,
    } = replayed;
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

    /// A SQLite project whose migrations are `scripts`, in order, with empty
    /// target and scratch files.
    struct Project {
        _dir: tempfile::TempDir,
        inputs: CapturedInputs,
        target: std::path::PathBuf,
        scratch: std::path::PathBuf,
    }
    fn sqlite_project(scripts: &[String]) -> Project {
        let scripts: Vec<_> = scripts.iter().map(|sql| (sql.clone(), None)).collect();
        sqlite_project_with(&scripts)
    }
    /// The same, where a migration may name the table it rebuilds
    /// (`sqlite_rebuild`).
    fn sqlite_project_with(scripts: &[(String, Option<&str>)]) -> Project {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::create_dir(root.join("migrations")).unwrap();
        std::fs::write(
            root.join("ntnt.toml"),
            "[databases.app]\nengine = \"sqlite\"\nschema = \"main\"\nmigrations_dir = \"migrations\"\n\
             [databases.app.targets.test]\nconnection_secret = \"DB_TARGET\"\n\
             [databases.app.scratch]\nconnection_secret = \"DB_SCRATCH\"\ndisposable = true\n\
             [secrets.DB_TARGET]\n[secrets.DB_SCRATCH]\n",
        )
        .unwrap();
        for (i, (sql, rebuild)) in scripts.iter().enumerate() {
            let migrations = root.join("migrations");
            std::fs::write(migrations.join(format!("{:04}_step.sql", i + 1)), sql).unwrap();
            if let Some(table) = rebuild {
                let sidecar = format!("sqlite_rebuild = \"{table}\"\n");
                std::fs::write(migrations.join(format!("{:04}_step.toml", i + 1)), sidecar)
                    .unwrap();
            }
        }
        let inputs = CapturedInputs::load(&root.join("ntnt.toml"), "test", None).unwrap();
        let (target, scratch) = (root.join("target.sqlite"), root.join("scratch.sqlite"));
        for path in [&target, &scratch] {
            std::fs::File::create(path).unwrap();
        }
        Project {
            _dir: dir,
            inputs,
            target,
            scratch,
        }
    }
    /// The target's identity and a scratch session prepared for replay, as
    /// plan has them.
    async fn prepared_scratch(project: &Project) -> (Identity, Adapter) {
        use crate::interpreter::SecretValue;
        let secret = |path: &std::path::Path| {
            SecretValue::new("FIXTURE", path.to_str().unwrap().to_owned()).unwrap()
        };
        let endpoint = &project.inputs.endpoint;
        let target = Adapter::connect(
            &secret(&project.target),
            endpoint,
            "sqlite",
            Access::Inspect,
        )
        .await
        .unwrap();
        let identity = target.identity(endpoint).await.unwrap();
        drop(target);
        let scratch_endpoint = &project.inputs.config.scratch.as_ref().unwrap().endpoint;
        let scratch = Adapter::connect(
            &secret(&project.scratch),
            scratch_endpoint,
            "sqlite",
            Access::Scratch(json!(&identity)),
        )
        .await
        .unwrap();
        scratch
            .prepare_scratch(&identity, scratch_endpoint, "main", &[])
            .await
            .unwrap();
        (identity, scratch)
    }
    fn current_thread() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    }

    /// Replay `n` migrations, `migration(i)` the SQL of the i-th, on a fresh
    /// SQLite scratch as a plan does and return the inspection work its
    /// session did.
    fn replay_work(
        n: usize,
        migration: impl Fn(usize) -> String,
    ) -> super::super::sqlite_cache::Stats {
        let project = sqlite_project(&(1..=n).map(migration).collect::<Vec<_>>());
        let inputs = &project.inputs;
        current_thread().block_on(async {
            let (identity, scratch) = prepared_scratch(&project).await;
            let mut replayed = Replayed::default();
            replay_prefix(&scratch, inputs, "", (n, n, true), &mut replayed)
                .await
                .unwrap();
            assert_eq!(replayed.fingerprints.len(), n + 1);
            assert!(replayed.expected.is_some() && replayed.candidate.is_some());
            let Adapter::Sqlite(db) = &scratch else {
                unreachable!("a SQLite scratch")
            };
            let work = db.stats().await.unwrap();
            scratch.rollback().await.unwrap();
            // status and verify replay the same states without fingerprinting them.
            let scratch_endpoint = &inputs.config.scratch.as_ref().unwrap().endpoint;
            scratch
                .prepare_scratch(&identity, scratch_endpoint, "main", &[])
                .await
                .unwrap();
            let mut replayed = Replayed::default();
            replay_prefix(&scratch, inputs, "", (n, n, false), &mut replayed)
                .await
                .unwrap();
            assert!(replayed.fingerprints.is_empty() && replayed.candidate.is_none());
            assert!(replayed.expected.is_some());
            scratch.rollback().await.unwrap();
            work
        })
    }

    /// #421: replaying n migrations inspects n + 1 states, and reads each
    /// object's metadata and compiles its programs once, when the migration
    /// that creates it runs, not again after every later migration. The work
    /// is exact for n and 2n; it used to grow with the square of n.
    #[test]
    fn replay_inspection_work_grows_linearly_with_history() {
        const N: u64 = 8;
        let tables = |i: usize| format!("CREATE TABLE t{i}(id INTEGER PRIMARY KEY);");
        // An index and a view per table as well.
        let views = |i: usize| {
            format!("CREATE TABLE t{i}(id INTEGER PRIMARY KEY, v TEXT); CREATE INDEX t{i}_v ON t{i}(v); CREATE VIEW v{i} AS SELECT id, v FROM t{i};")
        };
        for n in [N, 2 * N] {
            let work = replay_work(n as usize, tables);
            // The state before the first migration and after each one.
            assert_eq!(work.catalogs, n + 1, "{work:?}");
            // A table's columns and foreign keys.
            assert_eq!(work.metadata_queries, 2 * n, "{work:?}");
            // INSERT, UPDATE, DELETE and the three rowid UPDATEs of a table.
            assert_eq!(work.programs_compiled, 6 * n, "{work:?}");
            // One allowlist for each inspection that compiles anything.
            assert_eq!(work.allowlists, n, "{work:?}");
            let work = replay_work(n as usize, views);
            assert_eq!(work.catalogs, n + 1, "{work:?}");
            // The index's columns and properties too.
            assert_eq!(work.metadata_queries, 4 * n, "{work:?}");
            // The view's SELECT too.
            assert_eq!(work.programs_compiled, 7 * n, "{work:?}");
            assert_eq!(work.allowlists, n, "{work:?}");
        }
    }

    /// A small deterministic pseudo-random generator (xorshift64*), so the
    /// generated history is the same on every run.
    struct Rng(u64);
    impl Rng {
        fn below(&mut self, n: usize) -> usize {
            self.0 ^= self.0 >> 12;
            self.0 ^= self.0 << 25;
            self.0 ^= self.0 >> 27;
            (self.0.wrapping_mul(0x2545_f491_4f6c_dd1d) >> 33) as usize % n
        }
        fn pick<'a>(&mut self, names: &[&'a str]) -> &'a str {
            names[self.below(names.len())]
        }
        /// A name as a later statement may spell it: SQLite compares names
        /// ASCII case-insensitively.
        fn spell(&mut self, name: &str) -> String {
            match self.below(3) {
                0 => name.to_ascii_uppercase(),
                _ => name.to_owned(),
            }
        }
    }

    /// `steps` migrations over a few names that collide across kinds:
    /// tables, views, indexes and triggers created, changed, renamed and
    /// dropped in any order, triggers that write other tables, foreign keys
    /// with and without actions, and now and then a trigger that writes the
    /// ledger. Each migration is one an in-memory SQLite database with
    /// foreign keys on accepted after the ones before it (its foreign-key
    /// check included), so most of them also run in replay; what they leave
    /// behind (a view of a dropped table, a trigger writing a renamed one)
    /// is often outside coverage.
    fn generated_history(steps: usize) -> Vec<String> {
        const TABLES: &[&str] = &["a", "b", "c", "Dd"];
        const VIEWS: &[&str] = &["va", "vb", "a"];
        const TRIGGERS: &[&str] = &["a", "b", "ta", "tb", "va"];
        const INDEXES: &[&str] = &["ia", "ib", "b"];
        let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
        let db = rusqlite::Connection::open_in_memory().unwrap();
        db.execute_batch("PRAGMA foreign_keys=ON; PRAGMA recursive_triggers=ON;")
            .unwrap();
        let accepted = |db: &rusqlite::Connection, script: &str| -> bool {
            let check = || -> rusqlite::Result<bool> {
                db.execute_batch(script)?;
                let mut statement = db.prepare("PRAGMA foreign_key_check")?;
                let clean = statement.query([])?.next()?.is_none();
                Ok(clean)
            };
            db.execute_batch("SAVEPOINT candidate").unwrap();
            let ok = check().unwrap_or(false);
            let end = if ok {
                "RELEASE candidate"
            } else {
                "ROLLBACK TO candidate; RELEASE candidate"
            };
            db.execute_batch(end).unwrap();
            ok
        };
        let mut history = Vec::new();
        while history.len() < steps {
            let mut script = String::new();
            for _ in 0..=rng.below(2) {
                let t = rng.pick(TABLES);
                let other = rng.pick(TABLES);
                let other = rng.spell(other);
                let v = rng.pick(VIEWS);
                let statement = match rng.below(19) {
                    0..=4 => {
                        let fk = match rng.below(4) {
                            0 => format!(", r INTEGER REFERENCES {other}(id) ON DELETE CASCADE"),
                            1 => format!(", r INTEGER REFERENCES {other}(id) ON DELETE SET NULL"),
                            2 => format!(", r INTEGER REFERENCES {other}"),
                            _ => String::new(),
                        };
                        format!("CREATE TABLE {t}(id INTEGER PRIMARY KEY, x INTEGER, y TEXT{fk})")
                    }
                    5 => format!("DROP TABLE {t}"),
                    6 => format!("ALTER TABLE {t} ADD COLUMN z{} INTEGER", rng.below(3)),
                    7 => format!("ALTER TABLE {t} RENAME TO {}", rng.pick(TABLES)),
                    8 => match rng.below(3) {
                        0 => format!("ALTER TABLE {t} RENAME COLUMN x TO w"),
                        1 => format!("ALTER TABLE {t} RENAME COLUMN w TO x"),
                        _ => format!("ALTER TABLE {t} DROP COLUMN y"),
                    },
                    9 | 10 => format!(
                        "CREATE {}INDEX {} ON {t}(x){}",
                        ["", "UNIQUE "][rng.below(2)],
                        rng.pick(INDEXES),
                        ["", " WHERE x > 0"][rng.below(2)]
                    ),
                    11 => format!("DROP INDEX {}", rng.pick(INDEXES)),
                    12 | 13 => match rng.below(3) {
                        0 => format!("CREATE VIEW {v} AS SELECT id, x FROM {other}"),
                        1 => format!("CREATE VIEW {v} AS SELECT * FROM {}", rng.pick(VIEWS)),
                        _ => format!(
                            "CREATE VIEW {v} AS SELECT {t}.id FROM {t} JOIN {other} USING (id)"
                        ),
                    },
                    14 => format!("DROP VIEW {v}"),
                    15..=17 => {
                        let body = match rng.below(12) {
                            0 | 1 => format!("INSERT INTO {other}(x) VALUES (1)"),
                            2 | 3 => format!("DELETE FROM {other} WHERE id = 0"),
                            4 | 5 => format!("UPDATE {other} SET x = x WHERE id = 0"),
                            6 => format!("SELECT x FROM {}", rng.pick(VIEWS)),
                            7 => format!("INSERT INTO {other}(w) VALUES (1)"),
                            8 => "DELETE FROM _ntnt_schema_migrations_v2".to_owned(),
                            _ => "SELECT abs(1)".to_owned(),
                        };
                        let event = ["INSERT", "DELETE", "UPDATE OF x"][rng.below(3)];
                        format!(
                            "CREATE TRIGGER {} AFTER {event} ON {t} BEGIN {body}; END",
                            rng.pick(TRIGGERS)
                        )
                    }
                    _ => format!("DROP TRIGGER {}", rng.pick(TRIGGERS)),
                };
                script.push_str(&statement);
                script.push_str(";\n");
            }
            if accepted(&db, &script) {
                history.push(script);
            }
        }
        history
    }

    /// Run `scripts` one at a time on a SQLite scratch, as replay does,
    /// rolling the scratch back to empty once after `restart` of them, and
    /// after each one require the session's inspection to report exactly
    /// what an inspection that reuses nothing reports: the same catalog, the
    /// same coverage and json_digest of that catalog as the fingerprint, or
    /// the same error. The first inspection after a migration goes through
    /// `catalog`, `fingerprint` and `coverage` in turn. A failed migration's
    /// state is inspected too. Returns the migrations that failed and how
    /// many states were outside coverage.
    fn assert_incremental_inspection_is_exact(
        scripts: &[(String, Option<&str>)],
        restart: usize,
    ) -> (Vec<String>, usize) {
        let project = sqlite_project_with(scripts);
        let inputs = &project.inputs;
        current_thread().block_on(async {
            let (identity, scratch) = prepared_scratch(&project).await;
            let Adapter::Sqlite(db) = &scratch else {
                unreachable!("a SQLite scratch")
            };
            let (mut failed, mut uncovered_states) = (Vec::new(), 0);
            for (index, migration) in inputs.migrations.iter().enumerate() {
                if index == restart {
                    // A rolled-back replay leaves the session's cache behind.
                    scratch.rollback().await.unwrap();
                    let scratch_endpoint = &inputs.config.scratch.as_ref().unwrap().endpoint;
                    scratch
                        .prepare_scratch(&identity, scratch_endpoint, "main", &[])
                        .await
                        .unwrap();
                }
                let outcome = scratch
                    .execute_captured(
                        migration,
                        "main",
                        &[],
                        super::super::inventory::Limits::default().statement(),
                    )
                    .await;
                if let Err(d) = &outcome {
                    failed.push(format!("{} {}: {}", migration.name, d.code, d.message));
                }
                let fresh = db.fresh_catalog().await;
                let at = format!("after {} ({outcome:?})", migration.name);
                let expect = |fresh: &Result<Catalog>| match fresh {
                    Ok(catalog) => Ok((
                        json!(catalog),
                        catalog.uncovered.clone(),
                        json_digest(catalog),
                    )),
                    Err(d) => Err((d.code.clone(), d.message.clone())),
                };
                let error = |d: Diagnostic| (d.code, d.message);
                for call in 0..3 {
                    let reported = match (index + call) % 3 {
                        0 => scratch.catalog("main", &[], "").await.map(|catalog| {
                            let digest = json_digest(&catalog);
                            (json!(catalog), catalog.uncovered.clone(), digest)
                        }),
                        1 => scratch
                            .fingerprint("main", &[], "", &mut CatalogDigests::default())
                            .await
                            .map(|(uncovered, fingerprint)| {
                                let catalog = fresh.as_ref().map_or(Value::Null, |c| json!(c));
                                (catalog, uncovered, fingerprint)
                            }),
                        _ => scratch.coverage("main", &[], "").await.map(|uncovered| {
                            let catalog = fresh.as_ref().map_or(Value::Null, |c| json!(c));
                            let digest = fresh.as_ref().map_or(String::new(), json_digest);
                            (catalog, uncovered, digest)
                        }),
                    };
                    assert_eq!(reported.map_err(error), expect(&fresh), "{at}, call {call}");
                }
                if fresh.is_ok_and(|catalog| !catalog.uncovered.is_empty()) {
                    uncovered_states += 1;
                }
            }
            scratch.rollback().await.unwrap();
            (failed, uncovered_states)
        })
    }

    /// #421: what a session reuses from its earlier inspections never
    /// changes what an inspection reports. Hand-written histories aim at each
    /// dependency a compiled program has (trigger bodies, views, foreign keys
    /// in both directions, names shared across kinds, definitions changed
    /// under the same name, metadata over its bounds, a table rebuild); a
    /// generated history covers their combinations.
    #[test]
    fn incremental_inspection_reports_what_a_fresh_inspection_does() {
        let ledger = "_ntnt_schema_migrations_v2";
        let long = "x".repeat(300);
        let written: Vec<(String, Option<&str>)> = [
            "CREATE TABLE t(id INTEGER PRIMARY KEY, v TEXT); CREATE TABLE audit(id INTEGER, at TEXT);".to_owned(),
            "CREATE TRIGGER t_audit AFTER UPDATE OF v ON t BEGIN INSERT INTO audit(id, at) VALUES(new.id, 'x'); END;".to_owned(),
            "DROP TABLE audit; CREATE TABLE audit(id INTEGER);".to_owned(),
            "ALTER TABLE audit ADD COLUMN at TEXT;".to_owned(),
            "CREATE TABLE u(y INTEGER); CREATE TABLE t2(x INTEGER); CREATE TRIGGER t2 AFTER INSERT ON u BEGIN SELECT 1; END; CREATE VIEW v2 AS SELECT x FROM t2;".to_owned(),
            "DROP TABLE t2;".to_owned(),
            "CREATE TABLE t2(x INTEGER);".to_owned(),
            "CREATE TABLE p(id INTEGER PRIMARY KEY); CREATE TABLE q(id INTEGER); CREATE TRIGGER p AFTER INSERT ON q BEGIN SELECT 1; END;".to_owned(),
            "CREATE TABLE c(id INTEGER PRIMARY KEY, p_id INTEGER REFERENCES p(id) ON DELETE CASCADE);".to_owned(),
            format!("CREATE TRIGGER c_gone AFTER DELETE ON c BEGIN DELETE FROM {ledger}; END;"),
            "DROP TRIGGER c_gone;".to_owned(),
            "CREATE TABLE m(id INTEGER PRIMARY KEY, k INTEGER REFERENCES n(k));".to_owned(),
            "CREATE TABLE n(k INTEGER);".to_owned(),
            "CREATE UNIQUE INDEX n_k ON n(k);".to_owned(),
            "DROP INDEX n_k;".to_owned(),
            "CREATE UNIQUE INDEX n_k ON n(k); CREATE VIEW w1 AS SELECT id FROM t; CREATE VIEW w2 AS SELECT id FROM w1;".to_owned(),
            "DROP VIEW w1;".to_owned(),
            "CREATE VIEW w1 AS SELECT id, v FROM T;".to_owned(),
            "CREATE TABLE mid(id INTEGER); CREATE TABLE sink(id INTEGER); CREATE TRIGGER mid_sink AFTER INSERT ON mid BEGIN INSERT INTO sink VALUES(new.id); END; CREATE TRIGGER t_mid AFTER INSERT ON t BEGIN INSERT INTO mid VALUES(new.id); END;".to_owned(),
            "DROP TABLE sink;".to_owned(),
            "CREATE TABLE sink(id INTEGER);".to_owned(),
            "ALTER TABLE sink RENAME TO sink3;".to_owned(),
            format!("CREATE TABLE wide(\"{long}\" INTEGER);"),
            "DROP TABLE wide;".to_owned(),
            "CREATE TABLE kv(k TEXT PRIMARY KEY, v TEXT) WITHOUT ROWID; CREATE TABLE s(id INTEGER PRIMARY KEY, n INTEGER) STRICT; CREATE TRIGGER kv_touch AFTER UPDATE OF v ON kv BEGIN UPDATE s SET n = n + 1 WHERE id = 1; END;".to_owned(),
            "CREATE TABLE g(a INTEGER, b INTEGER GENERATED ALWAYS AS (a + 1));".to_owned(),
            "DROP TABLE g;".to_owned(),
            "CREATE TABLE accounts(id INTEGER PRIMARY KEY, email TEXT); CREATE TABLE children(id INTEGER PRIMARY KEY, account_id INTEGER REFERENCES accounts(id) ON DELETE CASCADE); CREATE INDEX email_idx ON accounts(email); CREATE TRIGGER email_changed AFTER UPDATE OF email ON accounts BEGIN INSERT INTO audit(id) VALUES(new.id); END;".to_owned(),
            "CREATE TABLE replacement(id INTEGER PRIMARY KEY, email TEXT NOT NULL); INSERT INTO replacement SELECT id, lower(email) FROM accounts; DROP TABLE accounts; ALTER TABLE replacement RENAME TO accounts; CREATE INDEX email_idx ON accounts(email); CREATE TRIGGER email_changed AFTER UPDATE OF email ON accounts BEGIN INSERT INTO audit(id) VALUES(new.id); END;".to_owned(),
            "CREATE TABLE Foo(x INTEGER); CREATE TRIGGER foo_t AFTER INSERT ON foo BEGIN SELECT 1; END;".to_owned(),
            "DROP TRIGGER foo_t;".to_owned(),
        ]
        .into_iter()
        .enumerate()
        .map(|(i, sql)| (sql, (i == 28).then_some("accounts")))
        .collect();
        let (failed, uncovered) = assert_incremental_inspection_is_exact(&written, 29);
        // Only the two that leave m's foreign key without a unique parent key
        // fail, at the foreign-key check, and several states are outside
        // coverage.
        let mismatch = "DB_SQLITE: SQLite operation failed (code 1). Details suppressed.";
        assert_eq!(
            failed,
            [
                format!("0013_step.sql {mismatch}"),
                format!("0015_step.sql {mismatch}")
            ]
        );
        assert!(uncovered >= 8, "{uncovered}");
        let generated: Vec<_> = generated_history(240)
            .into_iter()
            .map(|sql| (sql, None))
            .collect();
        let (failed, uncovered) = assert_incremental_inspection_is_exact(&generated, 120);
        // Most of the generated history runs (replay refuses some drops that
        // would run ON DELETE actions, and what follows them can fail), and
        // both covered and uncovered states occur.
        assert!(
            failed.len() <= 60 && (24..=216).contains(&uncovered),
            "{failed:?} {uncovered}"
        );
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
