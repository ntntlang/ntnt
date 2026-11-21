//! Explicit baseline identity, distinct from evidence of executed migrations.
use super::inventory::CapturedInputs;
use super::model::History;
use super::{Diagnostic, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
/// Captured adoption action; count is covered source files, not executed SQL.
pub(super) struct Adoption {
    pub kind: String,
    pub through_id: String,
    pub count: usize,
    pub prefix_checksum: String,
    pub checks: Vec<String>,
}
fn invalid() -> Diagnostic {
    Diagnostic::new("DB_ADOPTION_INPUT","Adoption requires an exact captured prefix, named adoption_checks, and either no ledger or a fully matching recognized legacy v1 ledger. No history is inferred or repaired.")
}
/// A refusal for one adoption requirement, with advice for it (#422).
fn refuse(message: &str, next_action: &str) -> Diagnostic {
    let mut d = Diagnostic::new("DB_ADOPTION_INPUT", message);
    d.next_action = next_action.to_owned();
    d
}
impl Adoption {
    fn new(inputs: &CapturedInputs, count: usize, kind: &str) -> Result<Self> {
        if inputs.config.adoption_checks.is_empty() {
            let table = format!("[databases.{}.adoption_checks]", inputs.database_name);
            let schema = &inputs.config.schema;
            return Err(refuse(
                &format!("Adoption needs at least one named check, and {table} declares none. No history is inferred or repaired."),
                &format!("Add a {table} table to the manifest with at least one named check that the existing data must pass, such as no_null_email = \"SELECT NOT EXISTS (SELECT 1 FROM {schema}.accounts WHERE email IS NULL)\", then plan again. See docs/DATABASE_MIGRATIONS.md#explicit-adoption-of-existing-databases."),
            ));
        }
        if !(kind == "adopt_baseline"
            || (kind == "import_legacy_v1" && inputs.config.engine == "postgresql"))
        {
            return Err(invalid());
        }
        let checkpoint = inputs.checkpoint(count)?;
        Ok(Self {
            kind: kind.into(),
            through_id: checkpoint.id,
            count,
            prefix_checksum: checkpoint.checksum,
            checks: inputs.config.adoption_checks.keys().cloned().collect(),
        })
    }
    /// Resolve the explicit ID/check policy before any database I/O.
    pub fn selected(inputs: &CapturedInputs, through: u64) -> Result<Self> {
        let count = inputs
            .migrations
            .iter()
            .position(|m| m.id == through)
            .map(|n| n + 1)
            .ok_or_else(|| {
                let dir = &inputs.config.migrations_dir;
                match inputs.migrations.last() {
                    Some(last) => refuse(
                        &format!("Migration ID {through} is not in {dir}; the highest ID there is {}.", last.id),
                        &format!("Pass --adopt-through with the ID of the last migration the existing database already matches, such as --adopt-through {}: a migration ID in {dir}, not a count of files.", last.id),
                    ),
                    None => refuse(
                        &format!("Migration ID {through} is not in {dir}, which holds no migrations."),
                        &format!("Add the migrations the existing database already matches to {dir}, then pass the ID of the last one to --adopt-through."),
                    ),
                }
            })?;
        Self::new(inputs, count, "adopt_baseline")
    }
    /// Select an absent-ledger checkpoint or a fully matching recognized import.
    pub fn request(inputs: &CapturedInputs, through: u64, history: &History) -> Result<Self> {
        let mut action = Self::selected(inputs, through)?;
        if history.present && history.scope.is_none() && inputs.config.engine == "postgresql" {
            action.kind = "import_legacy_v1".into();
        }
        action.before(inputs, history)?;
        Ok(action)
    }
    /// Reconstruct the descriptor from captured inputs before resolving approval.
    pub fn from_plan(inputs: &CapturedInputs, saved: &Value) -> Result<Option<Self>> {
        let Some(value) = saved["plan"].get("action") else {
            return Ok(None);
        };
        let action: Self = serde_json::from_value(value.clone()).map_err(|_| invalid())?;
        if action != Self::new(inputs, action.count, &action.kind)? {
            return Err(invalid());
        }
        Ok(Some(action))
    }
    /// Covered-prefix index expected before this adoption action.
    pub fn base_count(&self) -> usize {
        if self.kind == "import_legacy_v1" {
            self.count
        } else {
            0
        }
    }
    /// Validate original ledger eligibility without mutating or repairing it.
    pub fn before(&self, inputs: &CapturedInputs, history: &History) -> Result<()> {
        const NEVER_FORCE: &str = "never delete, reset or rewrite a ledger to force adoption";
        if self.kind == "import_legacy_v1" {
            if !history.present || history.scope.is_some() || inputs.config.engine != "postgresql" {
                return Err(invalid());
            }
            if history.entries.is_empty() {
                return Err(refuse(
                    "The target's legacy v1 ledger is empty, so it records no history to import. No history is inferred or repaired.",
                    &format!("Inspect _ntnt.schema_migrations_v1 on the target with an operator: an empty legacy ledger is not eligible for adoption; {NEVER_FORCE}."),
                ));
            }
            if history.entries.iter().any(|e| e.state != "applied") {
                return Err(refuse(
                    "The target's legacy v1 ledger records migrations that are not applied, so it is not eligible for import. No history is inferred or repaired.",
                    &format!("Investigate the unapplied entries in _ntnt.schema_migrations_v1 with an operator; {NEVER_FORCE}."),
                ));
            }
            let covered = super::report::prefix(history, inputs)?;
            if covered != self.count {
                let last = history.entries.last().map_or("", |e| e.id.as_str());
                return Err(refuse(
                    &format!(
                        "The target's legacy v1 ledger records {covered} migration(s), through ID {last}, but --adopt-through {} covers {}; a legacy ledger is imported only through its last migration.",
                        self.through_id, self.count
                    ),
                    &format!("Pass --adopt-through {last}, the legacy ledger's last migration ID."),
                ));
            }
        } else if history.present || !history.entries.is_empty() || history.scope.is_some() {
            let entries = history.entries.len();
            return Err(refuse(
                &format!(
                    "The target already has an ntnt migration ledger ({entries} {}), so it is not eligible for adoption, which only records a baseline for a database without one.",
                    if entries == 1 { "entry" } else { "entries" }
                ),
                &format!("Its history is already recorded: plan and migrate without --adopt-through; {NEVER_FORCE}."),
            ));
        }
        Ok(())
    }
    /// Exact expected ledger for post-write validation and fresh reconciliation.
    pub fn after(&self, inputs: &CapturedInputs) -> Result<History> {
        Ok(History {
            present: true,
            scope: Some(json!({"database":inputs.database_name,"schema":inputs.config.schema})),
            entries: if self.kind == "import_legacy_v1" {
                inputs
                    .migrations
                    .iter()
                    .take(self.count)
                    .enumerate()
                    .map(|(i, m)| super::model::HistoryEntry {
                        position: i as i64 + 1,
                        id: m.id.to_string(),
                        name: m.name.clone(),
                        checksum: m.checksum.clone(),
                        state: "imported".into(),
                    })
                    .collect()
            } else {
                vec![inputs.checkpoint(self.count)?]
            },
        })
    }
}
pub(super) fn format(engine: &str) -> &'static str {
    if engine == "sqlite" {
        "sqlite-adoption-v1"
    } else {
        "postgresql-adoption-v1"
    }
}
