//! Native SQL history, catalog replay and externally authorized relational execution.

mod adapter;
mod adoption;
mod analysis;
mod catalog_sql;
mod execution;
mod input_file;
mod inventory;
mod model;
mod plan_state;
mod postgres;
pub(crate) mod postgres_tls;
mod report;
mod scaffold;
mod sqlite;
mod sqlite_cache;
mod sqlite_errors;
mod sqlite_file;
use serde::Serialize;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize)]
/// Machine-readable failure with a stable code and an actionable, non-secret explanation.
pub struct Diagnostic {
    pub code: String,
    pub message: String,
    pub source: Option<String>,
    /// Catalog keys of the objects this diagnostic is about, such as the
    /// uncovered objects of `DB_COVERAGE_INCOMPLETE`, or the names a statement
    /// refused before it ran uses (`DB_OUTSIDE_SCOPE`). Omitted when empty.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub objects: Vec<String>,
    pub next_action: String,
}

/// A migration with no statements yet (#428): only comments, whitespace or
/// semicolons, such as the file `ntnt db new` creates.
pub(super) fn empty_migration() -> Diagnostic {
    Diagnostic::new(
        "DB_MIGRATION_EMPTY",
        "Migration has no SQL statements yet; it holds only comments or whitespace.",
    )
}

