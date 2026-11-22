//! Shared report construction: authored history, observed target, expected prefix.
use super::adapter::{self, Access, Adapter, Identity};
use super::analysis::analyze_in_scope;
use super::inventory::{baseline_checksum, json_digest, CapturedInputs, CatalogDigests, Migration};
use super::model::{Catalog, History, HistoryEntry};
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
/// `DB_SECRET` for a connection secret `name` of `role` (target or scratch,
/// or the approval secret) that could not be resolved: names the secret and
/// the cause, never a value, with advice for that cause (#422).
pub(super) fn unresolved_secret(
    role: &str,
    name: &str,
    engine: &str,
    error: crate::stdlib::secrets::ManifestSecretError,
) -> Diagnostic {
    use crate::stdlib::secrets::ManifestSecretError::*;
    let subject = match role {
        "scratch" => format!("Scratch connection secret {name}"),
        "approval" => format!("Approval secret {name}"),
        _ => format!("Target connection secret {name}"),
    };
    let value = match (engine, role) {
        (_, "approval") => "the reviewed plan's plan_digest",
        ("sqlite", "scratch") => "the absolute path of a separate, empty scratch file (`touch` one: an empty file is a valid SQLite database)",
        ("sqlite", _) => "the absolute path of the existing target file, for example \"$(pwd -P)/data/app.sqlite\"",
        (_, "scratch") => "the scratch database's libpq connection string",
        _ => "the target database's libpq connection string",
    };
    let (message, next_action) = match error {
        UnsetInEnvironment => (
            format!("{subject} is unset or empty in the environment (provider env)."),
            format!("Set the environment variable {name} to {value}, then rerun; the env provider reads exactly that name. See docs/DATABASE_MIGRATIONS.md#quick-start."),
        ),
        UnsetInProvider => (
            format!("{subject} is not set in the unix-socket secrets provider."),
            format!("Add {name}, holding {value}, to the secrets agent behind NTNT_SECRETS_SOCKET_ENDPOINTS for this deployment's authorization scope, then rerun."),
        ),
        Undeclared => (
            format!("{subject} is not declared under [secrets] in the manifest."),
            format!("Add a {} table to the manifest (the name only, never its value), then rerun.", super::secrets_table(name)),
        ),
        InvalidDeclarations => (
            format!("{subject} cannot be resolved: the manifest's [secrets] declarations are invalid."),
            "Correct the manifest's [secrets] tables: each [secrets.<NAME>] may hold only label and description (strings), required (true or false) and environments (an array of strings).".to_owned(),
        ),
        EnvProviderDisabled => (
            format!("{subject} cannot be read: the env secrets provider is disabled when NTNT_ENV is production."),
            "For local development, unset NTNT_ENV or set NTNT_ENV=development. In production, set NTNT_SECRETS_PROVIDER=unix-socket with NTNT_SECRETS_SOCKET_ENDPOINTS and NTNT_SECRETS_AUTHORIZATION_SCOPE.".to_owned(),
        ),
        UnsupportedProvider => (
            format!("{subject} cannot be read: NTNT_SECRETS_PROVIDER names an unsupported provider (supported: env, and unix-socket on Unix)."),
            "Set NTNT_SECRETS_PROVIDER to env for local development or to unix-socket (on Unix, with NTNT_SECRETS_SOCKET_ENDPOINTS and NTNT_SECRETS_AUTHORIZATION_SCOPE), or unset it to use env.".to_owned(),
        ),
        ProviderConfiguration => (
            format!("{subject} cannot be read: the unix-socket secrets provider's configuration is invalid."),
            "Correct NTNT_SECRETS_SOCKET_ENDPOINTS (absolute socket paths, beneath /run/ntnt-secrets in production), NTNT_SECRETS_AUTHORIZATION_SCOPE and NTNT_SECRETS_TIMEOUT_MS (10-10000), as std/secrets documents in docs/STDLIB_REFERENCE.md, then rerun.".to_owned(),
        ),
        NotUnicode => (
            format!("{subject} is set in the environment but is not valid Unicode."),
            format!("Set the environment variable {name} to {value}, as valid Unicode text, then rerun."),
        ),
        ProviderUnavailable => (
            format!("{subject} cannot be read: the secrets provider is unavailable."),
            "Check that the secrets agent is running and listening on NTNT_SECRETS_SOCKET_ENDPOINTS, then rerun.".to_owned(),
        ),
        AccessDenied => (
            format!("{subject} cannot be read: the secrets provider denied access to it."),
            format!("Allow this deployment's authorization scope (NTNT_SECRETS_AUTHORIZATION_SCOPE) to read {name} in the secrets agent, then rerun."),
        ),
        ProviderRejected => (
            format!("{subject} cannot be read: the secrets provider rejected the request."),
            "Check the secrets agent's configuration and its log for the rejected request, then rerun.".to_owned(),
        ),
        InvalidName => (
            format!("{subject} is not a valid secret name."),
            "Use a secret name matching [A-Za-z_][A-Za-z0-9_.-]* of at most 128 characters.".to_owned(),
        ),
    };
    let mut d = Diagnostic::new("DB_SECRET", &message);
    d.next_action = next_action;
    d
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
/// A history refusal about migration `source`, with advice for it (#423).
fn history_refusal(code: &str, source: &str, message: String, next_action: String) -> Diagnostic {
    let mut d = Diagnostic::new(code, &message);
    d.source = Some(source.to_owned());
    d.next_action = next_action;
    d
}
/// The migration file name a ledger entry records, when it has the shape
/// ntnt writes there (`<digits>_<name>.sql`). Reports never echo other ledger
/// text: a tampered ledger could hold anything.
fn ledger_name(entry: &HistoryEntry) -> Option<&str> {
    let name = entry.name.as_str();
    let (digits, rest) = name.strip_suffix(".sql")?.split_once('_')?;
    let word = |b: u8| b.is_ascii_alphanumeric() || b == b'_' || b == b'-';
    (name.len() <= 255
        && !digits.is_empty()
        && digits.bytes().all(|b| b.is_ascii_digit())
        && !rest.is_empty()
        && rest.bytes().all(word))
    .then_some(name)
}
/// The ID a ledger entry records, when it is a decimal number.
fn ledger_id(entry: &HistoryEntry) -> Option<&str> {
    let id = entry.id.as_str();
    (!id.is_empty() && id.len() <= 20 && id.bytes().all(|b| b.is_ascii_digit())).then_some(id)
}
/// The checksum a ledger entry records, when it is a SHA-256 hex digest.
fn ledger_checksum(entry: &HistoryEntry) -> String {
    let checksum = entry.checksum.as_str();
    if checksum.len() == 64
        && checksum
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        format!("checksum {checksum}")
    } else {
        "a checksum that is not a SHA-256 digest".to_owned()
    }
}
/// A ledger entry as history diagnostics name it: `0020_b.sql (ID 20)`.
fn recorded(entry: &HistoryEntry) -> String {
    match (ledger_name(entry), ledger_id(entry)) {
        (Some(name), Some(id)) => format!("{name} (ID {id})"),
        (Some(name), None) => format!("{name} (with an ID that is not a decimal number)"),
        (None, _) => "an entry whose name is not a migration file name".to_owned(),
    }
}
/// `DB_HISTORY_MISSING` for the applied (or adopted) migration `entry`, which
/// is not in the migrations directory `dir`; `others` later entries are not
/// either.
fn missing_migration(dir: &str, entry: &HistoryEntry, others: usize) -> Diagnostic {
    let position = entry.position;
    let Some(name) = ledger_name(entry) else {
        return Diagnostic::new(
            "DB_HISTORY_MISSING",
            &format!("The ledger entry at position {position} is not a migration in {dir}/: its recorded name is not a migration file name."),
        );
    };
    let id = ledger_id(entry).map_or_else(String::new, |id| format!("ID {id}, "));
    let (what, at, adopted) = if entry.state == "adopted" {
        (
            "Adopted migration",
            format!("the adopted checkpoint ends with it ({id}ledger position {position})"),
            " and every migration the checkpoint covers",
        )
    } else {
        (
            "Applied migration",
            format!("{id}ledger position {position}"),
            "",
        )
    };
    let more = match others {
        0 => String::new(),
        1 => " One more applied migration is missing too.".to_owned(),
        n => format!(" {n} more applied migrations are missing too."),
    };
    history_refusal(
        "DB_HISTORY_MISSING",
        name,
        format!("{what} {name} is missing from {dir}/ ({at}).{more}"),
        format!("Restore {dir}/{name}{adopted}, with a .toml sidecar where it had one, to the exact bytes that were applied, for example from version control. Applied migrations stay in {dir}/ under their applied names; never delete or rewrite ledger rows to match."),
    )
}
/// The ID the ledger records last, the highest applied or adopted ID.
fn last_recorded_id(history: &History) -> Option<u64> {
    history
        .entries
        .iter()
        .filter_map(|e| ledger_id(e)?.parse().ok())
        .max()
}
/// The first ID above `last` that no captured migration uses, written as wide
/// as `name`'s ID, with `name`'s description: what an unapplied migration
/// numbered below `last` can be renamed to.
fn renumbered(inputs: &CapturedInputs, name: &str, last: u64) -> Option<String> {
    let (digits, rest) = name.split_once('_')?;
    let mut id = last.checked_add(1)?;
    while inputs.migrations.iter().any(|m| m.id == id) {
        id = id.checked_add(1)?;
    }
    Some(format!("{id:0width$}_{rest}", width = digits.len()))
}
/// Advice for an unapplied `migration` numbered below `last`, the ID the
/// ledger records last, as `DB_DUPLICATE_ID` advises (#423). The loader
/// pairs a sidecar with its migration by stem, so a sidecar is renamed with
/// it (CodeRabbit, report.rs:234).
fn renumber_advice(inputs: &CapturedInputs, migration: &Migration, last: u64) -> String {
    let dir = &inputs.config.migrations_dir;
    let name = &migration.name;
    let sidecar = |name: &str| {
        let stem = name.strip_suffix(".sql").unwrap_or(name);
        format!("{dir}/{stem}.toml")
    };
    let (what, keep) = match migration.metadata {
        Some(_) => (
            format!("{dir}/{name} and its sidecar {}", sidecar(name)),
            ", keeping one stem for both",
        ),
        None => (format!("{dir}/{name}"), ""),
    };
    let example = renumbered(inputs, name, last)
        .map(|new| match migration.metadata {
            Some(_) => format!(", such as {dir}/{new} and {}", sidecar(&new)),
            None => format!(", such as {dir}/{new}"),
        })
        .unwrap_or_default();
    format!("This target has not applied {name}, but it is numbered below {last}, the last ID in the target's ledger, and migrations run in ID order. Rename {what} to an unused ID above {last}{keep}{example}, as for DB_DUPLICATE_ID, if no other target has applied it yet; never renumber a migration that has been applied anywhere.")
}
/// `DB_HISTORY_DIVERGENT` at ledger position `position`, where the ledger
/// records `entry` and the captured history has `migration` (#423).
fn divergent(
    history: &History,
    inputs: &CapturedInputs,
    entry: &HistoryEntry,
    migration: &Migration,
    position: usize,
) -> Diagnostic {
    let dir = &inputs.config.migrations_dir;
    let message = format!(
        "Applied history diverges at ledger position {position}: the ledger records {} there, but position {position} in {dir}/ is now {} (ID {}).",
        recorded(entry), migration.name, migration.id
    );
    let applied = |name: &str| history.entries.iter().any(|e| e.name == name);
    // The ledger's ID under another name is an applied migration renamed,
    // not one merged in below the last applied ID: renumbering it would
    // make SQL the target already ran pending again.
    let renamed = history
        .entries
        .iter()
        .any(|e| ledger_id(e).and_then(|id| id.parse::<u64>().ok()) == Some(migration.id));
    let last = last_recorded_id(history);
    if let Some(last) =
        last.filter(|last| migration.id < *last && !applied(&migration.name) && !renamed)
    {
        let advice = renumber_advice(inputs, migration, last);
        return history_refusal("DB_HISTORY_DIVERGENT", &migration.name, message, advice);
    }
    match ledger_name(entry) {
        Some(name) if !inputs.migrations.iter().any(|m| m.name == name) => {
            let advice = format!("{name} was applied under that name and is no longer in {dir}/. If {} is that migration renamed, rename it back to {name}; otherwise restore {dir}/{name} with its exact applied bytes, for example from version control. Applied migrations are never renamed or renumbered.", migration.name);
            history_refusal("DB_HISTORY_DIVERGENT", name, message, advice)
        }
        _ => {
            let mut d = Diagnostic::new("DB_HISTORY_DIVERGENT", &message);
            d.source = Some(migration.name.clone());
            d
        }
    }
}
/// The checkpoint `entry` covers the first `count` captured migrations, but
/// the `count`-th is no longer the one it ends with (#423).
fn divergent_checkpoint(
    history: &History,
    inputs: &CapturedInputs,
    entry: &HistoryEntry,
    count: usize,
) -> Diagnostic {
    let dir = &inputs.config.migrations_dir;
    let now = &inputs.migrations[count - 1];
    let mut message = format!(
        "The adopted checkpoint covers the first {count} migrations, through {}, but position {count} in {dir}/ is now {} (ID {}).",
        recorded(entry), now.name, now.id
    );
    let name = ledger_name(entry);
    let moved = name.and_then(|name| inputs.migrations.iter().position(|m| m.name == name));
    let (Some(name), Some(id)) = (name, ledger_id(entry)) else {
        let mut d = Diagnostic::new("DB_HISTORY_DIVERGENT", &message);
        d.source = Some(now.name.clone());
        return d;
    };
    match (moved, last_recorded_id(history)) {
        (Some(index), Some(last)) if index >= count => {
            let more = index + 1 - count;
            // The file at the checkpoint's position can be a covered one, so
            // the checkpoint's checksum decides: one added migration is the
            // one without which the files through `name` give that checksum
            // (CodeRabbit 4656058147).
            let added = (more == 1)
                .then(|| {
                    let summaries: Vec<Value> = inputs.migrations[..=index]
                        .iter()
                        .map(Migration::summary)
                        .collect();
                    (0..index).find(|&skip| {
                        let rest: Vec<Value> = summaries
                            .iter()
                            .enumerate()
                            .filter(|(i, _)| *i != skip)
                            .map(|(_, s)| s.clone())
                            .collect();
                        baseline_checksum(&rest) == entry.checksum
                    })
                })
                .flatten()
                .map(|skip| &inputs.migrations[skip]);
            if let Some(added) = added {
                message.push_str(&format!(
                    " {name} is now at position {}: {} (ID {}) was added after the adoption, numbered below {id}.",
                    index + 1,
                    added.name,
                    added.id,
                ));
                let advice = renumber_advice(inputs, added, last);
                return history_refusal("DB_HISTORY_DIVERGENT", &added.name, message, advice);
            }
            message.push_str(&format!(
                " {name} is now at position {}: {dir}/ holds {more} more migration(s) numbered below {id} than the checkpoint covers.",
                index + 1,
            ));
            let advice = format!("Compare {dir}/ with the release that was adopted. Keep the {count} migrations the checkpoint covers, through {name}, under the names and with the exact bytes they were adopted with. Rename each migration numbered below {id} that was added after the adoption to an unused ID above {last}, and its .toml sidecar, where it has one, to the same new stem, if no other target has applied it yet; never rename or renumber a migration that has been adopted or applied anywhere.");
            history_refusal("DB_HISTORY_DIVERGENT", name, message, advice)
        }
        (None, _) => {
            let advice = format!("Restore {dir}/{name} and every migration before it that the checkpoint covers, with their exact adopted bytes, for example from version control; adopted migrations are never removed, renamed or renumbered.");
            history_refusal("DB_HISTORY_DIVERGENT", name, message, advice)
        }
        _ => {
            let mut d = Diagnostic::new("DB_HISTORY_DIVERGENT", &message);
            d.source = Some(name.to_owned());
            d
        }
    }
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
    let dir = &inputs.config.migrations_dir;
    if history.entries.len() > inputs.migrations.len() {
        // Name the first recorded migration that is not captured (#423).
        let captured = |e: &&HistoryEntry| inputs.migrations.iter().any(|m| m.name == e.name);
        let mut missing = history.entries.iter().filter(|e| !captured(e));
        return Err(match missing.next() {
            Some(entry) => missing_migration(dir, entry, missing.count()),
            None => Diagnostic::new(
                "DB_HISTORY_MISSING",
                "Applied history contains migrations missing from the captured inventory.",
            ),
        });
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
                    // A checkpoint beyond the captured migrations: name the
                    // one it ends with when that is gone (#423).
                    if entry.position > 0 && !inputs.migrations.iter().any(|m| m.name == entry.name) {
                        return missing_migration(dir, entry, 0);
                    }
                    let mut d = Diagnostic::new(
                        "DB_HISTORY_MISSING",
                        &format!(
                            "The adopted checkpoint covers {} migrations, through {}, but {dir}/ holds {}.",
                            entry.position,
                            recorded(entry),
                            inputs.migrations.len()
                        ),
                    );
                    d.source = ledger_name(entry).map(str::to_owned);
                    d
                })?;
            let expected = inputs.checkpoint(count)?;
            if entry.id != expected.id || entry.name != expected.name {
                return Err(divergent_checkpoint(history, inputs, entry, count));
            }
            if entry.checksum != expected.checksum {
                // The ledger keeps one checksum for the whole covered prefix.
                let first = &inputs.migrations[0].name;
                return Err(history_refusal(
                    "DB_HISTORY_EDITED",
                    &expected.name,
                    format!(
                        "A migration the adopted checkpoint covers has changed since the adoption: the checkpoint covers {count} migrations, {first} through {}, and records {} for them, but they now hash to checksum {}. The checkpoint has one checksum for all of them, so it cannot say which file changed.",
                        expected.name, ledger_checksum(entry), expected.checksum
                    ),
                    format!("Restore the migrations from {dir}/{first} through {dir}/{}, with their .toml sidecars, to the exact bytes they had when the target was adopted, for example by comparing each with the version control revision that was adopted; line endings and whitespace count. Put any further change in a new migration (`ntnt db new <name>`).", expected.name),
                ));
            }
            covered = count;
            continue;
        }
        let migration = inputs.migrations.get(covered).ok_or_else(|| {
            let others = history.entries.len() - i - 1;
            missing_migration(dir, entry, others)
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
        if entry.position != covered as i64 + 1 {
            let mut d = Diagnostic::new("DB_HISTORY_DIVERGENT",&format!("Applied ledger is not the exact ordered prefix of this inventory: the ledger records {} at position {} where position {} is expected.", recorded(entry), entry.position, covered + 1));
            d.source = ledger_name(entry).map(str::to_owned);
            return Err(d);
        }
        if entry.id != migration.id.to_string() || entry.name != migration.name {
            return Err(divergent(history, inputs, entry, migration, covered + 1));
        }
        if entry.checksum != migration.checksum {
            let name = &migration.name;
            return Err(history_refusal(
                "DB_HISTORY_EDITED",
                name,
                format!(
                    "Applied migration {name} has changed since it was applied: the ledger records {}, but its SQL and sidecar bytes now hash to checksum {}.",
                    ledger_checksum(entry), migration.checksum
                ),
                format!("Restore {dir}/{name}, with its .toml sidecar if it has one, to the exact bytes that were applied, for example from version control; line endings and whitespace count, so check what an editor or git core.autocrlf changed. Put any further change in a new migration (`ntnt db new <name>`); never rewrite the ledger's checksum."),
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
    /// PostgreSQL: every object the replayed migrations created outside the
    /// managed scope, at any step, by catalog key. The target is inspected
    /// for these (`outside` in `catalog`), and a plan records them.
    outside: std::collections::BTreeSet<String>,
}

/// Replay the first `count` migrations on a prepared scratch, inspecting the
/// catalog before the first and after each one: an object outside coverage
/// that exists only between two migrations is reported against the migration
/// that made it. Each state is inspected once and fingerprinted only for a
/// plan; the catalog itself is kept only at `reference` and, for a plan, at
/// the end. Each state's inspection includes the objects the history has
/// created outside the managed scope by then, as replay finds them.
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
            // PostgreSQL scratch also refuses a migration that acts on _ntnt
            // or a system schema here (DB_OUTSIDE_SCOPE).
            scratch
                .execute_captured(
                    migration,
                    schema,
                    super::inventory::Limits::default().statement(),
                )
                .await?;
        }
        // Scratch holds nothing outside the managed scope it was not prepared
        // with, so what is there now is this history's work, however it got
        // there (a qualified name, search_path, a DO block, SQL NTNT cannot
        // parse). Nothing before the first migration.
        let outside = if migration.is_some() {
            scratch.outside_objects(schema, extra).await?
        } else {
            vec![]
        };
        replayed.outside.extend(outside.iter().cloned());
        let keep = index == reference || (plan && index == count);
        let (uncovered, catalog, fingerprint) = if keep {
            let catalog = scratch.catalog(schema, extra, &outside, role).await?;
            let fingerprint = plan.then(|| digests.digest(&catalog));
            (catalog.uncovered.clone(), Some(catalog), fingerprint)
        } else if plan {
            let (uncovered, fingerprint) = scratch
                .fingerprint(schema, extra, &outside, role, &mut digests)
                .await?;
            (uncovered, None, Some(fingerprint))
        } else {
            (
                scratch.coverage(schema, extra, &outside, role).await?,
                None,
                None,
            )
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
    let name = &inputs.endpoint.connection_secret;
    let secret = crate::stdlib::secrets::require_from_manifest(name, &inputs.document)
        .map_err(|error| unresolved_secret("target", name, &inputs.config.engine, error))?;
    let (identity, catalog, history) = observe(&inputs, &secret, &[]).await?;
    inspect_observed(
        &inputs,
        command,
        identity,
        catalog,
        history,
        adopt_through,
        &secret,
    )
    .await
}
/// One read-only snapshot of the target: its identity, the catalog of the
/// managed scope and of the `outside` objects (see `Adapter::catalog`), and
/// its history, kept as a result so that a history error becomes a report
/// diagnostic alongside the catalog. The session is closed before this
/// returns: no target lock or idle transaction survives into scratch replay,
/// and an idle-session disconnect cannot erase a completed report.
async fn observe(
    inputs: &CapturedInputs,
    secret: &crate::interpreter::SecretValue,
    outside: &[String],
) -> Result<(Identity, Catalog, Result<History>)> {
    let target = Adapter::connect(
        secret,
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
                outside,
                &inputs.endpoint.role,
            )
            .await?;
        let history = target.history().await;
        Ok::<_, Diagnostic>((identity, catalog, history))
    }
    .await;
    let cleanup = target.rollback().await;
    drop(target);
    match (observed, cleanup) {
        (Ok(observed), Ok(())) => Ok(observed),
        (Err(d), _) | (_, Err(d)) => Err(d),
    }
}
/// The catalog of a second snapshot of the target (`observe`) when it shows
/// the identity and history the first one did. Otherwise nothing may be
/// compared: `DB_TARGET_CHANGED` if either changed or its history could not
/// be read, or the error the snapshot itself failed with.
fn second_snapshot(
    identity: &Identity,
    history: &History,
    second: Result<(Identity, Catalog, Result<History>)>,
) -> Result<Catalog> {
    match second {
        Ok((again, catalog, Ok(later))) if json!(&again) == json!(identity) && later == *history => {
            Ok(catalog)
        }
        Ok(_) => Err(Diagnostic::new("DB_TARGET_CHANGED", "The target's identity or migration history changed between its two inspections; nothing was compared.")),
        Err(d) => Err(d),
    }
}
/// Combine captured target facts with applied-prefix and optional candidate replay.
/// Plan bindings retain logical labels, physical identity and exact source/policy digests.
/// Missing or incomplete evidence stays unknown; no target connection remains open here.
/// When replay finds objects the history creates outside the managed scope,
/// a second snapshot through `target_secret` inspects them on the target.
async fn inspect_observed(
    inputs: &CapturedInputs,
    command: &str,
    identity: Identity,
    mut catalog: Catalog,
    history: Result<History>,
    adopt_through: Option<u64>,
    target_secret: &crate::interpreter::SecretValue,
) -> Result<Value> {
    let inventory: Vec<_> = inputs
        .migrations
        .iter()
        .map(|m| {
            let mut v = m.summary();
            v["analysis"] = json!(analyze_in_scope(
                &inputs.config.engine,
                &m.sql,
                &inputs.config.schema,
                &inputs.history_schemas
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
    // The target's coverage diagnostic goes here, before replay's, once its
    // catalog is final (below).
    let coverage_at = report["diagnostics"].as_array().map_or(0, Vec::len);
    let mut replayed = Replayed::default();
    if let Some(scratch_config) = &inputs.config.scratch {
        let mut cleanup_failure = None;
        let replay = async {
            let name = &scratch_config.endpoint.connection_secret;
            let secret = crate::stdlib::secrets::require_from_manifest(name, &inputs.document)
                .map_err(|error| {
                    unresolved_secret("scratch", name, &inputs.config.engine, error)
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
    // Inspect on the target what replay found the history creates outside
    // the managed scope. The first snapshot could not know these objects, so
    // a second one reads the whole catalog with them; it must show the same
    // target and history, or nothing is compared.
    let outside: Vec<String> = replayed.outside.iter().cloned().collect();
    if !outside.is_empty() {
        report["target"]["outside_objects"] = json!(outside);
        let second = observe(inputs, target_secret, &outside).await;
        match second_snapshot(&identity, &history, second) {
            Ok(observed) => {
                catalog = observed;
                report["catalog"] = json!(&catalog);
            }
            Err(d) => {
                diagnostic(&mut report, d);
                replayed.expected = None;
            }
        }
    }
    if !catalog.uncovered.is_empty() {
        report["diagnostics"].as_array_mut().unwrap().insert(
            coverage_at,
            json!(Diagnostic::new("DB_COVERAGE_INCOMPLETE","Observed objects or privileges exceed declared catalog coverage; equality cannot be asserted.").with_objects(&catalog.uncovered)),
        );
    }
    let Replayed {
        fingerprints,
        expected,
        candidate,
        outside: _,
    } = replayed;
    if let Some(expected) = &expected {
        report["drift"] = compare(&catalog, expected);
        if adoption.is_some() {
            report["drift"]["reference"] = json!("requested_baseline");
        }
    }
    report["outcome"] = report["drift"]["outcome"].clone();
    if report["drift"]["outcome"] == "fail" {
        let mut drift = Diagnostic::new("DB_DRIFT","Observed managed catalog differs from the replayed applied prefix; inspect object-level differences.");
        // An adoption refused for tables the history does not create says
        // how to retire an application's previous ledger (#431).
        if let Some(advice) = adoption.as_ref().and_then(|action| {
            super::adoption::retire_ledger_advice(
                &drift.next_action,
                &inputs.config.engine,
                &inputs.config.schema,
                &inputs.history_schemas,
                &action.through_id,
                &report["drift"]["differences"],
            )
        }) {
            drift.next_action = advice;
        }
        diagnostic(&mut report, drift);
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
            v["analysis"] = json!(analyze_in_scope(
                &inputs.config.engine,
                &m.sql,
                &inputs.config.schema,
                &inputs.history_schemas
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
        // So are the objects outside the managed scope it inspects on the
        // target: everything replay found the history creates there.
        if !outside.is_empty() {
            execution["outside_objects"] = json!(outside);
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
            .prepare_scratch(&identity, scratch_endpoint, "main")
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
                .prepare_scratch(&identity, scratch_endpoint, "main")
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
    /// is exact for n and 2n; it used to grow with the square of n. That
    /// holds for histories dense in foreign keys too: a new table that
    /// references the last of a chain of ON DELETE CASCADE keys, a hub that
    /// every table references (each with an index and a trigger), and a DAG
    /// of CASCADE and SET NULL keys. The DELETE of every table above a new
    /// one now cascades into it, yet none of them is compiled again. Tables
    /// that each reference themselves certify their own programs, which are
    /// reused as long as nothing in them changed.
    #[test]
    fn replay_inspection_work_grows_linearly_with_history() {
        const N: u64 = 8;
        let tables = |i: usize| format!("CREATE TABLE t{i}(id INTEGER PRIMARY KEY);");
        // An index and a view per table as well.
        let views = |i: usize| {
            format!("CREATE TABLE t{i}(id INTEGER PRIMARY KEY, v TEXT); CREATE INDEX t{i}_v ON t{i}(v); CREATE VIEW v{i} AS SELECT id, v FROM t{i};")
        };
        let chain = |i: usize| {
            match i {
            1 => "CREATE TABLE t1(id INTEGER PRIMARY KEY);".to_owned(),
            _ => format!(
                "CREATE TABLE t{i}(id INTEGER PRIMARY KEY, prev INTEGER REFERENCES t{}(id) ON DELETE CASCADE);",
                i - 1
            ),
        }
        };
        let hub = |i: usize| {
            match i {
            1 => "CREATE TABLE users(id INTEGER PRIMARY KEY, email TEXT UNIQUE); CREATE TABLE audit(id INTEGER PRIMARY KEY, what TEXT);".to_owned(),
            _ => format!(
                "CREATE TABLE t{i}(id INTEGER PRIMARY KEY, user_id INTEGER REFERENCES users(id) ON DELETE CASCADE, v TEXT); CREATE INDEX t{i}_user ON t{i}(user_id); CREATE TRIGGER t{i}_audit AFTER UPDATE OF v ON t{i} BEGIN INSERT INTO audit(what) VALUES ('t{i}'); END;"
            ),
        }
        };
        let dag = |i: usize| {
            match i {
            1 | 2 => format!("CREATE TABLE t{i}(id INTEGER PRIMARY KEY);"),
            _ => format!(
                "CREATE TABLE t{i}(id INTEGER PRIMARY KEY, a INTEGER REFERENCES t{}(id) ON DELETE CASCADE, b INTEGER REFERENCES t{}(id) ON DELETE SET NULL);",
                1 + (i * 7) % (i - 1),
                1 + (i * 13) % (i - 1)
            ),
        }
        };
        let trees = |i: usize| {
            format!("CREATE TABLE t{i}(id INTEGER PRIMARY KEY, parent INTEGER REFERENCES t{i}(id) ON DELETE CASCADE);")
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
            for (shape, history) in [
                ("chain", &chain as &dyn Fn(usize) -> String),
                ("dag", &dag),
                ("trees", &trees),
            ] {
                let work = replay_work(n as usize, history);
                assert_eq!(work.catalogs, n + 1, "{shape} {work:?}");
                assert_eq!(work.metadata_queries, 2 * n, "{shape} {work:?}");
                assert_eq!(work.programs_compiled, 6 * n, "{shape} {work:?}");
                assert_eq!(work.allowlists, n, "{shape} {work:?}");
            }
            let work = replay_work(n as usize, hub);
            assert_eq!(work.catalogs, n + 1, "{work:?}");
            // users (with the index of its UNIQUE email) and audit, then a
            // table and its index per migration.
            assert_eq!(work.metadata_queries, 4 * n + 2, "{work:?}");
            assert_eq!(work.programs_compiled, 6 * n + 6, "{work:?}");
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
    /// state is inspected too. `depth_limit` lowers SQLite's limit on nested
    /// trigger programs for the session. Returns the migrations that failed
    /// and, after each migration, what was outside coverage (`None` where
    /// inspection failed).
    fn assert_incremental_inspection_is_exact(
        scripts: &[(String, Option<&str>)],
        restart: usize,
        depth_limit: Option<i32>,
    ) -> (Vec<String>, Vec<Option<Vec<String>>>) {
        let project = sqlite_project_with(scripts);
        let inputs = &project.inputs;
        current_thread().block_on(async {
            let (identity, scratch) = prepared_scratch(&project).await;
            let Adapter::Sqlite(db) = &scratch else {
                unreachable!("a SQLite scratch")
            };
            if let Some(limit) = depth_limit {
                db.set_trigger_depth_limit(limit).await.unwrap();
            }
            let (mut failed, mut states) = (Vec::new(), Vec::new());
            for (index, migration) in inputs.migrations.iter().enumerate() {
                if index == restart {
                    // A rolled-back replay leaves the session's cache behind.
                    scratch.rollback().await.unwrap();
                    let scratch_endpoint = &inputs.config.scratch.as_ref().unwrap().endpoint;
                    scratch
                        .prepare_scratch(&identity, scratch_endpoint, "main")
                        .await
                        .unwrap();
                }
                let outcome = scratch
                    .execute_captured(
                        migration,
                        "main",
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
                        0 => scratch.catalog("main", &[], &[], "").await.map(|catalog| {
                            let digest = json_digest(&catalog);
                            (json!(catalog), catalog.uncovered.clone(), digest)
                        }),
                        1 => scratch
                            .fingerprint("main", &[], &[], "", &mut CatalogDigests::default())
                            .await
                            .map(|(uncovered, fingerprint)| {
                                let catalog = fresh.as_ref().map_or(Value::Null, |c| json!(c));
                                (catalog, uncovered, fingerprint)
                            }),
                        _ => scratch
                            .coverage("main", &[], &[], "")
                            .await
                            .map(|uncovered| {
                                let catalog = fresh.as_ref().map_or(Value::Null, |c| json!(c));
                                let digest = fresh.as_ref().map_or(String::new(), json_digest);
                                (catalog, uncovered, digest)
                            }),
                    };
                    assert_eq!(reported.map_err(error), expect(&fresh), "{at}, call {call}");
                }
                states.push(fresh.ok().map(|catalog| catalog.uncovered));
            }
            scratch.rollback().await.unwrap();
            (failed, states)
        })
    }
    /// How many of these states were outside coverage.
    fn uncovered_states(states: &[Option<Vec<String>>]) -> usize {
        states
            .iter()
            .filter(|state| state.as_ref().is_some_and(|u| !u.is_empty()))
            .count()
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
            // What a trigger writes is written: tables referencing it count.
            "CREATE TABLE tt(id INTEGER PRIMARY KEY); CREATE TABLE uu(id INTEGER PRIMARY KEY); CREATE TRIGGER tt_uu AFTER DELETE ON tt BEGIN DELETE FROM uu WHERE id = old.id; END;".to_owned(),
            format!("CREATE TABLE cc(id INTEGER PRIMARY KEY, uu_id INTEGER REFERENCES uu(id) ON DELETE CASCADE); CREATE TRIGGER cc_gone AFTER DELETE ON cc BEGIN DELETE FROM {ledger}; END;"),
            "DROP TRIGGER cc_gone;".to_owned(),
        ]
        .into_iter()
        .enumerate()
        .map(|(i, sql)| (sql, (i == 28).then_some("accounts")))
        .collect();
        let (failed, states) = assert_incremental_inspection_is_exact(&written, 29, None);
        let uncovered = uncovered_states(&states);
        // Only the two that leave m's foreign key without a unique parent key
        // fail, at the foreign-key check, and several states are outside
        // coverage.
        let mismatch = "DB_SQLITE: foreign key mismatch - \"m\" referencing \"n\" (SQLITE_ERROR).";
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
        let (failed, states) = assert_incremental_inspection_is_exact(&generated, 120, None);
        let uncovered = uncovered_states(&states);
        // Most of the generated history runs (replay refuses some drops that
        // would run ON DELETE actions, and what follows them can fail), and
        // both covered and uncovered states occur.
        assert!(
            failed.len() <= 60 && (24..=216).contains(&uncovered),
            "{failed:?} {uncovered}"
        );
    }

    /// Captured inputs of a SQLite project whose migrations are `files`
    /// (name, SQL), for checking history without a database.
    fn named_inputs(files: &[(&str, &str)]) -> (tempfile::TempDir, CapturedInputs) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir(root.join("migrations")).unwrap();
        std::fs::write(
            root.join("ntnt.toml"),
            "[databases.app]\nengine = \"sqlite\"\nschema = \"main\"\nmigrations_dir = \"migrations\"\n\
             [databases.app.targets.test]\nconnection_secret = \"DB_TARGET\"\n[secrets.DB_TARGET]\n",
        )
        .unwrap();
        for (name, sql) in files {
            std::fs::write(root.join("migrations").join(name), sql).unwrap();
        }
        let inputs = CapturedInputs::load(&root.join("ntnt.toml"), "test", None).unwrap();
        (dir, inputs)
    }
    fn entry(
        position: usize,
        m: &super::super::inventory::Migration,
    ) -> super::super::model::HistoryEntry {
        super::super::model::HistoryEntry {
            position: position as i64,
            id: m.id.to_string(),
            name: m.name.clone(),
            checksum: m.checksum.clone(),
            state: "applied".into(),
        }
    }
    /// The ledger of a target that adopted the first `adopted` of `inputs`'
    /// migrations (none: no checkpoint) and then applied the rest up to `through`.
    fn ledger(inputs: &CapturedInputs, adopted: usize, through: usize) -> History {
        let mut entries: Vec<_> = (adopted > 0)
            .then(|| inputs.checkpoint(adopted).unwrap())
            .into_iter()
            .collect();
        entries.extend(
            inputs.migrations[adopted..through]
                .iter()
                .enumerate()
                .map(|(i, m)| entry(adopted + i + 1, m)),
        );
        History {
            present: true,
            entries,
            scope: Some(json!({"database":"app","schema":"main"})),
        }
    }
    fn refusal(history: &History, inputs: &CapturedInputs) -> Diagnostic {
        prefix_with_attempt(history, inputs).unwrap_err()
    }
    const A: (&str, &str) = ("0010_a.sql", "CREATE TABLE a(id INTEGER);\n");
    const B: (&str, &str) = ("0020_b.sql", "CREATE TABLE b(id INTEGER);\n");
    const C: (&str, &str) = ("0030_c.sql", "CREATE TABLE c(id INTEGER);\n");
    const D: (&str, &str) = ("0040_d.sql", "CREATE TABLE d(id INTEGER);\n");
    /// Nothing a migration holds appears in a history diagnostic.
    fn shows_no_sql(d: &Diagnostic) {
        let text = format!("{} {}", d.message, d.next_action);
        assert!(!text.contains("CREATE TABLE"), "{text}");
    }

    /// #423 (sq-13, pg-07, cli-15): an applied migration whose bytes changed
    /// is named, with the checksum it was applied with and its checksum now.
    #[test]
    fn an_edited_applied_migration_is_named_with_both_checksums() {
        let (_before, applied) = named_inputs(&[A, B, C]);
        let history = ledger(&applied, 0, 3);
        // An editor or git core.autocrlf rewrote its line endings.
        let (_after, now) =
            named_inputs(&[A, ("0020_b.sql", "CREATE TABLE b(id INTEGER);\r\n"), C]);
        let d = refusal(&history, &now);
        assert_eq!(d.code, "DB_HISTORY_EDITED");
        assert_eq!(d.source.as_deref(), Some("0020_b.sql"), "{d:?}");
        for shown in [
            "0020_b.sql",
            &applied.migrations[1].checksum,
            &now.migrations[1].checksum,
        ] {
            assert!(d.message.contains(shown), "{shown}: {}", d.message);
        }
        assert!(
            d.next_action.contains("migrations/0020_b.sql"),
            "{}",
            d.next_action
        );
        assert!(d.next_action.contains("new migration"), "{}", d.next_action);
        shows_no_sql(&d);
    }

    /// #423: an applied migration missing from migrations/ is named, wherever
    /// it was in the history.
    #[test]
    fn a_missing_applied_migration_is_named() {
        let (_before, applied) = named_inputs(&[A, B, C]);
        let history = ledger(&applied, 0, 3);
        for (files, missing) in [(vec![A, B], "0030_c.sql"), (vec![A, C], "0020_b.sql")] {
            let (_after, now) = named_inputs(&files);
            let d = refusal(&history, &now);
            assert_eq!(d.code, "DB_HISTORY_MISSING");
            assert_eq!(d.source.as_deref(), Some(missing), "{d:?}");
            assert!(
                d.message
                    .contains(&format!("{missing} is missing from migrations/")),
                "{}",
                d.message
            );
            assert!(
                d.next_action.contains(&format!("migrations/{missing}")),
                "{}",
                d.next_action
            );
            shows_no_sql(&d);
        }
        // After a checkpoint, an applied migration past the end of migrations/.
        let (_before, applied) = named_inputs(&[A, B, C, D]);
        let history = ledger(&applied, 2, 4);
        let (_after, now) = named_inputs(&[A, B, C]);
        let d = refusal(&history, &now);
        assert_eq!(d.code, "DB_HISTORY_MISSING");
        assert_eq!(d.source.as_deref(), Some("0040_d.sql"), "{d:?}");
    }

    /// #423 (sq-13): a migration numbered below the last applied one, such as
    /// one merged from another branch, is named at the first ledger position
    /// that differs, and the advice is to renumber it above that ID, as for
    /// DB_DUPLICATE_ID. The suggested name is free.
    #[test]
    fn an_unapplied_migration_below_the_last_applied_id_is_renumbered_above_it() {
        let (_before, applied) = named_inputs(&[A, C, D]);
        let history = ledger(&applied, 0, 2);
        let inserted = ("0020_b.sql", "CREATE TABLE b(id INTEGER);\n");
        let (_after, now) = named_inputs(&[A, inserted, C, D]);
        let d = refusal(&history, &now);
        assert_eq!(d.code, "DB_HISTORY_DIVERGENT");
        assert_eq!(d.source.as_deref(), Some("0020_b.sql"), "{d:?}");
        for shown in ["position 2", "0030_c.sql", "0020_b.sql"] {
            assert!(d.message.contains(shown), "{shown}: {}", d.message);
        }
        // 31 is the first free ID above 30, the last applied ID (40 is pending).
        assert!(d.next_action.contains("above 30"), "{}", d.next_action);
        assert!(d.next_action.contains("0031_b.sql"), "{}", d.next_action);
        shows_no_sql(&d);
        // Following the advice gives a valid history with two pending.
        let (_fixed, fixed) =
            named_inputs(&[A, C, ("0031_b.sql", "CREATE TABLE b(id INTEGER);\n"), D]);
        assert_eq!(prefix_with_attempt(&history, &fixed).unwrap(), (2, None));
    }

    /// CodeRabbit (report.rs:234): the loader pairs a sidecar with its
    /// migration by stem, so the renumber advice for a migration that has one
    /// renames both to one new stem; followed, nothing is orphaned.
    #[test]
    fn a_renumbered_migration_takes_its_sidecar_along() {
        let (_before, applied) = named_inputs(&[A, C, D]);
        let history = ledger(&applied, 0, 2);
        let (sql, toml) = ("CREATE TABLE b(id INTEGER);\n", "intent_ids = [\"b\"]\n");
        let (_after, now) = named_inputs(&[A, ("0020_b.sql", sql), ("0020_b.toml", toml), C, D]);
        let d = refusal(&history, &now);
        assert_eq!(d.code, "DB_HISTORY_DIVERGENT");
        assert_eq!(d.source.as_deref(), Some("0020_b.sql"), "{d:?}");
        assert!(
            d.next_action.contains("Rename migrations/0020_b.sql and its sidecar migrations/0020_b.toml to an unused ID above 30, keeping one stem for both, such as migrations/0031_b.sql and migrations/0031_b.toml, as for DB_DUPLICATE_ID"),
            "{}",
            d.next_action
        );
        // Following the advice: the sidecar stays paired with its migration.
        let (_fixed, fixed) = named_inputs(&[A, C, ("0031_b.sql", sql), ("0031_b.toml", toml), D]);
        assert!(fixed.migrations[2].metadata.is_some());
        assert_eq!(prefix_with_attempt(&history, &fixed).unwrap(), (2, None));
        // The advice for a migration without a sidecar names none.
        let (_after, now) = named_inputs(&[A, ("0020_b.sql", sql), C, D]);
        let d = refusal(&history, &now);
        assert!(!d.next_action.contains(".toml"), "{}", d.next_action);
    }

    /// #423: an applied migration renamed in migrations/ is named under the
    /// name it was applied with.
    #[test]
    fn a_renamed_applied_migration_is_named_as_applied() {
        let (_before, applied) = named_inputs(&[A, B]);
        let history = ledger(&applied, 0, 2);
        let (_after, now) = named_inputs(&[A, ("0020_bee.sql", B.1)]);
        let d = refusal(&history, &now);
        assert_eq!(d.code, "DB_HISTORY_DIVERGENT");
        assert_eq!(d.source.as_deref(), Some("0020_b.sql"), "{d:?}");
        for shown in ["position 2", "0020_b.sql", "0020_bee.sql"] {
            assert!(d.message.contains(shown), "{shown}: {}", d.message);
        }
        assert!(d.next_action.contains("0020_b.sql"), "{}", d.next_action);
    }

    /// #423 (review r1): an applied migration renamed with its ID kept is a
    /// rename even when later migrations are applied too, so the advice is to
    /// rename it back, never to renumber SQL the target has already run.
    #[test]
    fn a_renamed_applied_migration_below_the_last_applied_id_is_renamed_back() {
        let (_before, applied) = named_inputs(&[A, B, C]);
        let history = ledger(&applied, 0, 3);
        let (_after, now) = named_inputs(&[A, ("0020_bee.sql", B.1), C]);
        let d = refusal(&history, &now);
        assert_eq!(d.code, "DB_HISTORY_DIVERGENT");
        assert_eq!(d.source.as_deref(), Some("0020_b.sql"), "{d:?}");
        for shown in ["position 2", "0020_b.sql", "0020_bee.sql"] {
            assert!(d.message.contains(shown), "{shown}: {}", d.message);
        }
        assert!(
            d.next_action.contains(
                "If 0020_bee.sql is that migration renamed, rename it back to 0020_b.sql"
            ),
            "{}",
            d.next_action
        );
        assert!(!d.next_action.contains("above"), "{}", d.next_action);
        shows_no_sql(&d);
        // Following the advice: under its applied name the history matches.
        let (_fixed, fixed) = named_inputs(&[A, B, C]);
        assert_eq!(prefix_with_attempt(&history, &fixed).unwrap(), (3, None));
    }

    /// #423: the adopted-checkpoint variants name the checkpoint's migrations.
    #[test]
    fn adopted_checkpoint_refusals_name_the_covered_migrations() {
        let (_before, adopted) = named_inputs(&[A, B, C]);
        let history = ledger(&adopted, 2, 3);
        // A covered migration changed.
        let (_after, now) = named_inputs(&[("0010_a.sql", "CREATE TABLE a(id INTEGER); \n"), B, C]);
        let d = refusal(&history, &now);
        assert_eq!(d.code, "DB_HISTORY_EDITED");
        assert_eq!(d.source.as_deref(), Some("0020_b.sql"), "{d:?}");
        for shown in [
            "adopted checkpoint",
            "0010_a.sql",
            "0020_b.sql",
            &history.entries[0].checksum,
        ] {
            assert!(d.message.contains(shown), "{shown}: {}", d.message);
        }
        shows_no_sql(&d);
        // A migration numbered below the checkpoint's last ID.
        let (_after, now) =
            named_inputs(&[A, ("0015_x.sql", "CREATE TABLE x(id INTEGER);\n"), B, C]);
        let d = refusal(&history, &now);
        assert_eq!(d.code, "DB_HISTORY_DIVERGENT");
        assert_eq!(d.source.as_deref(), Some("0015_x.sql"), "{d:?}");
        for shown in [
            "adopted checkpoint",
            "0020_b.sql",
            "0015_x.sql",
            "position 2",
        ] {
            assert!(d.message.contains(shown), "{shown}: {}", d.message);
        }
        assert!(d.next_action.contains("above 30"), "{}", d.next_action);
        // The checkpoint's last migration is missing.
        let (_after, now) = named_inputs(&[A]);
        let d = refusal(&ledger(&adopted, 2, 2), &now);
        assert_eq!(d.code, "DB_HISTORY_MISSING");
        assert_eq!(d.source.as_deref(), Some("0020_b.sql"), "{d:?}");
        assert!(
            d.message.contains("0020_b.sql is missing from migrations/"),
            "{}",
            d.message
        );
    }

    /// CodeRabbit 4656058147 (#423): a migration added below every one an
    /// adopted checkpoint covers moves the checkpoint's end, so the file at
    /// its position is a covered one. The checkpoint's checksum tells the
    /// added migration from the covered ones, and the added one is named;
    /// when it cannot, the advice is to compare with the adopted release. A
    /// covered migration is never named to renumber, and following either
    /// piece of advice gives a valid history.
    #[test]
    fn a_migration_added_below_an_adopted_checkpoint_is_told_from_the_covered_ones() {
        let (_before, adopted) = named_inputs(&[A, B, C]);
        let history = ledger(&adopted, 3, 3);
        let x = ("0005_x.sql", "CREATE TABLE x(id INTEGER);\n");
        let (_after, now) = named_inputs(&[x, A, B, C]);
        let d = refusal(&history, &now);
        assert_eq!(d.code, "DB_HISTORY_DIVERGENT");
        assert_eq!(d.source.as_deref(), Some("0005_x.sql"), "{d:?}");
        assert!(
            d.message
                .contains("0005_x.sql (ID 5) was added after the adoption, numbered below 30."),
            "{}",
            d.message
        );
        assert!(
            d.next_action.contains(
                "Rename migrations/0005_x.sql to an unused ID above 30, such as migrations/0031_x.sql"
            ),
            "{}",
            d.next_action
        );
        for covered in [A.0, B.0, C.0] {
            assert!(
                !d.next_action.contains(covered),
                "{covered}: {}",
                d.next_action
            );
        }
        shows_no_sql(&d);
        // Following the advice: the checkpoint matches again, 0031_x.sql pending.
        let (_fixed, fixed) = named_inputs(&[A, B, C, ("0031_x.sql", x.1)]);
        assert_eq!(prefix_with_attempt(&history, &fixed).unwrap(), (3, None));

        // Two added: the checksum cannot say which files are the covered ones.
        let y = ("0015_y.sql", "CREATE TABLE y(id INTEGER);\n");
        let (_after, now) = named_inputs(&[x, A, y, B, C]);
        let d = refusal(&history, &now);
        assert_eq!(d.code, "DB_HISTORY_DIVERGENT");
        assert_eq!(d.source.as_deref(), Some("0030_c.sql"), "{d:?}");
        assert!(
            d.message.contains(
                "0030_c.sql is now at position 5: migrations/ holds 2 more migration(s) numbered below 30 than the checkpoint covers."
            ),
            "{}",
            d.message
        );
        assert!(
            d.next_action
                .starts_with("Compare migrations/ with the release that was adopted."),
            "{}",
            d.next_action
        );
        assert!(
            d.next_action
                .contains("Keep the 3 migrations the checkpoint covers, through 0030_c.sql, under the names and with the exact bytes they were adopted with"),
            "{}",
            d.next_action
        );
        for covered in [A.0, B.0] {
            assert!(
                !d.next_action.contains(covered),
                "{covered}: {}",
                d.next_action
            );
        }
        shows_no_sql(&d);
        // Following it: the two added ones are renamed above 30.
        let (_fixed, fixed) = named_inputs(&[A, B, C, ("0031_x.sql", x.1), ("0032_y.sql", y.1)]);
        assert_eq!(prefix_with_attempt(&history, &fixed).unwrap(), (3, None));
    }

    /// #423: ledger values are shown only in the shape ntnt writes there, so
    /// a tampered ledger cannot put its text into a report.
    #[test]
    fn tampered_ledger_values_are_never_shown() {
        let (_dir, inputs) = named_inputs(&[A, B]);
        let mut history = ledger(&inputs, 0, 2);
        history.entries[1].checksum = "ntnt_private_checksum".into();
        let d = refusal(&history, &inputs);
        assert_eq!(d.code, "DB_HISTORY_EDITED");
        assert!(!format!("{d:?}").contains("ntnt_private"), "{d:?}");
        history.entries[1].name = "ntnt_private_name".into();
        history.entries[1].id = "ntnt_private_id".into();
        let d = refusal(&history, &inputs);
        assert_eq!(d.code, "DB_HISTORY_DIVERGENT");
        assert!(!format!("{d:?}").contains("ntnt_private"), "{d:?}");
        let (_dir, fewer) = named_inputs(&[A]);
        let d = refusal(&history, &fewer);
        assert_eq!(d.code, "DB_HISTORY_MISSING");
        assert!(!format!("{d:?}").contains("ntnt_private"), "{d:?}");
    }

    /// The second snapshot replay's outside objects need is used only when it
    /// shows the target and history the first one did.
    #[test]
    fn a_second_snapshot_is_used_only_for_the_same_target_and_history() {
        use super::super::model::HistoryEntry;
        let identity = Identity::Postgres(super::super::postgres::Identity {
            cluster: "7000000000000000001".into(),
            database_oid: 16384,
            database: "app".into(),
            role: "app".into(),
            version: 180_000,
            settings: json!({"encoding": "UTF8"}),
            authority: json!({"database_owner": "app"}),
            transport: json!({"sslmode": "disable", "tls_version": null, "server_ssl": false}),
        });
        let history = History {
            present: true,
            entries: vec![HistoryEntry {
                position: 1,
                id: "1".into(),
                name: "0001_initial.sql".into(),
                checksum: "c".repeat(64),
                state: "applied".into(),
            }],
            scope: Some(json!({"database": "app", "schema": "app"})),
        };
        let mut catalog = Catalog::default();
        catalog
            .objects
            .insert("public.relation:audit_log".into(), json!({"kind": "r"}));
        let code = |second| {
            second_snapshot(&identity, &history, second)
                .map(|c| json!(c))
                .map_err(|d| d.code)
        };
        let same = || Ok((identity.clone(), catalog.clone(), Ok(history.clone())));
        assert_eq!(code(same()), Ok(json!(catalog)));
        let Identity::Postgres(mut other) = identity.clone() else {
            unreachable!()
        };
        other.database_oid += 1;
        let moved = Ok((
            Identity::Postgres(other),
            catalog.clone(),
            Ok(history.clone()),
        ));
        assert_eq!(code(moved), Err("DB_TARGET_CHANGED".into()));
        let mut later = history.clone();
        later.entries[0].checksum = "d".repeat(64);
        let edited = Ok((identity.clone(), catalog.clone(), Ok(later)));
        assert_eq!(code(edited), Err("DB_TARGET_CHANGED".into()));
        let mut later = history.clone();
        later.entries.push(HistoryEntry {
            position: 2,
            ..history.entries[0].clone()
        });
        let migrated = Ok((identity.clone(), catalog.clone(), Ok(later)));
        assert_eq!(code(migrated), Err("DB_TARGET_CHANGED".into()));
        let unreadable = Ok((
            identity.clone(),
            catalog.clone(),
            Err(failure("DB_LEDGER_UNKNOWN")),
        ));
        assert_eq!(code(unreadable), Err("DB_TARGET_CHANGED".into()));
        assert_eq!(code(Err(failure("DB_TIMEOUT"))), Err("DB_TIMEOUT".into()));
    }

    /// The tables outside coverage in each state, by name. Each state must
    /// have a catalog: one whose inspection failed (None) fails, naming its
    /// migration, so it can never match a covered state.
    fn uncovered_tables(states: &[Option<Vec<String>>]) -> Vec<Vec<String>> {
        states
            .iter()
            .enumerate()
            .map(|(index, state)| {
                let Some(state) = state else {
                    panic!(
                        "after migration {} the inspection returned no catalog",
                        index + 1
                    )
                };
                state
                    .iter()
                    .map(|key| {
                        key.strip_prefix("table:\"")
                            .and_then(|k| k.strip_suffix('"'))
                            .unwrap_or(key)
                            .to_owned()
                    })
                    .collect()
            })
            .collect()
    }

    /// SQLite reads U+FEFF at the start of a token as a space, so a view
    /// whose FROM has one right before its table's name reads that table:
    /// once the table is dropped, the view is outside coverage in a session's
    /// inspection as in a fresh one (review r4654895940).
    #[test]
    fn incremental_inspection_is_exact_for_a_byte_order_mark_before_a_name() {
        let scripts: Vec<(String, Option<&str>)> = [
            "CREATE TABLE t(id INTEGER); CREATE VIEW v AS SELECT id FROM \u{FEFF}t;",
            "DROP TABLE t;",
        ]
        .into_iter()
        .map(|sql| (sql.to_owned(), None))
        .collect();
        let (failed, states) = assert_incremental_inspection_is_exact(&scripts, usize::MAX, None);
        assert_eq!(failed, Vec::<String>::new());
        assert_eq!(uncovered_tables(&states), [vec![], vec!["view:\"v\""]]);
    }

    /// `uncovered_tables` names each state's uncovered tables, `[]` for a
    /// catalog with nothing uncovered.
    #[test]
    fn uncovered_tables_names_what_each_catalog_leaves_uncovered() {
        let states = [
            Some(vec![]),
            Some(vec!["table:\"t\"".to_owned(), "view:\"v\"".to_owned()]),
        ];
        assert_eq!(uncovered_tables(&states), [vec![], vec!["t", "view:\"v\""]]);
    }

    /// A state whose inspection returned no catalog cannot match a covered
    /// one: `uncovered_tables` fails, naming the migration.
    #[test]
    #[should_panic(expected = "after migration 2 the inspection returned no catalog")]
    fn uncovered_tables_fails_for_a_state_without_a_catalog() {
        uncovered_tables(&[Some(vec![]), None, Some(vec![])]);
    }

    /// #421: a table's programs are reused across changes below it in its
    /// foreign-key tree only where compiling them again could not change
    /// what they report. Each step here changes something below an unchanged
    /// table that takes it outside coverage, or brings it back: through
    /// cascades several levels deep, SET NULL, SET DEFAULT (with a function
    /// default) and ON UPDATE CASCADE actions, RESTRICT, a key whose parent
    /// key is not unique, triggers on children that the action fires or does
    /// not fire, a REPLACE conflict on a WITHOUT ROWID parent, a cycle, a
    /// table that references itself, and children dropped again.
    #[test]
    fn incremental_inspection_is_exact_when_foreign_keys_change_below_a_table() {
        let l = "_ntnt_schema_migrations_v2";
        let gone = |table: &str, event: &str| {
            format!("CREATE TRIGGER {table}_t AFTER {event} ON {table} BEGIN DELETE FROM {l}; END;")
        };
        let undo = |table: &str| format!("DROP TRIGGER {table}_t;");
        let steps: Vec<(String, &[&str])> = vec![
            ("CREATE TABLE a(id INTEGER PRIMARY KEY, k TEXT UNIQUE);".into(), &[]),
            ("CREATE TABLE b(id INTEGER PRIMARY KEY, a_id INTEGER REFERENCES a(id) ON DELETE CASCADE);".into(), &[]),
            ("CREATE TABLE c(id INTEGER PRIMARY KEY, b_id INTEGER REFERENCES b(id) ON DELETE CASCADE);".into(), &[]),
            // A cascade three levels below a.
            (format!("CREATE TABLE d(id INTEGER PRIMARY KEY, c_id INTEGER REFERENCES c(id) ON DELETE CASCADE); {}", gone("d", "DELETE")), &["a", "b", "c", "d"]),
            (undo("d"), &[]),
            // SET NULL updates the key column of the child: an UPDATE OF
            // trigger on that column fires, one on another column does not.
            ("CREATE TABLE e(id INTEGER PRIMARY KEY, b_id INTEGER REFERENCES b(id) ON DELETE SET NULL);".into(), &[]),
            (gone("e", "UPDATE OF b_id"), &["a", "b", "e"]),
            (undo("e"), &[]),
            (gone("e", "UPDATE OF id"), &["e"]),
            (undo("e"), &[]),
            // ON UPDATE CASCADE and RESTRICT through a unique key; a key into
            // a column that is not unique takes the parent's programs and,
            // through the cascade, the grandparent's outside coverage.
            ("CREATE TABLE f(id INTEGER PRIMARY KEY, a_k TEXT REFERENCES a(k) ON UPDATE CASCADE ON DELETE RESTRICT);".into(), &[]),
            ("CREATE TABLE g(id INTEGER PRIMARY KEY, f_a_k TEXT REFERENCES f(a_k) ON UPDATE CASCADE);".into(), &["a", "f", "g"]),
            ("DROP TABLE g;".into(), &[]),
            ("CREATE UNIQUE INDEX f_a_k ON f(a_k); CREATE TABLE g(id INTEGER PRIMARY KEY, f_a_k TEXT REFERENCES f(a_k) ON UPDATE CASCADE);".into(), &[]),
            (gone("g", "UPDATE"), &["a", "f", "g"]),
            (undo("g"), &[]),
            // SET DEFAULT with function defaults.
            ("CREATE TABLE h(id INTEGER PRIMARY KEY, a_id INTEGER DEFAULT (abs(-1)) REFERENCES a(id) ON DELETE SET DEFAULT, at TEXT DEFAULT (datetime('now')));".into(), &[]),
            (gone("h", "UPDATE OF a_id"), &["a", "h"]),
            (undo("h"), &[]),
            // A REPLACE conflict on the parent deletes rows too.
            ("CREATE TABLE w(k TEXT PRIMARY KEY ON CONFLICT REPLACE, v TEXT) WITHOUT ROWID;".into(), &[]),
            (format!("CREATE TABLE wc(id INTEGER PRIMARY KEY, w_k TEXT REFERENCES w(k) ON DELETE CASCADE); {}", gone("wc", "DELETE")), &["w", "wc"]),
            (undo("wc"), &[]),
            // A cycle, and a table that references itself.
            ("CREATE TABLE x1(id INTEGER PRIMARY KEY, x2_id INTEGER REFERENCES x2(id) ON DELETE CASCADE); CREATE TABLE x2(id INTEGER PRIMARY KEY, x1_id INTEGER REFERENCES x1(id) ON DELETE CASCADE);".into(), &[]),
            (format!("CREATE TABLE x3(id INTEGER PRIMARY KEY, x2_id INTEGER REFERENCES x2(id) ON DELETE CASCADE); {}", gone("x3", "DELETE")), &["x1", "x2", "x3"]),
            (undo("x3"), &[]),
            ("CREATE TABLE tree(id INTEGER PRIMARY KEY, parent INTEGER REFERENCES tree(id) ON DELETE CASCADE);".into(), &[]),
            (format!("CREATE TABLE twig(id INTEGER PRIMARY KEY, tree_id INTEGER REFERENCES tree(id) ON DELETE CASCADE); {}", gone("twig", "DELETE")), &["tree", "twig"]),
            (undo("twig"), &[]),
            // ON UPDATE CASCADE into a child that itself cascades updates to
            // a grandchild: the child's UPDATE is no longer a leaf.
            ("CREATE TABLE c2(id INTEGER PRIMARY KEY, c_id INTEGER REFERENCES c(id) ON DELETE CASCADE ON UPDATE CASCADE);".into(), &[]),
            ("CREATE UNIQUE INDEX c2_c ON c2(c_id); CREATE TABLE c3(id INTEGER PRIMARY KEY, c2_c INTEGER REFERENCES c2(c_id) ON UPDATE CASCADE);".into(), &[]),
            (gone("c3", "UPDATE"), &["c", "c2", "c3"]),
            (undo("c3"), &[]),
            // Children dropped, a parent changed, and a cascade four levels
            // deep, which a trigger on another table also reaches.
            ("DROP TABLE e; DROP TABLE wc;".into(), &[]),
            ("ALTER TABLE b ADD COLUMN note TEXT;".into(), &[]),
            ("CREATE TABLE tz(id INTEGER); CREATE TRIGGER tz_del AFTER INSERT ON tz BEGIN DELETE FROM b WHERE id = 0; END;".into(), &[]),
            (format!("CREATE TABLE deep(id INTEGER PRIMARY KEY, d_id INTEGER REFERENCES d(id) ON DELETE CASCADE); {}", gone("deep", "DELETE")), &["a", "b", "c", "d", "deep", "tz"]),
            (undo("deep"), &[]),
            // A trigger reaches a SET NULL whose UPDATE later cascades on to a
            // new grandchild.
            ("CREATE TABLE sp(id INTEGER PRIMARY KEY); CREATE TABLE sc(id INTEGER PRIMARY KEY, sp_id INTEGER UNIQUE REFERENCES sp(id) ON DELETE SET NULL); CREATE TABLE sw(id INTEGER); CREATE TRIGGER sw_del AFTER INSERT ON sw BEGIN DELETE FROM sp WHERE id = 0; END;".into(), &[]),
            (format!("CREATE TABLE sg(id INTEGER PRIMARY KEY, sc_sp INTEGER REFERENCES sc(sp_id) ON UPDATE CASCADE); {}", gone("sg", "UPDATE")), &["sc", "sg", "sp", "sw"]),
            (undo("sg"), &[]),
            // A view of a view of a table: rebuilding the table without the
            // column the views read takes both outside coverage.
            ("CREATE TABLE vt(id INTEGER PRIMARY KEY, x INTEGER); CREATE VIEW vv1 AS SELECT id, x FROM vt; CREATE VIEW vv2 AS SELECT x FROM vv1;".into(), &[]),
            ("DROP TABLE vt; CREATE TABLE vt(id INTEGER PRIMARY KEY);".into(), &["view:\"vv1\"", "view:\"vv2\""]),
            ("DROP TABLE vt; CREATE TABLE vt(id INTEGER PRIMARY KEY, x INTEGER);".into(), &[]),
            // A trigger reaches SET DEFAULT, ON UPDATE SET NULL and ON UPDATE
            // SET DEFAULT actions whose children have UPDATE triggers writing
            // tables that are then rebuilt without the column they write.
            ("CREATE TABLE ra1(id INTEGER PRIMARY KEY, w INTEGER); CREATE TABLE ra2(id INTEGER PRIMARY KEY, w INTEGER); CREATE TABLE ra3(id INTEGER PRIMARY KEY, w INTEGER); CREATE TABLE rx(id INTEGER PRIMARY KEY, k INTEGER UNIQUE); CREATE TABLE rh1(id INTEGER PRIMARY KEY, x_id INTEGER DEFAULT 0 REFERENCES rx(id) ON DELETE SET DEFAULT); CREATE TABLE rh2(id INTEGER PRIMARY KEY, x_k INTEGER REFERENCES rx(k) ON UPDATE SET NULL); CREATE TABLE rh3(id INTEGER PRIMARY KEY, x_k INTEGER DEFAULT 0 REFERENCES rx(k) ON UPDATE SET DEFAULT); CREATE TRIGGER rh1_t AFTER UPDATE ON rh1 BEGIN INSERT INTO ra1(w) VALUES (1); END; CREATE TRIGGER rh2_t AFTER UPDATE ON rh2 BEGIN INSERT INTO ra2(w) VALUES (1); END; CREATE TRIGGER rh3_t AFTER UPDATE ON rh3 BEGIN INSERT INTO ra3(w) VALUES (1); END; CREATE TABLE rp(id INTEGER PRIMARY KEY); CREATE TRIGGER rp_t AFTER INSERT ON rp BEGIN DELETE FROM rx WHERE id = new.id; UPDATE rx SET k = k + 1 WHERE id = new.id; END;".into(), &[]),
            ("DROP TABLE ra1; CREATE TABLE ra1(id INTEGER PRIMARY KEY);".into(), &["rh1", "rp", "rx"]),
            ("DROP TABLE ra1; CREATE TABLE ra1(id INTEGER PRIMARY KEY, w INTEGER);".into(), &[]),
            ("DROP TABLE ra2; CREATE TABLE ra2(id INTEGER PRIMARY KEY);".into(), &["rh2", "rp", "rx"]),
            ("DROP TABLE ra2; CREATE TABLE ra2(id INTEGER PRIMARY KEY, w INTEGER);".into(), &[]),
            ("DROP TABLE ra3; CREATE TABLE ra3(id INTEGER PRIMARY KEY);".into(), &["rh3", "rp", "rx"]),
            ("DROP TABLE ra3; CREATE TABLE ra3(id INTEGER PRIMARY KEY, w INTEGER);".into(), &[]),
        ];
        let scripts: Vec<(String, Option<&str>)> =
            steps.iter().map(|(sql, _)| (sql.clone(), None)).collect();
        let (failed, states) = assert_incremental_inspection_is_exact(&scripts, usize::MAX, None);
        // The key into a column that is not unique fails the foreign-key
        // check; its state is inspected all the same.
        assert_eq!(
            failed,
            ["0012_step.sql DB_SQLITE: foreign key mismatch - \"g\" referencing \"f\" (SQLITE_ERROR)."]
        );
        let expected: Vec<Vec<String>> = steps
            .iter()
            .map(|(_, tables)| tables.iter().map(|t| (*t).to_owned()).collect())
            .collect();
        assert_eq!(uncovered_tables(&states), expected);
    }

    /// #421: a program that nests sub-programs `SQLITE_LIMIT_TRIGGER_DEPTH`
    /// levels deep does not compile. With the limit lowered to 5, a cascade
    /// chain takes the tables at its top outside coverage one by one as it
    /// grows past 5 levels below them, and back as it shrinks. Where the
    /// longest path below a table is past the limit but SQLite need not
    /// take it (it reaches s through d0's own key first, and finds s already
    /// coded at the end of the long path), and where a trigger in the chain
    /// leaves only a loose bound, the table is compiled again: d0 still
    /// compiles in the first case and not in the second.
    #[test]
    fn incremental_inspection_is_exact_at_the_trigger_depth_limit() {
        let link = |i: usize| {
            format!("CREATE TABLE d{i}(id INTEGER PRIMARY KEY, p INTEGER REFERENCES d{}(id) ON DELETE CASCADE);", i - 1)
        };
        let mut steps: Vec<(String, &[&str])> =
            vec![("CREATE TABLE d0(id INTEGER PRIMARY KEY);".into(), &[])];
        for i in 1..=5 {
            steps.push((link(i), &[]));
        }
        steps.extend([
            (link(6), &["d0"][..]),
            (link(7), &["d0", "d1"]),
            ("DROP TABLE d7;".into(), &["d0"]),
            ("DROP TABLE d6;".into(), &[]),
            // s is one level below d0 and five below it through d1..d4.
            ("CREATE TABLE s(id INTEGER PRIMARY KEY, d4_id INTEGER REFERENCES d4(id) ON DELETE CASCADE, d0_id INTEGER REFERENCES d0(id) ON DELETE CASCADE);".into(), &[]),
            ("CREATE TABLE s2(id INTEGER PRIMARY KEY, s_id INTEGER REFERENCES s(id) ON DELETE CASCADE);".into(), &[]),
            ("DROP TABLE s2;".into(), &[]),
            ("CREATE TABLE au(id INTEGER); CREATE TRIGGER d3_t AFTER DELETE ON d3 BEGIN INSERT INTO au VALUES (old.id); END;".into(), &[]),
            (link(6), &["d0"]),
        ]);
        let scripts: Vec<(String, Option<&str>)> =
            steps.iter().map(|(sql, _)| (sql.clone(), None)).collect();
        let (failed, states) =
            assert_incremental_inspection_is_exact(&scripts, usize::MAX, Some(5));
        assert_eq!(failed, Vec::<String>::new());
        let expected: Vec<Vec<String>> = steps
            .iter()
            .map(|(_, tables)| tables.iter().map(|t| (*t).to_owned()).collect())
            .collect();
        assert_eq!(uncovered_tables(&states), expected);
    }

    /// #421: the bound on how deeply a program nests sub-programs counts
    /// every way SQLite nests them. In each history a change below an
    /// unchanged table takes it, and not the table below it that certifies
    /// it, past a lowered limit, so it is outside coverage only if it is
    /// compiled again:
    ///
    /// - limit 1: a table that references itself nests its own cascade once.
    /// - limit 2: a SET NULL action's UPDATE fires the child's triggers and
    ///   the ON UPDATE actions (CASCADE or RESTRICT) of keys into the columns
    ///   it changes, named or through the primary key; one that fires
    ///   nothing still nests a level.
    /// - limit 3: in a cycle of keys, one key's action nests twice, for a
    ///   DELETE and for an UPDATE.
    /// - limit 4: SQLite codes a trigger once per conflict policy it is fired
    ///   with, so a path through triggers passes one trigger more than once.
    /// - limit 7: the levels below a cycle add to those in it.
    #[test]
    fn incremental_inspection_is_exact_where_sub_programs_nest_at_the_limit() {
        let cascade = |child: &str, parent: &str| {
            format!("CREATE TABLE {child}(id INTEGER PRIMARY KEY, p_id INTEGER REFERENCES {parent}(id) ON DELETE CASCADE);")
        };
        let set_null = |child: &str, key: &str, parent: &str| {
            format!("CREATE TABLE {child}({key} REFERENCES {parent}(id) ON DELETE SET NULL);")
        };
        let policy = |i: usize, policy: &str| {
            format!("CREATE TRIGGER d{i} AFTER DELETE ON t BEGIN INSERT OR {policy} INTO u(v) VALUES (old.x); END;")
        };
        let mut below_cycle = String::from("CREATE TABLE r8(id INTEGER PRIMARY KEY); CREATE TABLE x(id INTEGER PRIMARY KEY, r_id INTEGER REFERENCES r8(id) ON DELETE CASCADE, y_id INTEGER REFERENCES y(id) ON DELETE CASCADE); CREATE TABLE y(id INTEGER PRIMARY KEY, x_id INTEGER REFERENCES x(id) ON DELETE CASCADE);");
        below_cycle.push_str(&cascade("z1", "y"));
        for i in 2..=5 {
            below_cycle.push_str(&cascade(&format!("z{i}"), &format!("z{}", i - 1)));
        }
        type Steps<'a> = Vec<(String, &'a [&'a str])>;
        let histories: Vec<(i32, Steps)> = vec![
            (1, vec![
                (format!("CREATE TABLE r7(id INTEGER PRIMARY KEY); {}", cascade("tree", "r7")), &[]),
                ("ALTER TABLE tree ADD COLUMN parent INTEGER REFERENCES tree(id) ON DELETE CASCADE;".into(), &["r7"]),
            ]),
            (2, vec![
                (format!("CREATE TABLE p1(id INTEGER PRIMARY KEY); {} {} CREATE TABLE q1(v INTEGER);", cascade("c1", "p1"), set_null("s1", "id INTEGER PRIMARY KEY, c_id INTEGER", "c1")), &[]),
                ("CREATE TRIGGER s1_t AFTER UPDATE ON s1 BEGIN INSERT INTO q1(v) VALUES (1); END;".into(), &["p1"]),
                ("DROP TRIGGER s1_t;".into(), &[]),
                (format!("CREATE TABLE p2(id INTEGER PRIMARY KEY); {} {}", cascade("c2", "p2"), set_null("s2", "id INTEGER PRIMARY KEY, c_id INTEGER UNIQUE", "c2")), &[]),
                ("CREATE TABLE g2(id INTEGER PRIMARY KEY, s_c INTEGER REFERENCES s2(c_id) ON UPDATE CASCADE);".into(), &["p2"]),
                ("DROP TABLE g2;".into(), &[]),
                ("CREATE TABLE g2(id INTEGER PRIMARY KEY, s_c INTEGER REFERENCES s2(c_id) ON UPDATE RESTRICT);".into(), &["p2"]),
                ("DROP TABLE g2;".into(), &[]),
                (format!("CREATE TABLE p4(id INTEGER PRIMARY KEY); {} CREATE TABLE s4(c_id INTEGER REFERENCES c4(id) ON DELETE SET NULL, v INTEGER, PRIMARY KEY(c_id, v));", cascade("c4", "p4")), &[]),
                ("CREATE TABLE g4(c INTEGER, v INTEGER, FOREIGN KEY(c, v) REFERENCES s4 ON UPDATE CASCADE);".into(), &["p4"]),
                ("DROP TABLE g4;".into(), &[]),
                (format!("CREATE TABLE p5(id INTEGER PRIMARY KEY); {} {}", cascade("c5", "p5"), cascade("d5", "c5")), &[]),
                (set_null("s5", "id INTEGER PRIMARY KEY, d_id INTEGER", "d5"), &["p5"]),
            ]),
            (3, vec![
                ("CREATE TABLE r(id INTEGER PRIMARY KEY); CREATE TABLE p(id INTEGER PRIMARY KEY REFERENCES c(p_id) ON UPDATE CASCADE, r_id INTEGER REFERENCES r(id) ON DELETE CASCADE); CREATE TABLE c(id INTEGER PRIMARY KEY, p_id INTEGER UNIQUE REFERENCES p(id) ON DELETE SET NULL);".into(), &[]),
                ("DROP TABLE c; CREATE TABLE c(id INTEGER PRIMARY KEY, p_id INTEGER UNIQUE REFERENCES p(id) ON DELETE SET NULL ON UPDATE CASCADE);".into(), &["r"]),
            ]),
            (4, vec![
                (format!("CREATE TABLE p(id INTEGER PRIMARY KEY); CREATE TABLE t(p_id INTEGER REFERENCES p(id) ON DELETE CASCADE, x INTEGER); CREATE TABLE u(v INTEGER); CREATE TRIGGER tu AFTER INSERT ON u BEGIN DELETE FROM t WHERE x = new.v; END; {}", policy(1, "ROLLBACK")), &[]),
                (policy(2, "ABORT"), &["p", "u"]),
            ]),
            (7, vec![
                (below_cycle, &[]),
                (cascade("z6", "z5"), &["r8"]),
            ]),
        ];
        for (limit, steps) in histories {
            let scripts: Vec<(String, Option<&str>)> =
                steps.iter().map(|(sql, _)| (sql.clone(), None)).collect();
            let (failed, states) =
                assert_incremental_inspection_is_exact(&scripts, usize::MAX, Some(limit));
            assert_eq!(failed, Vec::<String>::new(), "limit {limit}");
            let expected: Vec<Vec<String>> = steps
                .iter()
                .map(|(_, tables)| tables.iter().map(|t| (*t).to_owned()).collect())
                .collect();
            assert_eq!(uncovered_tables(&states), expected, "limit {limit}");
        }
    }

    /// #421: programs compiled with foreign keys off, as a rebuild
    /// migration's guard inspects the rebuilt catalog, are compiled again
    /// once they are on. Rebuilding x with a cascade from c nests p's DELETE
    /// two levels deep with foreign keys on, past a limit of 1, and not at
    /// all with them off.
    #[test]
    fn incremental_inspection_is_exact_when_foreign_key_enforcement_changes() {
        let scripts: Vec<(String, Option<&str>)> = [
            ("CREATE TABLE p(id INTEGER PRIMARY KEY); CREATE TABLE c(id INTEGER PRIMARY KEY, p_id INTEGER REFERENCES p(id) ON DELETE CASCADE); CREATE TABLE x(id INTEGER PRIMARY KEY, c_id INTEGER);", None),
            ("CREATE TABLE x2(id INTEGER PRIMARY KEY, c_id INTEGER REFERENCES c(id) ON DELETE CASCADE); INSERT INTO x2 SELECT id, c_id FROM x; DROP TABLE x; ALTER TABLE x2 RENAME TO x;", Some("x")),
            ("CREATE TABLE y(id INTEGER);", None),
        ]
        .into_iter()
        .map(|(sql, rebuild)| (sql.to_owned(), rebuild))
        .collect();
        let (failed, states) =
            assert_incremental_inspection_is_exact(&scripts, usize::MAX, Some(1));
        assert_eq!(failed, Vec::<String>::new());
        assert_eq!(uncovered_tables(&states), [vec![], vec!["p"], vec!["p"]]);
    }

    /// `steps` migrations over a few tables that reference each other and
    /// themselves through the rowid or a unique key, with every ON DELETE
    /// and ON UPDATE action, function defaults, triggers on any of them that
    /// write another (now and then the ledger), indexes, added columns and
    /// drops (none that replay refuses). Each migration is one an in-memory
    /// SQLite database with foreign keys on accepted after the ones before it.
    fn generated_foreign_key_history(steps: usize, seed: u64) -> Vec<String> {
        const TABLES: &[&str] = &["f0", "f1", "f2", "f3", "f4", "f5"];
        const ACTIONS: &[&str] = &[
            "NO ACTION",
            "RESTRICT",
            "CASCADE",
            "SET NULL",
            "SET DEFAULT",
        ];
        const EVENTS: &[&str] = &[
            "DELETE",
            "INSERT",
            "UPDATE",
            "UPDATE OF v",
            "UPDATE OF r0",
            "UPDATE OF s0",
            "UPDATE OF k",
        ];
        let mut rng = Rng(seed);
        let db = rusqlite::Connection::open_in_memory().unwrap();
        db.execute_batch("PRAGMA foreign_keys=ON; PRAGMA recursive_triggers=ON;")
            .unwrap();
        let mut history = Vec::new();
        while history.len() < steps {
            let t = rng.pick(TABLES);
            let other = rng.pick(TABLES);
            let statement = match rng.below(14) {
                0..=4 => {
                    let mut columns = String::from("id INTEGER PRIMARY KEY, k TEXT UNIQUE, v TEXT");
                    for j in 0..rng.below(3) {
                        let target = rng.pick(TABLES);
                        let column = match rng.below(3) {
                            0 => format!("r{j} INTEGER DEFAULT (abs(-1)) REFERENCES {target}(id)"),
                            1 => format!("s{j} TEXT REFERENCES {target}(k)"),
                            _ => format!("r{j} INTEGER REFERENCES {target}"),
                        };
                        let (on_delete, on_update) = (rng.pick(ACTIONS), rng.pick(ACTIONS));
                        columns.push_str(&format!(
                            ", {column} ON DELETE {on_delete} ON UPDATE {on_update}"
                        ));
                    }
                    format!("CREATE TABLE {t}({columns})")
                }
                5 => {
                    // Replay refuses to drop a table whose rows other tables'
                    // ON DELETE actions would change (#419).
                    let refused: i64 = db
                        .query_row(
                            "SELECT count(*) FROM sqlite_schema AS m, pragma_foreign_key_list(m.name) AS f \
                             WHERE m.type = 'table' AND m.name <> ?1 COLLATE NOCASE AND f.\"table\" = ?1 COLLATE NOCASE \
                             AND f.on_delete IN ('CASCADE', 'SET NULL', 'SET DEFAULT')",
                            [t],
                            |row| row.get(0),
                        )
                        .unwrap();
                    if refused > 0 {
                        continue;
                    }
                    format!("DROP TABLE {t}")
                }
                6..=8 => {
                    let body = match rng.below(9) {
                        0 | 1 => format!("DELETE FROM {other} WHERE id = 0"),
                        2 => format!("UPDATE {other} SET k = k WHERE id = 0"),
                        3 => format!("UPDATE {other} SET v = v WHERE id = 0"),
                        4 | 5 => format!("INSERT INTO {other}(v) VALUES ('x')"),
                        6 => "DELETE FROM _ntnt_schema_migrations_v2".to_owned(),
                        _ => "SELECT 1".to_owned(),
                    };
                    let event = EVENTS[rng.below(EVENTS.len())];
                    let name = ["ga", "gb", "gc"][rng.below(3)];
                    format!("CREATE TRIGGER {name} AFTER {event} ON {t} BEGIN {body}; END")
                }
                9 => format!("DROP TRIGGER {}", ["ga", "gb", "gc"][rng.below(3)]),
                10 => format!(
                    "CREATE {}INDEX {} ON {t}(v)",
                    ["", "UNIQUE "][rng.below(2)],
                    ["ia", "ib"][rng.below(2)]
                ),
                11 => format!("DROP INDEX {}", ["ia", "ib"][rng.below(2)]),
                _ => format!("ALTER TABLE {t} ADD COLUMN z{} TEXT", rng.below(3)),
            };
            let script = format!("{statement};\n");
            db.execute_batch("SAVEPOINT candidate").unwrap();
            let accepted = db.execute_batch(&script).is_ok();
            let end = if accepted {
                "RELEASE candidate"
            } else {
                "ROLLBACK TO candidate; RELEASE candidate"
            };
            db.execute_batch(end).unwrap();
            if accepted {
                history.push(script);
            }
        }
        history
    }

    /// #421: generated foreign-key histories, at SQLite's own limit on
    /// nested trigger programs and at a limit of 3 that their cascades reach.
    #[test]
    fn incremental_inspection_is_exact_over_generated_foreign_key_histories() {
        let mut uncovered = 0;
        for (seed, limit) in [
            (0x2545_f491_4f6c_dd1d, None),
            (0x9e37_79b9_7f4a_7c15, Some(3)),
            (0x1234_5678_9abc_def1, Some(3)),
        ] {
            let history: Vec<_> = generated_foreign_key_history(100, seed)
                .into_iter()
                .map(|sql| (sql, None))
                .collect();
            let (failed, states) = assert_incremental_inspection_is_exact(&history, 50, limit);
            assert!(failed.len() <= 50, "{failed:?}");
            uncovered += uncovered_states(&states);
        }
        // Both covered and uncovered states occur.
        assert!((30..=270).contains(&uncovered), "{uncovered}");
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
