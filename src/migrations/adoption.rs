//! Explicit baseline identity, distinct from evidence of executed migrations.
use super::inventory::CapturedInputs;
use super::model::History;
use super::{Diagnostic, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Adoption {
    pub kind: String,
    pub through_id: String,
    pub count: usize,
    pub prefix_checksum: String,
    pub checks: Vec<String>,
}
fn invalid() -> Diagnostic {
    Diagnostic::new("DB_ADOPTION_INPUT","Adoption requires an exact captured prefix, named adoption_checks, and an unrecognized target with no ledger. No history is inferred or repaired.")
}
impl Adoption {
    fn new(inputs: &CapturedInputs, count: usize) -> Result<Self> {
        if inputs.config.adoption_checks.is_empty() {
            return Err(invalid());
        }
        let checkpoint = inputs.checkpoint(count)?;
        Ok(Self {
            kind: "adopt_baseline".into(),
            through_id: checkpoint.id,
            count,
            prefix_checksum: checkpoint.checksum,
            checks: inputs.config.adoption_checks.keys().cloned().collect(),
        })
    }
    pub fn selected(inputs: &CapturedInputs, through: u64) -> Result<Self> {
        let count = inputs
            .migrations
            .iter()
            .position(|m| m.id == through)
            .map(|n| n + 1)
            .ok_or_else(invalid)?;
        Self::new(inputs, count)
    }
    pub fn request(inputs: &CapturedInputs, through: u64, history: &History) -> Result<Self> {
        let action = Self::selected(inputs, through)?;
        action.before(history)?;
        Ok(action)
    }
    pub fn from_plan(inputs: &CapturedInputs, saved: &Value) -> Result<Option<Self>> {
        let Some(value) = saved["plan"].get("action") else {
            return Ok(None);
        };
        let action: Self = serde_json::from_value(value.clone()).map_err(|_| invalid())?;
        if action != Self::new(inputs, action.count)? {
            return Err(invalid());
        }
        Ok(Some(action))
    }
    pub fn before(&self, history: &History) -> Result<()> {
        if history.present || !history.entries.is_empty() || history.scope.is_some() {
            return Err(invalid());
        }
        Ok(())
    }
    pub fn after(&self, inputs: &CapturedInputs) -> Result<History> {
        Ok(History {
            present: true,
            scope: Some(json!({"database":inputs.database_name,"schema":inputs.config.schema})),
            entries: vec![inputs.checkpoint(self.count)?],
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
