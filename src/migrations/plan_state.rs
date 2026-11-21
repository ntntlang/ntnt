//! Why a saved plan no longer applies (#423). A stale plan names the binding
//! that changed: the database and target it was made for, an input file, the
//! policy, the target itself, its history or its catalog. An artifact that is
//! not an intact saved plan (edited by hand, a forged digest, a blocked plan)
//! is invalid rather than stale, and its refusal says only that.
use super::inventory::{json_digest, CapturedInputs};
use super::model::{History, HistoryEntry};
use super::Diagnostic;
use serde_json::{json, Value};

/// The files a stale plan's refusal names, as the user gave them.
#[derive(Clone, Copy, Default)]
pub(super) struct Files<'a> {
    /// The saved plan (`--plan` or `--against`).
    pub plan: Option<&'a str>,
    /// The manifest (`--manifest`).
    pub manifest: Option<&'a str>,
}
impl Files<'_> {
    fn plan(&self) -> String {
        self.plan
            .map_or_else(|| "The saved plan".to_owned(), str::to_owned)
    }
    fn manifest(&self) -> &str {
        self.manifest.unwrap_or("the manifest")
    }
}

fn stale(message: String, next_action: String) -> Diagnostic {
    let mut d = Diagnostic::new("DB_PLAN_STALE", &message);
    d.next_action = next_action;
    d
}
/// The refusal of an artifact that is not an intact saved plan. It is not
/// about the target, the history or the inputs.
pub(super) fn invalid() -> Diagnostic {
    Diagnostic::new(
        "DB_PLAN_STALE",
        "Saved plan is invalid: its contents do not match its plan_digest or the shape `ntnt db plan --out` saves, so it was changed after it was saved or is not a saved plan. Generate a new plan; never edit a saved plan.",
    )
}
fn hex64(value: &Value) -> bool {
    value.as_str().is_some_and(|s| {
        s.len() == 64
            && s.bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
    })
}
/// Text from a saved plan as a refusal shows it: as written when it is a
/// simple name (a manifest key or a migration file name), otherwise not at
/// all. Hand-edited or forged plans can hold anything.
fn simple(value: &Value) -> Option<&str> {
    let text = value.as_str()?;
    (!text.is_empty()
        && text.len() <= 255
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.')))
    .then_some(text)
}

/// Refuse `saved` unless it is an intact plan as `ntnt db plan --out` saves
/// it, judging only what it holds: its digest matches, its own outcome
/// passed and its evidence is self-consistent. Every condition here is also
/// one `execution::validate` requires, so this refuses no plan it accepts.
pub(super) fn intact(saved: &Value) -> Result<(), Diagnostic> {
    let plan = &saved["plan"];
    if saved["format_version"] != 1
        || !plan.is_object()
        || saved["plan_digest"] != json_digest(plan)
    {
        return Err(invalid());
    }
    if saved["outcome"] != "pass"
        || plan["classification"] != "requires_review"
        || plan["candidate"]["outcome"] != "pass"
    {
        let outcome = simple(&saved["outcome"]).unwrap_or("not pass");
        return Err(stale(
            format!("Saved plan is blocked: its outcome is {outcome}, so `ntnt db plan` found problems it could not resolve, and a blocked plan is never executed."),
            "Fix what that plan's diagnostics reported, then plan, review and approve again.".to_owned(),
        ));
    }
    let b = &plan["binding"];
    let fingerprints = plan["execution"]["prefix_fingerprints"].as_array();
    let base = b["applied_count"].as_u64();
    let reference = match plan.get("action") {
        Some(action) => action["count"].as_u64(),
        None => base,
    };
    let at = |index: Option<u64>| {
        fingerprints
            .zip(index.and_then(|i| usize::try_from(i).ok()))
            .and_then(|(list, i)| list.get(i))
    };
    let consistent = plan["authorized"] == false
        && plan["executable"] == false
        && fingerprints.is_some_and(|list| !list.is_empty() && list.iter().all(hex64))
        && fingerprints.and_then(|list| list.last()) == Some(&plan["candidate"]["fingerprint"])
        && base.is_some_and(|n| usize::try_from(n).is_ok())
        && at(reference)
            .is_some_and(|f| *f == b["expected_base_digest"] && *f == b["base_catalog_digest"])
        && b["format_version"] == 1
        && b["canonicalization_version"] == 1
        && hex64(&b["base_history_digest"]);
    if consistent {
        Ok(())
    } else {
        Err(invalid())
    }
}

/// Refuse a plan made for another logical database or target. This needs
/// no I/O, so it is known before anything connects (#423, cli-09).
pub(super) fn selection(
    inputs: &CapturedInputs,
    saved: &Value,
    files: Files,
) -> Result<(), Diagnostic> {
    let b = &saved["plan"]["binding"];
    if b["database"] == inputs.database_name && b["target_name"] == inputs.target_name {
        return Ok(());
    }
    let (database, target) = (&inputs.database_name, &inputs.target_name);
    match (simple(&b["database"]), simple(&b["target_name"])) {
        (Some(planned_database), Some(planned_target)) => Err(stale(
            format!(
                "{} was planned for database {planned_database}, target {planned_target}; this run selected database {database}, target {target}.",
                files.plan()
            ),
            format!("Pass --database {planned_database} --target {planned_target}, the database and target this plan was reviewed for, or plan, review and approve a new plan for database {database}, target {target}."),
        )),
        _ => Err(invalid()),
    }
}

/// `name: plan X, now Y` for each field of two JSON objects that differs.
fn field_differences(planned: &Value, now: &Value) -> Vec<String> {
    let keys: std::collections::BTreeSet<&String> = [planned, now]
        .into_iter()
        .filter_map(Value::as_object)
        .flat_map(|object| object.keys())
        .collect();
    keys.into_iter()
        .filter(|key| planned[key.as_str()] != now[key.as_str()])
        .map(|key| {
            format!(
                "{key}: plan {}, now {}",
                shown(&planned[key.as_str()]),
                shown(&now[key.as_str()])
            )
        })
        .collect()
}
/// A JSON value in a refusal: strings as written, missing as `none`, other
/// values as compact JSON, at most 80 characters.
fn shown(value: &Value) -> String {
    let text = match value {
        Value::Null => "none".to_owned(),
        Value::String(text) => text.escape_debug().to_string(),
        other => other.to_string(),
    };
    if text.chars().count() > 80 {
        format!("{}…", text.chars().take(80).collect::<String>())
    } else {
        text
    }
}

/// Refuse a plan made under another policy for the selected database: its
/// limits, engine, managed schema or any other part of its configuration.
pub(super) fn policy(
    inputs: &CapturedInputs,
    saved: &Value,
    files: Files,
) -> Result<(), Diagnostic> {
    let plan = &saved["plan"];
    let b = &plan["binding"];
    let database = &inputs.database_name;
    let table = format!("[databases.{database}]");
    let replan =
        "Plan, review and approve again: the plan was reviewed under the previous configuration.";
    // Limits first: they are policy too, and the plan records them.
    let planned = plan["execution"].get("limits");
    let now = inputs.plan_limits();
    let limits = format!("[databases.{database}.limits]");
    let pairs = |value: &Value| {
        value.as_object().map_or_else(String::new, |fields| {
            fields
                .iter()
                .map(|(key, value)| format!("{key} = {}", shown(value)))
                .collect::<Vec<_>>()
                .join(", ")
        })
    };
    let changed = match (planned, &now) {
        (Some(planned), Some(now)) if planned != now => {
            Some(field_differences(planned, now).join("; "))
        }
        (Some(planned), None) => Some(format!(
            "the plan was made with {}, and the manifest now declares no limits table",
            pairs(planned)
        )),
        (None, Some(now)) if inputs.config.limits.is_some() => Some(format!(
            "the manifest now declares {}, and the plan was made without the table, with the default limits",
            pairs(now)
        )),
        _ => None,
    };
    if let Some(changed) = changed {
        return Err(stale(
            format!(
                "{limits} in {} changed after the plan was made: {changed}.",
                files.manifest()
            ),
            format!(
                "{replan} To run the reviewed plan instead, restore the limits it was made with."
            ),
        ));
    }
    let adapter = super::adapter::catalog_format(&inputs.config.engine);
    if b["adapter"] != adapter {
        return Err(stale(
            format!(
                "{table} in {} now declares engine {}, and the plan was made for another engine ({}).",
                files.manifest(),
                inputs.config.engine,
                simple(&b["adapter"]).unwrap_or("unknown")
            ),
            replan.to_owned(),
        ));
    }
    if b["managed_schema"] != inputs.config.schema {
        return Err(stale(
            format!(
                "{table} in {} changed after the plan was made: schema: plan {}, now {}.",
                files.manifest(),
                simple(&b["managed_schema"]).unwrap_or("unknown"),
                inputs.config.schema
            ),
            replan.to_owned(),
        ));
    }
    if b["policy_digest"] != inputs.policy_digest {
        return Err(stale(
            format!(
                "{table} in {} changed after the plan was made: the plan binds the database's whole configuration (engine, schema, migrations_dir, targets, scratch, approval_secret, adoption_checks and limits), and its policy digest differs now.",
                files.manifest()
            ),
            replan.to_owned(),
        ));
    }
    Ok(())
}

/// The ledger of a target whose first `count` captured migrations were all
/// applied by `ntnt db migrate`, or that has no ledger yet.
fn applied_ledger(inputs: &CapturedInputs, count: usize) -> History {
    History {
        present: count > 0,
        scope: (count > 0)
            .then(|| json!({"database":inputs.database_name,"schema":inputs.config.schema})),
        entries: inputs
            .migrations
            .iter()
            .take(count)
            .enumerate()
            .map(|(i, m)| HistoryEntry {
                position: i as i64 + 1,
                id: m.id.to_string(),
                name: m.name.clone(),
                checksum: m.checksum.clone(),
                state: "applied".into(),
            })
            .collect(),
    }
}

/// Refuse a plan whose captured inputs changed, naming the file (#423).
pub(super) fn inputs(
    inputs: &CapturedInputs,
    saved: &Value,
    files: Files,
) -> Result<(), Diagnostic> {
    let plan = &saved["plan"];
    let b = &plan["binding"];
    if b["input_digest"] == inputs.input_digest {
        return Ok(());
    }
    let dir = &inputs.config.migrations_dir;
    let replan = "Plan, review and approve again: the reviewed SQL is no longer what would run.";
    let changed = |message: String| Err(stale(message, replan.to_owned()));
    let base = b["applied_count"]
        .as_u64()
        .and_then(|n| usize::try_from(n).ok())
        .unwrap_or_default();
    let migrations = &inputs.migrations;
    if base > migrations.len() {
        return changed(format!(
            "The plan was made with {base} migration(s) applied, and {dir}/ now holds {}: applied migrations were removed after the plan was made.",
            migrations.len()
        ));
    }
    // An adoption plan records the checksum of the prefix it covers.
    if let Some(action) = plan.get("action") {
        let Some(count) = action["count"]
            .as_u64()
            .and_then(|n| usize::try_from(n).ok())
        else {
            return Err(invalid());
        };
        if count == 0 || count > migrations.len() {
            return changed(format!(
                "The adoption covers {count} migration(s), and {dir}/ now holds {}: migrations it covers were removed after the plan was made.",
                migrations.len()
            ));
        }
        let (first, last) = (&migrations[0].name, &migrations[count - 1].name);
        if inputs
            .checkpoint(count)
            .is_ok_and(|checkpoint| action["prefix_checksum"] != checkpoint.checksum)
        {
            return changed(format!(
                "A migration the adoption covers ({first} through {last}) changed after the plan was made: the plan records one checksum for all of them, so it cannot say which file changed."
            ));
        }
        return changed(format!(
            "{} or a migration after {last} changed or was added after the plan was made.",
            files.manifest()
        ));
    }
    // The plan lists the migrations it would apply, with their checksums.
    {
        let planned = plan["pending"].as_array().map_or(&[][..], Vec::as_slice);
        let current = &migrations[base..];
        for (index, entry) in planned.iter().enumerate() {
            let Some(name) = simple(&entry["source"]) else {
                return Err(invalid());
            };
            match current.get(index) {
                Some(now) if now.name == name => {
                    if entry["checksum"] != now.checksum {
                        let was = entry["checksum"]
                            .as_str()
                            .filter(|_| hex64(&entry["checksum"]));
                        return changed(format!(
                            "{name} changed after the plan was made: checksum: plan {}, now {}.",
                            was.unwrap_or("unknown"),
                            now.checksum
                        ));
                    }
                }
                Some(now) if !current.iter().any(|m| m.name == name) => {
                    return changed(format!(
                        "{name}, which the plan would apply, is no longer in {dir}/; {} is in its place.",
                        now.name
                    ));
                }
                Some(now) => {
                    return changed(format!(
                        "{} was added to {dir}/ after the plan was made, before {name}.",
                        now.name
                    ));
                }
                None => {
                    return changed(format!(
                        "{name}, which the plan would apply, is no longer in {dir}/."
                    ));
                }
            }
        }
        if let Some(added) = current.get(planned.len()) {
            return changed(format!(
                "{} was added to {dir}/ after the plan was made.",
                added.name
            ));
        }
    }
    // Neither: the applied migrations or the manifest. A ledger of plain
    // applied migrations can be rebuilt from the files as they are now; if
    // it matches the plan's, the applied migrations are unchanged.
    if base == 0 || json_digest(&applied_ledger(inputs, base)) == b["base_history_digest"] {
        return changed(format!(
            "{} changed after the plan was made: the plan binds its exact bytes, although the selected database's configuration is unchanged.",
            files.manifest()
        ));
    }
    let (first, last) = (&migrations[0].name, &migrations[base - 1].name);
    changed(format!(
        "{} or one of the migrations the plan treats as applied ({first} through {last}) changed after the plan was made; `ntnt db status` names an applied migration that changed (DB_HISTORY_EDITED).",
        files.manifest()
    ))
}

/// Refuse a plan whose target is no longer the one it was made for, naming
/// each field of its identity that differs: `target setting journal_mode:
/// plan delete, now wal` (#423, sq-12).
pub(super) fn target(planned: &Value, observed: &Value, engine: &str) -> Diagnostic {
    fn walk(path: &mut Vec<String>, planned: &Value, observed: &Value, out: &mut Vec<String>) {
        if let (Some(a), Some(b)) = (planned.as_object(), observed.as_object()) {
            let keys: std::collections::BTreeSet<&String> = a.keys().chain(b.keys()).collect();
            for key in keys {
                path.push(key.clone());
                walk(path, &planned[key.as_str()], &observed[key.as_str()], out);
                path.pop();
            }
        } else if planned != observed {
            let label = match path.as_slice() {
                [settings, key] if settings == "settings" => format!("target setting {key}"),
                [] => "target identity".to_owned(),
                _ => format!("target {}", path.join(".")),
            };
            out.push(format!(
                "{label}: plan {}, now {}",
                shown(planned),
                shown(observed)
            ));
        }
    }
    let mut differences = Vec::new();
    walk(&mut Vec::new(), planned, observed, &mut differences);
    let total = differences.len();
    differences.truncate(3);
    if total > 3 {
        differences.push(format!("and {} more", total - 3));
    }
    let mut next_action = "Plan, review and approve again for this target: a plan binds the target's identity and settings, so it runs only on the target as it was planned.".to_owned();
    if engine == "sqlite"
        && differences
            .iter()
            .any(|d| d.starts_with("target setting journal_mode"))
    {
        next_action.push_str(" std/db/sqlite connect() switches a database to WAL mode, so plan a new database after the application has opened it once.");
    }
    stale(
        format!(
            "The target changed after the plan was made: {}.",
            differences.join("; ")
        ),
        next_action,
    )
}

/// Refuse a plan whose target history changed: it was made with `planned`
/// migrations covered, and the target's ledger covers `now`.
pub(super) fn history(inputs: &CapturedInputs, planned: usize, now: usize) -> Diagnostic {
    history_of(&inputs.database_name, &inputs.target_name, planned, now)
}
fn history_of(database: &str, target: &str, planned: usize, now: usize) -> Diagnostic {
    let message = if planned == now {
        format!("The target's history changed after the plan was made: its ledger still covers {now} migration(s), but its entries are not the ones the plan was made with.")
    } else {
        format!("The target's history changed after the plan was made: the plan was made with {planned} migration(s) covered by its ledger, and the ledger now covers {now}.")
    };
    stale(
        message,
        format!(
            "Run `ntnt db status --database {database} --target {target}` to see the target's history now, then plan, review and approve again; never edit the ledger to match a plan."
        ),
    )
}

/// Refuse a plan whose target catalog is not the plan's state after its
/// first `applied` migrations: the schema changed outside migrations, or
/// holds objects outside coverage.
pub(super) fn catalog(inputs: &CapturedInputs, applied: usize, uncovered: &[String]) -> Diagnostic {
    let last = applied
        .checked_sub(1)
        .and_then(|i| inputs.migrations.get(i))
        .map(|m| m.name.as_str());
    catalog_of(&inputs.database_name, &inputs.target_name, last, uncovered)
}
/// `catalog` for the plan's state after migration `last` (none: before the first).
fn catalog_of(
    database: &str,
    target: &str,
    last: Option<&str>,
    uncovered: &[String],
) -> Diagnostic {
    let state = match last {
        Some(last) => format!("the plan's state after {last}"),
        None => "the plan's state before the first migration".to_owned(),
    };
    let verify = format!("ntnt db verify --database {database} --target {target}");
    if uncovered.is_empty() {
        stale(
            format!("The target's catalog changed after the plan was made: it no longer matches {state}, so something changed the schema outside migrations."),
            format!("Run `{verify}` to see the differences (drift.differences), undo or migrate the change deliberately, then plan, review and approve again."),
        )
    } else {
        stale(
            format!("The target's catalog has objects outside inspection coverage, so it cannot be matched to {state}; its objects field lists them."),
            format!("Remove or change those objects so `{verify}` passes, then plan, review and approve again."),
        )
        .with_objects(uncovered)
    }
}

/// Why the plan `saved` does not match what `report` observed, for
/// `--against`, or `None` when it does. The checks that need no I/O ran
/// before the inspection; these compare the target, history and catalog it
/// observed with the ones the plan bound.
pub(super) fn against(saved: &Value, report: &Value) -> Option<Diagnostic> {
    let p = &saved["plan"];
    if intact(saved).is_err() {
        return Some(invalid());
    }
    let observed = &report["binding"];
    if observed.is_null() {
        return Some(Diagnostic::new(
            "DB_PLAN_STALE",
            "The saved plan could not be checked against the target: this inspection did not complete (see the diagnostics above).",
        ));
    }
    let planned = &p["binding"];
    let (database, target) = (
        report["database"].as_str().unwrap_or_default(),
        report["target"]["name"].as_str().unwrap_or_default(),
    );
    if planned != observed {
        if planned["target"] != observed["target"] {
            let engine = if observed["adapter"] == super::adapter::catalog_format("sqlite") {
                "sqlite"
            } else {
                "postgresql"
            };
            return Some(self::target(
                &planned["target"],
                &observed["target"],
                engine,
            ));
        }
        let count = |b: &Value| {
            b["applied_count"]
                .as_u64()
                .and_then(|n| usize::try_from(n).ok())
        };
        if planned["base_history_digest"] != observed["base_history_digest"]
            || planned["applied_count"] != observed["applied_count"]
        {
            if let (Some(was), Some(now)) = (count(planned), count(observed)) {
                return Some(history_of(database, target, was, now));
            }
        }
        if planned["base_catalog_digest"] != observed["base_catalog_digest"]
            || planned["expected_base_digest"] != observed["expected_base_digest"]
        {
            let applied = count(observed).unwrap_or_default();
            let last = applied
                .checked_sub(1)
                .and_then(|i| report["source"]["migrations"][i]["source"].as_str());
            let uncovered: Vec<String> = report["catalog"]["uncovered"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|key| key.as_str().map(str::to_owned))
                .collect();
            return Some(catalog_of(database, target, last, &uncovered));
        }
        return Some(Diagnostic::new("DB_PLAN_STALE", "Saved plan is bound to different source, policy, target or base state. Generate a new plan."));
    }
    let matches = match p.get("action") {
        Some(action) => *action == report["adoption"]["action"] && p["pending"] == json!([]),
        None => {
            let covered = report["history"]["covered_count"]
                .as_u64()
                .and_then(|n| usize::try_from(n).ok())
                .unwrap_or(usize::MAX);
            let pending: Vec<&Value> = report["source"]["migrations"]
                .as_array()
                .into_iter()
                .flatten()
                .skip(covered)
                .collect();
            p["pending"] == json!(pending)
        }
    };
    (!matches).then(|| Diagnostic::new("DB_PLAN_STALE", "Saved plan is bound to different source, policy, target or base state. Generate a new plan."))
}
