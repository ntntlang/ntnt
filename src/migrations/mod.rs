//! Native SQL history, catalog replay and externally authorized relational execution.

mod adapter;
mod adoption;
mod analysis;
mod catalog_sql;
mod execution;
mod input_file;
mod inventory;
mod model;
mod postgres;
mod report;
mod sqlite;
mod sqlite_file;
use serde::Serialize;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize)]
/// Machine-readable failure with a stable code and an actionable, non-secret explanation.
pub struct Diagnostic {
    pub code: String,
    pub message: String,
    pub source: Option<String>,
    pub next_action: String,
}

impl Diagnostic {
    pub(crate) fn new(code: &str, message: &str) -> Self {
        let next_action = match code {
            "DB_ADOPTION_INPUT" => "Select an exact source-prefix ID with plan --adopt-through and declare named adoption_checks. Only absent history or matching recognized legacy history is eligible; never reset or rewrite a ledger to force adoption.",
            "DB_SQLITE_RECOVERY_REQUIRED" => "If a writer is active, wait for it to finish and retry inspection. Otherwise rerun migrate with the same approved plan to recover and reconcile under ownership; do not delete the journal or make inspection writable.",
            "DB_REFERENCE_UNAVAILABLE" => "Configure [databases.<name>.scratch] for the selected database with a distinct disposable database and secret, then rerun inspection.",
            "DB_DATABASE_REQUIRED" | "DB_DATABASE_UNKNOWN" => "Select a declared [databases.<name>] with --database <name>. --database may be omitted only when exactly one logical database is declared; --target is always required.",
            "DB_SCRATCH_COLLISION" | "DB_SCRATCH_CREDENTIALS" | "DB_SCRATCH_NOT_EMPTY" | "DB_SCRATCH_MISMATCH" => "Provision a separate empty engine-matching scratch database/file. PostgreSQL requires a separate unprivileged owner role. Do not reset the selected target.",
            "DB_REPLAY_UNSUPPORTED" => "Review the named migration against docs/DATABASE_MIGRATIONS.md. Unsupported SQL needs adapter support or separate manual review; do not replay it on the target.",
            "DB_COVERAGE_INCOMPLETE" | "DB_CATALOG_AMBIGUOUS" => "Inspect catalog.uncovered and object differences. Extend supported inspection or obtain manual evidence; do not interpret this result as no drift.",
            "DB_DRIFT" => "Review drift.differences against the applied release. Reconcile deliberately; these commands never repair or adopt state.",
            "DB_LEDGER_UNKNOWN" | "DB_UNRECOGNIZED_HISTORY" => "Inspect the existing ledger and managed scope with an operator. Baseline adoption is not implemented; never insert fictional applied rows.",
            "DB_HISTORY_MISSING" | "DB_HISTORY_DIVERGENT" | "DB_HISTORY_EDITED" | "DB_HISTORY_DIRTY" => "Recover the correct immutable release history or investigate the dirty attempt. Do not rewrite checksums, reorder applied files or blindly retry execution.",
            "DB_SECRET" => "Declare the connection secret in ntnt.toml and configure the existing std/secrets provider. Never pass a connection URL on the command line.",
            "DB_TIMEOUT" => "Check endpoint availability and conflicting locks. Preserve committed progress; reconcile unknown migration outcomes with the same reviewed plan before any further SQL.",
            "DB_PLAN_STALE" | "DB_PLAN_INVALID" | "DB_APPROVAL_REQUIRED" | "DB_LEDGER_SCOPE" | "DB_LEDGER_UNSAFE" => "Generate and review a new plan for the explicit target; a plan digest never grants authority.",
            "DB_OUTPUT" => "Select a new output filename with a writable parent directory; existing files are never replaced.",
            "DB_OUTPUT_LIMIT" => "Reduce the saved report using --object, or reduce oversized migration metadata. No artifact was written; verification is never truncated to fit.",
            _ => "Correct the reported input or configuration using docs/DATABASE_MIGRATIONS.md; inspect committed progress before retrying any migration.",
        };
        Self {
            code: code.into(),
            message: message.into(),
            source: None,
            next_action: next_action.into(),
        }
    }
}
impl std::fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}
impl std::error::Error for Diagnostic {}
pub type Result<T> = std::result::Result<T, Diagnostic>;

