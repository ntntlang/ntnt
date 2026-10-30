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
impl Adoption {
    fn new(inputs: &CapturedInputs, count: usize, kind: &str) -> Result<Self> {
        if inputs.config.adoption_checks.is_empty()
            || !(kind == "adopt_baseline"
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
            .ok_or_else(invalid)?;
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
        if self.kind == "import_legacy_v1" {
            if !history.present
                || history.scope.is_some()
                || inputs.config.engine != "postgresql"
                || history.entries.iter().any(|e| e.state != "applied")
                || super::report::prefix(history, inputs)? != self.count
            {
                return Err(invalid());
            }
        } else if history.present || !history.entries.is_empty() || history.scope.is_some() {
            return Err(invalid());
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