impl Diagnostic {
    pub(crate) fn new(code: &str, message: &str) -> Self {
        let next_action = match code {
            "DB_ADOPTION_INPUT" => "Select an exact source-prefix ID with plan --adopt-through and declare named adoption_checks. Only absent history or matching recognized legacy history is eligible; never reset or rewrite a ledger to force adoption.",
            "DB_SQLITE_RECOVERY_REQUIRED" => "If a writer is active, wait for it to finish and retry inspection. Otherwise rerun migrate with the same approved plan to recover and reconcile under ownership; do not delete the journal or make inspection writable.",
            "DB_REFERENCE_UNAVAILABLE" => "Configure [databases.<name>.scratch] for the selected database with a distinct disposable database and secret, then rerun inspection.",
            "DB_DATABASE_REQUIRED" | "DB_DATABASE_UNKNOWN" => "Select a declared [databases.<name>] with --database <name>. --database may be omitted only when exactly one logical database is declared; --target is required by every command except init and new.",
            "DB_SCRATCH_COLLISION" | "DB_SCRATCH_CREDENTIALS" | "DB_SCRATCH_NOT_EMPTY" | "DB_SCRATCH_MISMATCH" => "Provision a separate empty engine-matching scratch database/file. PostgreSQL requires a separate unprivileged owner role. Do not reset the selected target.",
            "DB_REPLAY_UNSUPPORTED" => "Review the named migration's analysis.unknowns against docs/DATABASE_MIGRATIONS.md. Remove transaction control, role/session changes and other non-transactional statements from migration files. The one supported non-transactional migration is a lone named CREATE INDEX CONCURRENTLY without checks.",
            "DB_COVERAGE_INCOMPLETE" => "Its objects field lists each catalog object outside supported inspection; when replaying a migration produced them, its source field names that migration (the target's own uncovered objects are also in catalog.uncovered). Change or remove those objects to stay within the coverage in docs/DATABASE_MIGRATIONS.md, or obtain manual evidence; do not interpret this result as no drift.",
            "DB_CATALOG_AMBIGUOUS" => "Inspect catalog.uncovered and object differences. Extend supported inspection or obtain manual evidence; do not interpret this result as no drift.",
            "DB_OUTSIDE_SCOPE" => "Keep the named objects inside the managed scope: create, change and drop objects only in the configured schema or in a schema the history itself creates with a top-level CREATE SCHEMA statement. Its objects field names each object outside that scope and source names the migration. Provision shared objects, such as extensions in public, outside migrations. See docs/DATABASE_MIGRATIONS.md#managed-scope.",
            "DB_DRIFT" => "Review drift.differences against the applied release. Reconcile deliberately; these commands never repair or adopt state.",
            "DB_UNRECOGNIZED_HISTORY" => "Review the existing managed scope. To adopt a matching captured prefix, declare named adoption_checks and create an explicit plan --adopt-through <migration-id>; obtain separate exact-plan approval before migrate. Never insert fictional applied rows.",
            "DB_LEDGER_UNKNOWN" => "Inspect the existing ledger and managed scope with an operator. Unknown ledgers are not eligible for automatic adoption or repair; never reset, rewrite or insert fictional applied rows.",
            "DB_HISTORY_MISSING" | "DB_HISTORY_DIVERGENT" | "DB_HISTORY_EDITED" | "DB_HISTORY_DIRTY" => "Recover the correct immutable release history or investigate the dirty attempt. Do not rewrite checksums, reorder applied files or blindly retry execution.",
            "DB_SECRET" => "Declare the named connection secret under [secrets.<NAME>] in ntnt.toml and make it resolvable through the configured std/secrets provider (development: an environment variable with that exact name; production: NTNT_SECRETS_PROVIDER=unix-socket). Never pass a connection URL on the command line.",
            "DB_CONNECTION_CONFIG" => "Make the failing PostgreSQL connection secret (target or scratch) name exactly one explicit host, the user and dbname declared as that endpoint's role and database, and an explicit sslmode, with no startup options. Use sslmode=verify-full for any network path (host CA store by default, or sslrootcert=/absolute/path/ca.pem for a private CA); verify-ca only when the host name cannot match the certificate, and only with sslrootcert=/absolute/path/ca.pem naming the CA that issued it (never the host CA store); require only to accept an unverified server; disable only on a trusted local transport such as loopback, a Unix socket or a private container network.",
            "DB_TLS" => "Check the server certificate against the configured trust. verify-full checks the chain and the host name against the host CA store, or only against the CAs in sslrootcert=/absolute/path/ca.pem (put a managed provider's CA bundle there). Use verify-ca, with sslrootcert naming the CA that issued the server certificate, only when the host name cannot match. A server without TLS is never used in plaintext: enable TLS on the server, or choose sslmode=disable explicitly for a trusted local transport.",
            "DB_CONFIG" => "Fix ntnt.toml or the command line as the message describes: declare [databases.<name>] with engine, schema, a confined relative migrations_dir and named targets, use logical secret references (a separate disposable scratch secret and a separate approval_secret), give a [databases.<name>.limits] table only whole seconds (statement_seconds 2-86400; migrator_wait_seconds 1-300, PostgreSQL only), and pass --adopt-through only to plan. See docs/DATABASE_MIGRATIONS.md#quick-start.",
            "DB_ATTEMPT_UNRESOLVED" => "An earlier migrate stopped during a non-transactional migration. Make sure no migrate or index build is still running, then run `ntnt db reconcile --plan <the same approved plan>`; it records the outcome it observes or rolls the attempt back. Nothing else runs until then.",
            "DB_ATTEMPT_ACTIVE" => "The index build is still running in another session. Wait for it to finish or cancel that backend, then reconcile again.",
            "DB_RECONCILE_INCOMPLETE" => "The planned invalid index was dropped, so the database has changed, but the result did not match the plan's state before the migration and the attempt was kept. Inspect drift with an operator, then reconcile again.",
            "DB_RECONCILE_MISMATCH" => "The database does not match either outcome the plan predicts for this attempt. Nothing was changed. Inspect the named object with an operator; never edit the ledger to force a state.",
            "DB_REBUILD_REQUIRED" => "Rebuild the named table with a `sqlite_rebuild = \"<table>\"` sidecar (docs/DATABASE_MIGRATIONS.md, SQLite table rebuilds), which runs the migration with foreign-key actions off and checks the rebuilt table, or remove those foreign keys in an earlier migration. Nothing ran.",
            "DB_MIGRATION_EMPTY" => "Write the migration's SQL below its header comment, or delete the file if it is not needed yet. Nothing was run.",
            "DB_STATEMENT_LIMIT" => "Make the statement cheaper. For migrate on a target, raise statement_seconds in [databases.<name>.limits] (docs/DATABASE_MIGRATIONS.md, Time limits) and review a new plan, since the limit is part of the plan; replay's limit on the empty scratch database is fixed.",
            "DB_BUSY" => "Retry once the writer finishes: wait for the other connection's write transaction to end, or close the connection that set PRAGMA locking_mode=EXCLUSIVE (it keeps the lock until it closes), then rerun the same command. Nothing was committed while ntnt waited; migrate, rerun with the same plan, skips any migration already committed.",
            "DB_LOCK_CONFLICT" => "Another session held a conflicting lock; nothing was committed for this migration. Retry the same reviewed plan when application or migrator load allows.",
            "DB_TIMEOUT" => "Check endpoint availability and conflicting locks. Preserve committed progress; reconcile unknown migration outcomes with the same reviewed plan before any further SQL.",
            "DB_PLAN_STALE" | "DB_PLAN_INVALID" | "DB_LEDGER_SCOPE" | "DB_LEDGER_UNSAFE" => "Generate and review a new plan for the explicit target; a plan digest never grants authority.",
            "DB_APPROVAL_REQUIRED" => "Set this database's approval_secret, by its name, to the reviewed plan's plan_digest through the secrets provider, then rerun migrate with the same plan; a plan digest grants nothing until that secret holds it.",
            "DB_OUTPUT" => "Select a new output filename with a writable parent directory; existing files are never replaced.",
            "DB_OUTPUT_LIMIT" => "Reduce the saved report using --object, or reduce oversized migration metadata. No artifact was written; verification is never truncated to fit.",
            "DB_INIT_CONFLICT" => "Nothing was written. If another `ntnt db init` or `ntnt db new` held the manifest lock, or an editor was changing the manifest, rerun init once it has finished; do not run init while ntnt.toml is being edited. Otherwise choose another --database name (its secret names derive from it), or edit the existing [databases.<name>] and [secrets.<NAME>] declarations by hand; init never changes existing declarations or replaces an existing migrations path. Give a further database its own migrations_dir as in docs/DATABASE_MIGRATIONS.md#multiple-databases-in-one-application.",
            "DB_INIT_NAME" => "Pass --database with 1-50 lowercase ASCII letters, digits or underscores, starting with a letter (for example --database app). Names starting with pg_ and information_schema are reserved; secret, role and database names are derived from this name.",
            "DB_MIGRATION_NAME" => "Use 1-64 lowercase ASCII letters, digits or underscores, for example `ntnt db new add_accounts`; the numeric ID and .sql suffix are added for you.",
            "DB_MIGRATION_EXISTS" => "The existing file was left unchanged. Rerun `ntnt db new` to take the next free ID; never edit or renumber a migration that has been applied.",
            "DB_NEW_CONFLICT" => "Nothing was created. Rerun `ntnt db new` once the other `ntnt db new` or `ntnt db init` has finished; it then takes the next free ID. The lock ends when that command exits, so there is nothing to clean up.",
            "DB_SCAFFOLD_IO" => "Make the project and migrations directories writable and keep ntnt.toml a writable regular file on a filesystem that supports file locks, then rerun. Existing files are never truncated: an existing ntnt.toml is replaced only by a complete, synced copy with the new section appended.",
            _ => "Correct the reported input or configuration using docs/DATABASE_MIGRATIONS.md; inspect committed progress before retrying any migration.",
        };
        Self {
            code: code.into(),
            message: message.into(),
            source: None,
            objects: Vec::new(),
            next_action: next_action.into(),
        }
    }
    /// Name the catalog objects (their keys) this diagnostic is about.
    pub(crate) fn with_objects(mut self, objects: &[String]) -> Self {
        self.objects = objects.to_vec();
        self
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
/// Scaffold first-time setup, inspect database history/catalogs, or execute a
/// separately authorized exact plan.
pub struct DbArgs {
    #[command(subcommand)]
    pub command: DbCommand,
}
#[derive(clap::Subcommand)]
/// Read-only observations and explicit execution share captured inputs and catalog evidence.
pub enum DbCommand {
    /// Declare a logical database in ntnt.toml (secret names only) and create migrations/.
    Init(InitArgs),
    /// Create the next numbered SQL migration file (no secrets or connection needed).
    New(NewArgs),
    /// Inspect catalog, migration history, pending changes and drift (read-only target).
    Status(InspectArgs),
    /// Capture a target-bound, non-authorizing plan (never applies target SQL).
    Plan(PlanArgs),
    /// Verify the applied catalog/history; never runs application checks or repairs.
    Verify(InspectArgs),
    /// Execute an externally authorized exact plan, one transaction per migration.
    Migrate {
        #[command(flatten)]
        options: ExecuteArgs,
        #[arg(long)]
        plan: PathBuf,
    },
    /// Resolve an interrupted non-transactional migration under the same approved plan.
    Reconcile {
        #[command(flatten)]
        options: ExecuteArgs,
        #[arg(long)]
        plan: PathBuf,
    },
}
#[derive(clap::Args)]
/// Explicit selection and output controls shared by every command that reads a target.
pub struct Selection {
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
}
// Each command's --help lists only the flags it takes (#432). A flag it does
// not take is still parsed, hidden, so the command line keeps its JSON
// refusal naming that flag (`refused_flag`).
#[derive(clap::Args)]
/// status and verify: inspect, optionally revalidating a saved plan.
pub struct InspectArgs {
    #[command(flatten)]
    pub selection: Selection,
    /// Revalidate a previously saved plan's source/target/base binding (no execution).
    #[arg(long)]
    pub against: Option<PathBuf>,
    #[arg(long, hide = true, value_parser = clap::value_parser!(u64).range(1..))]
    pub adopt_through: Option<u64>,
}
#[derive(clap::Args)]
/// plan: capture a plan, or revalidate a saved one.
pub struct PlanArgs {
    #[command(flatten)]
    pub selection: Selection,
    /// Revalidate a previously saved plan's source/target/base binding (no execution).
    #[arg(long)]
    pub against: Option<PathBuf>,
    /// Plan explicit adoption through this migration ID; never execute its old SQL.
    #[arg(long, value_parser = clap::value_parser!(u64).range(1..))]
    pub adopt_through: Option<u64>,
}
#[derive(clap::Args)]
/// migrate and reconcile: run the saved plan given with --plan.
pub struct ExecuteArgs {
    #[command(flatten)]
    pub selection: Selection,
    #[arg(long, hide = true)]
    pub against: Option<PathBuf>,
    #[arg(long, hide = true, value_parser = clap::value_parser!(u64).range(1..))]
    pub adopt_through: Option<u64>,
}
/// One command's selection and flags, as `run` uses them.
struct Options {
    manifest: PathBuf,
    database: Option<String>,
    target: String,
    json: bool,
    out: Option<PathBuf>,
    object: Option<String>,
    against: Option<PathBuf>,
    adopt_through: Option<u64>,
}
impl Options {
    fn new(selection: Selection, against: Option<PathBuf>, adopt_through: Option<u64>) -> Self {
        let Selection {
            manifest,
            database,
            target,
            json,
            out,
            object,
        } = selection;
        Self {
            manifest,
            database,
            target,
            json,
            out,
            object,
            against,
            adopt_through,
        }
    }
}
/// A flag `command` does not take, refused naming that flag (#432).
fn refused_flag(command: &str, options: &Options) -> Option<Diagnostic> {
    let (against, adopt) = (options.against.is_some(), options.adopt_through.is_some());
    let (message, next_action) = match command {
        "migrate" | "reconcile" => match (against, adopt) {
            (true, true) => (
                format!("{command} does not take --against or --adopt-through: it runs only the saved plan given with --plan, which it revalidates itself."),
                format!("Drop both and rerun `ntnt db {command} --plan <file>` with the reviewed plan."),
            ),
            (true, false) => (
                format!("{command} does not take --against: it runs the saved plan given with --plan and revalidates that plan itself."),
                format!("Drop --against and rerun `ntnt db {command} --plan <file>` with the reviewed plan; `ntnt db verify --against <file>` checks a saved plan without running it."),
            ),
            (false, true) => (
                format!("{command} does not take --adopt-through: an adoption is planned by `ntnt db plan --adopt-through <id> --out <file>`."),
                format!("Drop --adopt-through and rerun `ntnt db {command} --plan <file>` with the adoption plan that command saved."),
            ),
            (false, false) => return None,
        },
        "plan" if against && adopt => (
            "plan takes --adopt-through or --against, not both: --adopt-through plans a new adoption, and --against revalidates a saved plan.".to_owned(),
            "Pass --adopt-through <id> to plan the adoption, or --against <file> alone to revalidate the saved adoption plan.".to_owned(),
        ),
        "status" | "verify" if adopt => (
            format!("{command} does not take --adopt-through: only `ntnt db plan --adopt-through <id>` plans an adoption."),
            format!("Drop --adopt-through from {command}, or plan the adoption with `ntnt db plan --adopt-through <id>`."),
        ),
        _ => return None,
    };
    let mut d = Diagnostic::new("DB_CONFIG", &message);
    d.next_action = next_action;
    Some(d)
}

#[derive(Clone, Copy, clap::ValueEnum)]
/// Engine of a newly declared logical database.
pub enum Engine {
    Sqlite,
    Postgresql,
}
#[derive(clap::Args)]
/// First-time setup: declare one logical database without secret values or a connection.
pub struct InitArgs {
    /// Database engine of the new logical database.
    #[arg(long, value_enum)]
    pub engine: Engine,
    /// Logical database name; secret, role and database names derive from it.
    #[arg(long, default_value = "app")]
    pub database: String,
    /// Manifest to create, or to extend without changing its existing bytes.
    #[arg(long, default_value = "ntnt.toml")]
    pub manifest: PathBuf,
    #[arg(long)]
    pub json: bool,
}
#[derive(clap::Args)]
/// Create the next numbered SQL migration; existing files are never replaced.
pub struct NewArgs {
    /// Migration name: lowercase ASCII letters, digits and underscores (add_accounts).
    pub name: String,
    #[arg(long, default_value = "ntnt.toml")]
    pub manifest: PathBuf,
    /// Logical database name; optional only when exactly one is declared.
    #[arg(long)]
    pub database: Option<String>,
    #[arg(long)]
    pub json: bool,
}

/// Run one database operation with an outer deadline and bounded runtime teardown.
/// Dedicated sessions are discarded; uncancellable resolver work cannot keep the CLI alive.
fn database_runtime<T>(
    deadline: std::time::Duration,
    future: impl std::future::Future<Output = Result<T>>,
) -> Result<T> {
    database_runtime_on(
        tokio::runtime::Builder::new_current_thread(),
        deadline,
        future,
    )
}

/// `database_runtime` on a given current-thread builder. Tests pass one that
/// starts with a paused clock.
fn database_runtime_on<T>(
    mut builder: tokio::runtime::Builder,
    deadline: std::time::Duration,
    future: impl std::future::Future<Output = Result<T>>,
) -> Result<T> {
    let runtime = builder
        .enable_all()
        .build()
        .map_err(|_| Diagnostic::new("DB_RUNTIME", "Cannot create database I/O runtime."))?;
    let result = runtime.block_on(async {
        tokio::time::timeout(deadline, future)
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

/// What `--out` holds for `command`.
fn saved_artifact(command: &str) -> &'static str {
    if command == "plan" {
        "plan"
    } else {
        "report"
    }
}
/// A file name for `--out` that does not exist yet, next to `path`: named for
/// the command and the newest migration, as `ntnt db new` suggests
/// (`review-0002.json` for a plan).
fn free_output_name(
    path: &std::path::Path,
    command: &str,
    inputs: Option<&inventory::CapturedInputs>,
) -> Option<String> {
    let newest = inputs?.migrations.last()?;
    let id = newest.name.split_once('_')?.0;
    let stem = if command == "plan" { "review" } else { command };
    let suggestion = path.with_file_name(format!("{stem}-{id}.json"));
    let free = |path: &std::path::Path| std::fs::symlink_metadata(path).is_err();
    let suggestion = if free(&suggestion) {
        suggestion
    } else {
        // The next ID's name, for when this migration's is taken.
        let next = format!("{:0width$}", newest.id.checked_add(1)?, width = id.len());
        Some(path.with_file_name(format!("{stem}-{next}.json"))).filter(|p| free(p))?
    };
    Some(inventory::shown_os(suggestion.as_os_str()))
}
/// `DB_OUTPUT` for `--out` `path`, which cannot be created for `cause`, with
/// advice for that cause (#425).
fn unwritable_output(
    path: &std::path::Path,
    command: &str,
    cause: OutputCause,
    suggestion: Option<&str>,
) -> Diagnostic {
    let shown = inventory::shown_os(path.as_os_str());
    let what = saved_artifact(command);
    let directory = || {
        path.parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .map_or_else(
                || ".".to_owned(),
                |parent| inventory::shown_os(parent.as_os_str()),
            )
    };
    let (problem, next_action) = match cause {
        OutputCause::Exists => (
            "it already exists, and --out never replaces a file".to_owned(),
            match suggestion {
                Some(name) => format!("Pass --out a file that does not exist yet, such as {name}; ntnt db never replaces an existing file, so keep or move the old one."),
                None => "Pass --out a file that does not exist yet; ntnt db never replaces an existing file, so keep or move the old one.".to_owned(),
            },
        ),
        OutputCause::NoDirectory => (
            format!("its directory {} does not exist", directory()),
            format!("Create the directory {} first, or pass --out a new file in an existing directory.", directory()),
        ),
        OutputCause::NotDirectory => (
            format!("{} is not a directory", directory()),
            "Pass --out a new file in an existing directory.".to_owned(),
        ),
        OutputCause::Create(error) => (
            format!("it cannot be created ({error})"),
            "Pass --out a new file in a directory this user can write.".to_owned(),
        ),
        OutputCause::Write(error) => (
            format!("writing it failed ({error}); the new file may be incomplete"),
            format!("Remove the incomplete {shown}, fix the cause (such as a full disk), then rerun with --out."),
        ),
    };
    let mut d = Diagnostic::new(
        "DB_OUTPUT",
        &format!("Cannot save the {what} to {shown}: {problem}."),
    );
    d.source = Some(shown);
    d.next_action = next_action;
    d
}
/// Why `--out` cannot be saved.
enum OutputCause {
    Exists,
    NoDirectory,
    NotDirectory,
    Create(std::io::Error),
    Write(std::io::Error),
}
/// Refuse an `--out` that cannot be created, before any work runs (#425):
/// one that exists, or whose directory does not. Anything else is found when
/// the file is created.
fn check_output(path: &std::path::Path, command: &str, suggestion: Option<&str>) -> Result<()> {
    if std::fs::symlink_metadata(path).is_ok() {
        return Err(unwritable_output(
            path,
            command,
            OutputCause::Exists,
            suggestion,
        ));
    }
    let directory = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(std::path::Path::new("."));
    match std::fs::metadata(directory) {
        Ok(meta) if meta.is_dir() => Ok(()),
        Ok(_) => Err(unwritable_output(
            path,
            command,
            OutputCause::NotDirectory,
            suggestion,
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Err(unwritable_output(
            path,
            command,
            OutputCause::NoDirectory,
            suggestion,
        )),
        // Found again, with its cause, when the file is created.
        Err(_) => Ok(()),
    }
}

/// Save a bounded report or compact plan to a new file without replacing existing data.
/// Artifact size is checked against the reader limit before creation. An I/O failure
/// returns an error and may leave a partial newly created file, never a success verdict.
fn save_report(
    path: &std::path::Path,
    command: &str,
    report: &serde_json::Value,
    suggestion: Option<&str>,
) -> Result<()> {
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
    let failed = |cause| unwritable_output(path, command, cause, suggestion);
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
    let mut file = options.open(path).map_err(|error| {
        failed(match error.kind() {
            std::io::ErrorKind::AlreadyExists => OutputCause::Exists,
            std::io::ErrorKind::NotFound => OutputCause::NoDirectory,
            _ => OutputCause::Create(error),
        })
    })?;
    file.write_all(&bytes)
        .map_err(|error| failed(OutputCause::Write(error)))?;
    file.sync_all()
        .map_err(|error| failed(OutputCause::Write(error)))
}

/// One-line human summary of a PostgreSQL session's transport.
fn describe_transport(transport: &serde_json::Value) -> String {
    let version = transport["tls_version"]
        .as_str()
        .map(|v| format!(", {v}"))
        .unwrap_or_default();
    match transport["sslmode"].as_str().unwrap_or("unknown") {
        "disable" => "plaintext (sslmode=disable; trusted local transport only)".into(),
        "require" => format!("TLS without server verification (sslmode=require){version}"),
        mode => format!(
            "TLS {mode}, {} roots{version}",
            transport["roots"].as_str().unwrap_or("unknown")
        ),
    }
}

/// Synchronous CLI entry point: capture inputs, inspect/replay, and render one result.
///
/// Only `migrate` may mutate the target, after independent approval and locked
/// revalidation. Output files are new-only; inspection/plan digests never authorize writes.
/// Returns an error after rendering any failed or unknown result.
pub fn run(args: DbArgs) -> anyhow::Result<()> {
    let (command, options, execution_plan) = match args.command {
        DbCommand::Init(args) => return scaffold::init(&args),
        DbCommand::New(args) => return scaffold::new(&args),
        DbCommand::Status(a) => (
            "status",
            Options::new(a.selection, a.against, a.adopt_through),
            None,
        ),
        DbCommand::Plan(a) => (
            "plan",
            Options::new(a.selection, a.against, a.adopt_through),
            None,
        ),
        DbCommand::Verify(a) => (
            "verify",
            Options::new(a.selection, a.against, a.adopt_through),
            None,
        ),
        DbCommand::Migrate { options, plan } => (
            "migrate",
            Options::new(options.selection, options.against, options.adopt_through),
            Some(plan),
        ),
        DbCommand::Reconcile { options, plan } => (
            "reconcile",
            Options::new(options.selection, options.against, options.adopt_through),
            Some(plan),
        ),
    };
    let executes = execution_plan.is_some();
    // Capture a review artifact before database I/O; never reopen it later.
    let prior = options.against.as_ref().map(|p| {
        inventory::file_bytes(inventory::Input::Plan(p, "--against")).and_then(|b| {
            serde_json::from_slice::<serde_json::Value>(&b).map_err(|_| {
                let shown = inventory::shown_os(p.as_os_str());
                let mut d = Diagnostic::new(
                    "DB_PLAN_INVALID",
                    &format!("--against file {shown}: not a saved plan (invalid JSON)."),
                );
                d.source = Some(shown);
                d
            })
        })
    });
    let inputs = inventory::CapturedInputs::load(
        &options.manifest,
        &options.target,
        options.database.as_deref(),
    );
    // An --out that cannot be created is refused before any work (#425).
    let suggestion = options
        .out
        .as_deref()
        .and_then(|path| free_output_name(path, command, inputs.as_ref().ok()));
    let refused_output = options
        .out
        .as_deref()
        .and_then(|path| check_output(path, command, suggestion.as_deref()).err());
    // How a stale plan's refusal names the files given (#423).
    let shown_manifest = inventory::shown_os(options.manifest.as_os_str());
    let shown_plan = execution_plan
        .as_ref()
        .or(options.against.as_ref())
        .map(|path| inventory::shown_os(path.as_os_str()));
    let files = plan_state::Files {
        plan: shown_plan.as_deref(),
        manifest: Some(&shown_manifest),
    };
    // A plan --against cannot match for a reason that needs no I/O: the
    // database and target it was made for, its policy or its inputs. Known
    // before the inspection runs, it is reported after it.
    let mut stale_against = None;
    let mut report = if let Some(path) = execution_plan {
        let mut progress = serde_json::json!({"format_version":1,"command":command,
            "observed_at":chrono::Utc::now().to_rfc3339(),"outcome":"unknown","diagnostics":[],
            "progress":[],"assertions":{"outcome":"not_run","executed":0}});
        let work = inputs.and_then(|inputs| {
            if let Some(refused) = refused_flag(command, &options) {
                return Err(refused);
            }
            if let Some(d) = refused_output.clone() {
                return Err(d);
            }
            let bytes = inventory::file_bytes(inventory::Input::Plan(&path, "--plan"))?;
            let saved = serde_json::from_slice(&bytes).map_err(|_| {
                Diagnostic::new("DB_PLAN_INVALID", "Invalid saved execution plan JSON.")
            })?;
            // Refuse a stale plan before any I/O, naming what changed (#423).
            execution::validate_named(&inputs, &saved, files)?;
            let deadline = inputs.limits().total_deadline();
            if command == "reconcile" {
                database_runtime(
                    deadline,
                    execution::reconcile_attempt(&inputs, &saved, &mut progress),
                )
            } else {
                database_runtime(deadline, execution::migrate(&inputs, &saved, &mut progress))
            }
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
                || progress["reconciliation"]["outcome"] == "unknown"
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
                if let Some(refused) = refused_flag(command, &options) {
                    return Err(refused);
                }
                if let Some(d) = refused_output.clone() {
                    return Err(d);
                }
                let mut adopt_through = options.adopt_through;
                if let Some(prior) = &prior {
                    // An unreadable --against file is reported as such (#422).
                    let saved = prior.as_ref().map_err(Clone::clone)?;
                    stale_against = execution::validate_named(&inputs, saved, files).err();
                    if stale_against.is_none() && saved["plan"].get("action").is_some() {
                        // Reuse the pure descriptor validator, never approval or
                        // the writer path. The explicit CLI target still owns
                        // all connection selection and inspection stays readonly.
                        execution::validate(&inputs, saved)?;
                        if let Some(action) = adoption::Adoption::from_plan(&inputs, saved)? {
                            adopt_through = Some(action.through_id.parse().map_err(|_| {
                                Diagnostic::new(
                                    "DB_PLAN_STALE",
                                    "Invalid adoption prefix ID in saved artifact.",
                                )
                            })?);
                        }
                    }
                }
                // Inspection keeps the default bounds: its replay runs on an
                // empty scratch database, not the target's large tables.
                let deadline = inventory::Limits::default().total_deadline();
                database_runtime(deadline, report::inspect(inputs, command, adopt_through))
            })
            .unwrap_or_else(|d| report::failure(command, d))
    };
    if let Some(prior) = prior.filter(|_| !executes) {
        // What makes the saved plan not match, naming the binding (#423): a
        // capture error, a check that needed no I/O, or what was observed.
        let problem = match &prior {
            Err(capture) => Some(capture.clone()),
            Ok(saved) => stale_against.or_else(|| plan_state::against(saved, &report)),
        };
        report["plan_validation"] = serde_json::json!({"outcome":if problem.is_none() {"pass"} else {"fail"},"authorized":false});
        if let Some(problem) = problem {
            report["outcome"] = serde_json::json!("fail");
            let diagnostics = report["diagnostics"].as_array_mut().unwrap();
            // Already the inspection's own failure, or a stale plan it reported.
            let reported = diagnostics.iter().any(|d| {
                d["code"] == problem.code
                    && (d["message"] == problem.message || problem.code == "DB_PLAN_STALE")
            });
            if !reported {
                diagnostics.push(serde_json::json!(problem));
            }
        }
    }
    if let Some(filter) = &options.object {
        if let Some(objects) = report["catalog"]["objects"].as_object_mut() {
            objects.retain(|key, _| key.contains(filter));
        }
        report["display_filter"] = serde_json::json!(filter);
    }
    // Whether --out (when given) was saved: only then is there a saved plan
    // whose digest the human report may show (#425).
    let mut saved = true;
    if let Some(path) = &options.out {
        let failure = match refused_output {
            Some(refused) => Some(refused),
            None => save_report(path, command, &report, suggestion.as_deref()).err(),
        };
        if let Some(diagnostic) = failure {
            saved = false;
            report["outcome"] = serde_json::json!("fail");
            let diagnostics = report["diagnostics"].as_array_mut().unwrap();
            // A refusal before the work is already the command's failure.
            if !diagnostics.contains(&serde_json::json!(diagnostic)) {
                diagnostics.push(serde_json::json!(diagnostic));
            }
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
            // A target not yet inspected has no identity (#425).
            let database = &t["identity"]["database"];
            if database.is_null() {
                println!("Target: {} / {}", t["name"], t["schema"]);
            } else {
                println!("Target: {} / {} ({database})", t["name"], t["schema"]);
            }
            if let Some(transport) = t["identity"].get("transport") {
                println!("Transport: {}", describe_transport(transport));
            }
        }
        if let Some(h) = report.get("history") {
            if h["applied_count"].is_null() {
                // Unknown or refused history has no counts (#423).
                let entries = match h["observed_entry_count"].as_u64() {
                    Some(1) => "; its ledger has 1 entry".to_owned(),
                    Some(n) => format!("; its ledger has {n} entries"),
                    None => String::new(),
                };
                println!(
                    "History: {}{entries}, see the diagnostics below; drift: {}",
                    h["outcome"].as_str().unwrap_or("unknown"),
                    report["drift"]["outcome"]
                );
            } else {
                println!(
                    "History: {} applied, {} adopted, {} imported, {} pending; drift: {}",
                    h["applied_count"],
                    h["adopted_count"],
                    h["imported_count"],
                    h["pending_count"],
                    report["drift"]["outcome"]
                );
            }
        }
        if let Some(action) = report.get("adoption").and_then(|a| a.get("action")) {
            println!(
                "Adoption reference: {} through ID {} ({} source files); adoption not executed.",
                action["kind"].as_str().unwrap_or("unknown"),
                action["through_id"].as_str().unwrap_or("unknown"),
                action["count"]
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
            for lock in migration["analysis"]["locks"]
                .as_array()
                .into_iter()
                .flatten()
            {
                let statements = match lock["through"].as_u64() {
                    Some(end) => format!("statements {}-{end}", lock["statement"]),
                    None => format!("statement {}", lock["statement"]),
                };
                println!(
                    "  Lock: {statements} take {} on {}",
                    lock["mode"], lock["relation"]
                );
            }
            for warning in migration["analysis"]["lock_warnings"]
                .as_array()
                .into_iter()
                .flatten()
            {
                println!(
                    "  Warning {} on {} (statement {}): {}",
                    warning["code"], warning["relation"], warning["statement"], warning["message"]
                );
            }
        }
        for d in report["diagnostics"].as_array().into_iter().flatten() {
            println!("{}: {}", d["code"], d["message"]);
            if !d["source"].is_null() {
                println!("  Source: {}", d["source"]);
            }
            if let Some(objects) = d.get("objects") {
                println!("  Objects: {objects}");
            }
            println!("  Next: {}", d["next_action"]);
        }
        if let Some(r) = report
            .get("reconciliation")
            .filter(|r| r.get("resolution").is_some())
        {
            println!(
                "Reconciliation: {} (index {})",
                r["resolution"].as_str().unwrap_or("unknown"),
                r["observed_index"].as_str().unwrap_or("not inspected")
            );
        }
        if let Some(d) = report.get("plan_digest").filter(|_| saved) {
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
            if let Some(kind) = entry["kind"].as_str() {
                println!(
                    "Adoption {} through ID {}: {}; migration SQL executed: {}",
                    kind,
                    entry["through_id"].as_str().unwrap_or("unknown"),
                    entry["state"].as_str().unwrap_or("unknown"),
                    entry["migration_sql_executed"]
                );
            } else {
                println!("Migration {}: {}", entry["source"], entry["state"]);
            }
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
    fn configuration_diagnostics_have_specific_next_actions() {
        use super::inventory::{
            MAX_MIGRATOR_WAIT_SECONDS, MAX_STATEMENT_SECONDS, MIN_MIGRATOR_WAIT_SECONDS,
            MIN_STATEMENT_SECONDS,
        };
        let generic = super::Diagnostic::new("DB_NOT_A_REAL_CODE", "x").next_action;
        // The next action states the ranges validation actually accepts.
        let statement_range =
            format!("statement_seconds {MIN_STATEMENT_SECONDS}-{MAX_STATEMENT_SECONDS}");
        let wait_range = format!(
            "migrator_wait_seconds {MIN_MIGRATOR_WAIT_SECONDS}-{MAX_MIGRATOR_WAIT_SECONDS}"
        );
        for (code, expected) in [
            (
                "DB_CONNECTION_CONFIG",
                [
                    "target or scratch",
                    "one explicit host",
                    "sslmode=verify-full",
                    "sslrootcert=",
                    "verify-ca only when the host name cannot match the certificate, and only with sslrootcert=",
                    "never the host CA store",
                    "disable only on a trusted local transport",
                    "no startup options",
                ]
                .as_slice(),
            ),
            (
                "DB_TLS",
                [
                    "host CA store",
                    "sslrootcert=",
                    "verify-ca, with sslrootcert",
                    "never used in plaintext",
                ]
                .as_slice(),
            ),
            (
                "DB_CONFIG",
                [
                    "[databases.<name>]",
                    "migrations_dir",
                    "approval_secret",
                    "--adopt-through",
                    "[databases.<name>.limits]",
                    statement_range.as_str(),
                    wait_range.as_str(),
                ]
                .as_slice(),
            ),
            (
                "DB_SECRET",
                ["[secrets.<NAME>]", "std/secrets provider", "exact name"].as_slice(),
            ),
            (
                "DB_INIT_CONFLICT",
                [
                    "Nothing was written",
                    "another `ntnt db init` or `ntnt db new`",
                    "being edited",
                    "--database",
                    "by hand",
                    "#multiple-databases-in-one-application",
                ]
                .as_slice(),
            ),
            (
                "DB_INIT_NAME",
                ["--database", "lowercase", "50", "pg_"].as_slice(),
            ),
            (
                "DB_MIGRATION_NAME",
                ["lowercase", "64", "ntnt db new add_accounts"].as_slice(),
            ),
            (
                "DB_NEW_CONFLICT",
                [
                    "Nothing was created",
                    "Rerun `ntnt db new`",
                    "`ntnt db init`",
                    "next free ID",
                ]
                .as_slice(),
            ),
            (
                "DB_MIGRATION_EXISTS",
                ["left unchanged", "Rerun `ntnt db new`"].as_slice(),
            ),
            ("DB_SCAFFOLD_IO", ["writable", "never truncated"].as_slice()),
            (
                "DB_APPROVAL_REQUIRED",
                [
                    "approval_secret",
                    "plan_digest",
                    "secrets provider",
                    "same plan",
                ]
                .as_slice(),
            ),
            (
                "DB_COVERAGE_INCOMPLETE",
                [
                    "objects field",
                    "source field names that migration",
                    "catalog.uncovered",
                    "no drift",
                ]
                .as_slice(),
            ),
        ] {
            let action = super::Diagnostic::new(code, "x").next_action;
            assert_ne!(action, generic, "{code} uses the generic next action");
            for fragment in expected {
                assert!(
                    action.contains(fragment),
                    "{code}: {fragment:?} missing from {action}"
                );
            }
        }
    }
    #[test]
    fn lock_conflicts_explain_what_to_do() {
        let d = super::Diagnostic::new("DB_LOCK_CONFLICT", "x");
        assert!(
            d.next_action.contains("nothing was committed"),
            "{}",
            d.next_action
        );
    }
    #[test]
    fn oversized_artifact_is_rejected_before_creating_output() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("oversized.json");
        let report = serde_json::json!({
            "format_version": 1,
            "outcome": "pass",
            "plan": {"padding": "x".repeat(super::inventory::MAX_INPUT_BYTES)},
        });
        let error = super::save_report(&path, "plan", &report, None).unwrap_err();
        assert_eq!(error.code, "DB_OUTPUT_LIMIT");
        assert!(!path.exists());
    }

    /// Hang guard for waits in these tests; only a bug or a hang reaches it.
    const HANG_GUARD: std::time::Duration = std::time::Duration::from_secs(60);

    #[test]
    fn the_outer_deadline_is_the_one_supplied() {
        use std::time::Duration;
        // The runtime starts with a paused clock, which moves only when all
        // work waits on timers and then straight to the next one. Work ending
        // a second before the supplied deadline completes; work ending a
        // second after it is cut off, whatever the scheduling. (The deadline
        // used to be a fixed 120 seconds.)
        let deadline = Duration::from_secs(30);
        let run = |work: Duration| {
            let mut paused = tokio::runtime::Builder::new_current_thread();
            paused.start_paused(true);
            super::database_runtime_on(paused, deadline, async move {
                tokio::time::sleep(work).await;
                Ok(())
            })
        };
        assert!(run(deadline - Duration::from_secs(1)).is_ok());
        let error = run(deadline + Duration::from_secs(1)).unwrap_err();
        assert_eq!(error.code, "DB_TIMEOUT");
    }

    #[test]
    fn runtime_cleanup_does_not_wait_for_uncancellable_resolver_work() {
        // The blocking work ends only once database_runtime has returned. If
        // runtime teardown waited for it, database_runtime would never return.
        let (release, released) = std::sync::mpsc::channel::<()>();
        let (returned, finished) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let result = super::database_runtime(std::time::Duration::from_secs(120), async {
                let (started, wait) = tokio::sync::oneshot::channel();
                tokio::task::spawn_blocking(move || {
                    let _ = started.send(());
                    let _ = released.recv();
                });
                wait.await.unwrap();
                Ok(())
            });
            let _ = returned.send(result);
        });
        let result = finished.recv_timeout(HANG_GUARD);
        // Let the blocking work end on every path.
        let _ = release.send(());
        result
            .expect("database_runtime must return while the blocking work still runs")
            .unwrap();
    }
}