#[derive(clap::Args)]
/// Inspect database history/catalogs or execute a separately authorized exact plan.
pub struct DbArgs {
    #[command(subcommand)]
    pub command: DbCommand,
}
#[derive(clap::Subcommand)]
/// Read-only observations and explicit execution share captured inputs and catalog evidence.
pub enum DbCommand {
    /// Inspect catalog, migration history, pending changes and drift (read-only target).
    Status(Options),
    /// Capture a target-bound, non-authorizing plan (never applies target SQL).
    Plan(Options),
    /// Verify the applied catalog/history; never runs application checks or repairs.
    Verify(Options),
    /// Execute an externally authorized exact plan, one transaction per migration.
    Migrate {
        #[command(flatten)]
        options: Options,
        #[arg(long)]
        plan: PathBuf,
    },
}
#[derive(clap::Args)]
/// Explicit selection and output controls shared by database inspection commands.
pub struct Options {
    #[arg(long, default_value = "ntnt.toml")]
    pub manifest: PathBuf,
    /// Logical database name; optional only when exactly one is declared.
    #[arg(long)]
    pub database: Option<String>,
    #[arg(long)]
    pub target: String,
    #[arg(long)]
    pub json: bool,
    /// Write a report/compact plan to a NEW file (never overwrite existing files).
    #[arg(long)]
    pub out: Option<PathBuf>,
    /// Restrict displayed catalog objects, not verification or plan identity.
    #[arg(long)]
    pub object: Option<String>,
    /// Revalidate a previously saved plan's source/target/base binding (no execution).
    #[arg(long)]
    pub against: Option<PathBuf>,
    /// Plan explicit adoption through this migration ID; never execute its old SQL.
    #[arg(long, value_parser = clap::value_parser!(u64).range(1..))]
    pub adopt_through: Option<u64>,
}

/// Run one database operation with an outer deadline and bounded runtime teardown.
/// Dedicated sessions are discarded; uncancellable resolver work cannot keep the CLI alive.
fn database_runtime<T>(future: impl std::future::Future<Output = Result<T>>) -> Result<T> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| Diagnostic::new("DB_RUNTIME", "Cannot create database I/O runtime."))?;
    let result = runtime.block_on(async {
        tokio::time::timeout(std::time::Duration::from_secs(120), future)
            .await
            .map_err(|_| {
                Diagnostic::new(
                    "DB_TIMEOUT",
                    "Total database operation deadline exceeded; dedicated sessions discarded. Retain reported committed progress and reconcile unknown work using the same plan.",
                )
            })?
    });
    // Tokio's OS resolver can outlive a cancelled connect future. Runtime Drop
    // otherwise waits indefinitely for blocking work and defeats our I/O bound.
    // No client/connection is recycled; detached resolver work cannot issue SQL.
    runtime.shutdown_timeout(std::time::Duration::from_millis(100));
    result
}

/// Save a bounded report or compact plan to a new file without replacing existing data.
/// Artifact size is checked against the reader limit before creation. An I/O failure
/// returns an error and may leave a partial newly created file, never a success verdict.
fn save_report(path: &std::path::Path, command: &str, report: &serde_json::Value) -> Result<()> {
    use std::io::Write;
    // Save the plan descriptor, not duplicated display catalogs and source
    // summaries. stdout still contains the complete observation report.
    let artifact = if command == "plan" && !report["plan"].is_null() {
        serde_json::json!({
            "format_version": report["format_version"],
            "command": command,
            "observed_at": report["observed_at"],
            "outcome": report["outcome"],
            "plan": report["plan"],
            "plan_digest": report["plan_digest"],
            "diagnostics": report["diagnostics"],
        })
    } else {
        report.clone()
    };
    let output_error = || {
        Diagnostic::new("DB_OUTPUT", "Cannot save report. Destination must be a new writable file; existing files are never replaced.")
    };
    let bytes = serde_json::to_vec_pretty(&artifact).map_err(|_| output_error())?;
    if bytes.len() > inventory::MAX_INPUT_BYTES {
        return Err(Diagnostic::new(
            "DB_OUTPUT_LIMIT",
            "Saved artifact exceeds the 1 MiB readback limit; no file was created.",
        ));
    }
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path).map_err(|_| output_error())?;
    file.write_all(&bytes).map_err(|_| output_error())?;
    file.sync_all().map_err(|_| output_error())
}

/// Synchronous CLI entry point: capture inputs, inspect/replay, and render one result.
///
/// Only `migrate` may mutate the target, after independent approval and locked
/// revalidation. Output files are new-only; inspection/plan digests never authorize writes.
/// Returns an error after rendering any failed or unknown result.
pub fn run(args: DbArgs) -> anyhow::Result<()> {
    let (command, options, execution_plan) = match args.command {
        DbCommand::Status(o) => ("status", o, None),
        DbCommand::Plan(o) => ("plan", o, None),
        DbCommand::Verify(o) => ("verify", o, None),
        DbCommand::Migrate { options, plan } => ("migrate", options, Some(plan)),
    };
    // Capture a review artifact before database I/O; never reopen it later.
    let prior = options.against.as_ref().map(|p| {
        inventory::file_bytes(p).and_then(|b| {
            serde_json::from_slice::<serde_json::Value>(&b)
                .map_err(|_| Diagnostic::new("DB_PLAN_INVALID", "Invalid saved plan JSON."))
        })
    });
    let inputs = inventory::CapturedInputs::load(
        &options.manifest,
        &options.target,
        options.database.as_deref(),
    );
    let mut report = if let Some(path) = execution_plan {
        let mut progress = serde_json::json!({"format_version":1,"command":"migrate",
            "observed_at":chrono::Utc::now().to_rfc3339(),"outcome":"unknown","diagnostics":[],
            "progress":[],"assertions":{"outcome":"not_run","executed":0}});
        let work = inputs.and_then(|inputs| {
            if options.against.is_some() || options.adopt_through.is_some() {
                return Err(Diagnostic::new(
                    "DB_CONFIG",
                    "migrate uses only --plan; --adopt-through belongs to plan.",
                ));
            }
            let bytes = inventory::file_bytes(&path)?;
            let saved = serde_json::from_slice(&bytes).map_err(|_| {
                Diagnostic::new("DB_PLAN_INVALID", "Invalid saved execution plan JSON.")
            })?;
            database_runtime(execution::migrate(&inputs, &saved, &mut progress))
        });
        if let Err(d) = work {
            progress["diagnostics"]
                .as_array_mut()
                .unwrap()
                .push(serde_json::json!(d));
            progress["outcome"] = serde_json::json!(if progress["progress"]
                .as_array()
                .unwrap()
                .iter()
                .any(|p| p["state"] == "unknown")
            {
                "unknown"
            } else {
                "fail"
            });
        }
        progress
    } else {
        inputs
            .and_then(|inputs| {
                if options.adopt_through.is_some() && (command!="plan" || options.against.is_some()) {
                    return Err(Diagnostic::new("DB_CONFIG","--adopt-through is only for an explicit plan, not status/verify/--against."));
                }
                database_runtime(report::inspect(inputs, command, options.adopt_through))
            })
            .unwrap_or_else(|d| report::failure(command, d))
    };
    if let Some(prior) = prior {
        let valid = prior.as_ref().is_ok_and(|p| {
            p["format_version"] == 1
                && p["outcome"] == "pass"
                && p["plan"]["authorized"] == false
                && p["plan"]["executable"] == false
                && p["plan"]["candidate"]["outcome"] == "pass"
                && p["plan_digest"] == inventory::json_digest(&p["plan"])
                && !report["binding"].is_null()
                && p["plan"]["binding"] == report["binding"]
                && p["plan"]["pending"]
                    == serde_json::json!(report["source"]["migrations"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .skip(report["history"]["covered_count"].as_u64().unwrap() as usize)
                        .collect::<Vec<_>>())
        });
        report["plan_validation"] =
            serde_json::json!({"outcome":if valid {"pass"} else {"fail"},"authorized":false});
        if !valid {
            report["outcome"] = serde_json::json!("fail");
            report["diagnostics"].as_array_mut().unwrap().push(serde_json::json!(Diagnostic::new("DB_PLAN_STALE","Saved plan is invalid, incomplete, or bound to different source, policy, target or base state. Generate a new plan.")));
        }
    }
    if let Some(filter) = &options.object {
        if let Some(objects) = report["catalog"]["objects"].as_object_mut() {
            objects.retain(|key, _| key.contains(filter));
        }
        report["display_filter"] = serde_json::json!(filter);
    }
    if let Some(path) = &options.out {
        if let Err(diagnostic) = save_report(path, command, &report) {
            report["outcome"] = serde_json::json!("fail");
            report["diagnostics"]
                .as_array_mut()
                .unwrap()
                .push(serde_json::json!(diagnostic));
        }
    }
    let output = serde_json::to_string_pretty(&report)?;
    if options.json {
        println!("{output}");
    } else {
        println!(
            "Database {command}: {}",
            report["outcome"].as_str().unwrap_or("unknown")
        );
        if let Some(database) = report.get("database") {
            println!("Logical database: {database}");
        }
        if let Some(t) = report.get("target") {
            println!(
                "Target: {} / {} ({})",
                t["name"], t["schema"], t["identity"]["database"]
            );
        }
        if let Some(h) = report.get("history") {
            println!(
                "History: {} applied, {} adopted, {} imported, {} pending; drift: {}",
                h["applied_count"],
                h["adopted_count"],
                h["imported_count"],
                h["pending_count"],
                report["drift"]["outcome"]
            );
        }
        if let Some(objects) = report["catalog"]["objects"].as_object() {
            println!(
                "Catalog coverage: {} object(s) displayed; unverified objects: {}",
                objects.len(),
                report["catalog"]["uncovered"]
            );
            for (key, value) in objects {
                println!("  {key:?}: {value}");
            }
        }
        for change in report["drift"]["differences"]
            .as_array()
            .into_iter()
            .flatten()
        {
            println!("Drift: {} {}", change["change"], change["object"]);
        }
        for migration in report["plan"]["pending"].as_array().into_iter().flatten() {
            println!(
                "Pending {} ({}): hazards {}; unknowns {}",
                migration["source"],
                migration["checksum"],
                migration["analysis"]["hazards"],
                migration["analysis"]["unknowns"]
            );
        }
        for d in report["diagnostics"].as_array().into_iter().flatten() {
            println!(
                "{}: {}\n  Next: {}",
                d["code"], d["message"], d["next_action"]
            );
        }
        if let Some(d) = report.get("plan_digest") {
            println!("Plan digest: {d} (not authorization)");
        }
        println!(
            "Assertions: {} ({} executed). Catalog equivalence is not data correctness.",
            report["assertions"]["outcome"]
                .as_str()
                .unwrap_or("not_run"),
            report["assertions"]["executed"]
        );
        for entry in report["progress"].as_array().into_iter().flatten() {
            println!("Migration {}: {}", entry["source"], entry["state"]);
        }
    }
    if report["outcome"] == "pass" {
        Ok(())
    } else {
        anyhow::bail!("Database operation did not pass")
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn oversized_artifact_is_rejected_before_creating_output() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("oversized.json");
        let report = serde_json::json!({
            "format_version": 1,
            "outcome": "pass",
            "plan": {"padding": "x".repeat(super::inventory::MAX_INPUT_BYTES)},
        });
        let error = super::save_report(&path, "plan", &report).unwrap_err();
        assert_eq!(error.code, "DB_OUTPUT_LIMIT");
        assert!(!path.exists());
    }

    #[test]
    fn runtime_cleanup_does_not_wait_for_uncancellable_resolver_work() {
        let start = std::time::Instant::now();
        super::database_runtime(async {
            let (started, wait) = tokio::sync::oneshot::channel();
            tokio::task::spawn_blocking(move || {
                let _ = started.send(());
                std::thread::sleep(std::time::Duration::from_secs(1));
            });
            wait.await.unwrap();
            Ok(())
        })
        .unwrap();
        assert!(start.elapsed() < std::time::Duration::from_millis(500));
    }
}
