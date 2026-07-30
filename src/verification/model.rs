use serde::Serialize;

use crate::intent::{FeatureVerification, IntentFile};

use super::ids::{SourceSpan, StableId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum DeclarationStatus {
    Declared,
    DocumentationOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum LinkageStatus {
    Linked,
    Unlinked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum BindingStatus {
    Bound,
    Unbound,
    Ambiguous,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum ExecutabilityStatus {
    Executable,
    Unsupported,
    Blocked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Disposition {
    Planned,
    Running,
    Passed,
    Failed,
    Flaky,
    Skipped,
    Cancelled,
    NoResult,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Freshness {
    Current,
    Stale,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum AssertionResolution {
    Resolved,
    Unknown,
    Unresolved,
}

/// One candidate source of execution evidence for an obligation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EvidenceBinding {
    pub id: String,
    pub obligation_id: String,
    pub source: SourceSpan,
    pub declaration: DeclarationStatus,
    pub linkage: LinkageStatus,
    pub binding: BindingStatus,
    pub executability: ExecutabilityStatus,
    pub disposition: Disposition,
    pub freshness: Freshness,
    pub assertion_resolution: AssertionResolution,
    pub evidence_atoms: usize,
}

impl EvidenceBinding {
    /// Passing execution is evidence only when its assertion was resolved and
    /// produced at least one current evidence atom.
    pub fn satisfies_obligation(&self) -> bool {
        self.declaration == DeclarationStatus::Declared
            && self.binding == BindingStatus::Bound
            && self.executability == ExecutabilityStatus::Executable
            && self.disposition == Disposition::Passed
            && self.freshness == Freshness::Current
            && self.assertion_resolution == AssertionResolution::Resolved
            && self.evidence_atoms > 0
    }
}

/// A source-located behavioral claim produced from one scenario outcome.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Obligation {
    pub id: StableId,
    pub feature_id: StableId,
    pub scenario_id: StableId,
    pub statement: String,
    pub source: SourceSpan,
    pub declaration: DeclarationStatus,
    pub linkage: LinkageStatus,
    pub binding: BindingStatus,
    pub executability: ExecutabilityStatus,
    pub disposition: Disposition,
    pub freshness: Freshness,
    pub evidence_bindings: Vec<EvidenceBinding>,
}

impl Obligation {
    pub fn is_verified(&self) -> bool {
        self.declaration == DeclarationStatus::Declared
            && self.binding == BindingStatus::Bound
            && self.executability == ExecutabilityStatus::Executable
            && self.disposition == Disposition::Passed
            && self.freshness == Freshness::Current
            && !self.evidence_bindings.is_empty()
            && self
                .evidence_bindings
                .iter()
                .all(EvidenceBinding::satisfies_obligation)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum FeatureProofStatus {
    Unproven,
    Pending,
    DocumentationOnly,
}

/// Feature-level truth remains visible even when there are no outcome obligations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FeatureTruth {
    pub id: StableId,
    pub name: String,
    pub source: SourceSpan,
    pub declaration: DeclarationStatus,
    pub rationale: Option<String>,
    pub proof_status: FeatureProofStatus,
    pub obligation_ids: Vec<StableId>,
}

impl FeatureTruth {
    pub fn is_unproven(&self) -> bool {
        self.proof_status == FeatureProofStatus::Unproven
    }
}

macro_rules! coverage_type {
    ($name:ident) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
        pub struct $name {
            pub covered: usize,
            pub total: usize,
        }
    };
}

coverage_type!(ImplementationCoverage);
coverage_type!(ExecutableCoverage);
coverage_type!(VerifiedCoverage);

/// Slice 1A's in-memory truth model. Rendering and exit policy intentionally
/// consume this in later slices rather than living here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VerificationTruth {
    pub features: Vec<FeatureTruth>,
    pub obligations: Vec<Obligation>,
}

impl VerificationTruth {
    pub fn from_intent(intent: &IntentFile) -> Self {
        let mut features = Vec::new();
        let mut obligations = Vec::new();

        for feature in &intent.features {
            let feature_id = feature.verification_id.clone();
            let (declaration, rationale) = match &feature.verification {
                FeatureVerification::Behavioral => (DeclarationStatus::Declared, None),
                FeatureVerification::DocumentationOnly { rationale } => (
                    DeclarationStatus::DocumentationOnly,
                    Some(rationale.clone()),
                ),
            };
            let mut obligation_ids = Vec::new();

            if declaration == DeclarationStatus::Declared {
                for scenario in &feature.scenarios {
                    let scenario_id = scenario.verification_id.clone();
                    for (statement, metadata) in
                        scenario.outcomes.iter().zip(&scenario.outcome_metadata)
                    {
                        obligation_ids.push(metadata.id.clone());
                        obligations.push(Obligation {
                            id: metadata.id.clone(),
                            feature_id: feature_id.clone(),
                            scenario_id: scenario_id.clone(),
                            statement: statement.clone(),
                            source: metadata.source.clone(),
                            declaration,
                            linkage: LinkageStatus::Unlinked,
                            binding: BindingStatus::Unbound,
                            executability: ExecutabilityStatus::Unsupported,
                            disposition: Disposition::NoResult,
                            freshness: Freshness::Current,
                            evidence_bindings: Vec::new(),
                        });
                    }
                }
            }

            let proof_status = if declaration == DeclarationStatus::DocumentationOnly {
                FeatureProofStatus::DocumentationOnly
            } else if obligation_ids.is_empty() {
                FeatureProofStatus::Unproven
            } else {
                FeatureProofStatus::Pending
            };
            features.push(FeatureTruth {
                id: feature_id,
                name: feature.name.clone(),
                source: feature.source.clone(),
                declaration,
                rationale,
                proof_status,
                obligation_ids,
            });
        }

        Self {
            features,
            obligations,
        }
    }

    pub fn behavioral_feature_count(&self) -> usize {
        self.features
            .iter()
            .filter(|feature| feature.declaration == DeclarationStatus::Declared)
            .count()
    }

    /// A feature is verified only when it has at least one required obligation
    /// and every one of those obligations is verified.
    pub fn feature_is_verified(&self, feature_id: &str) -> bool {
        let Some(feature) = self
            .features
            .iter()
            .find(|feature| feature.id.as_str() == feature_id)
        else {
            return false;
        };
        feature.declaration == DeclarationStatus::Declared
            && !feature.obligation_ids.is_empty()
            && feature.obligation_ids.iter().all(|obligation_id| {
                self.obligations
                    .iter()
                    .any(|obligation| obligation.id == *obligation_id && obligation.is_verified())
            })
    }

    pub fn mark_implementation_linked(&mut self, obligation_id: &str) -> Result<(), String> {
        let obligation = self
            .obligations
            .iter_mut()
            .find(|obligation| obligation.id.as_str() == obligation_id)
            .ok_or_else(|| format!("unknown obligation ID '{obligation_id}'"))?;
        obligation.linkage = LinkageStatus::Linked;
        Ok(())
    }

    pub fn implementation_coverage(&self) -> ImplementationCoverage {
        ImplementationCoverage {
            covered: self
                .obligations
                .iter()
                .filter(|obligation| obligation.linkage == LinkageStatus::Linked)
                .count(),
            total: self.obligations.len(),
        }
    }

    pub fn executable_coverage(&self) -> ExecutableCoverage {
        ExecutableCoverage {
            covered: self
                .obligations
                .iter()
                .filter(|obligation| {
                    obligation.binding == BindingStatus::Bound
                        && obligation.executability == ExecutabilityStatus::Executable
                })
                .count(),
            total: self.obligations.len(),
        }
    }

    pub fn verified_coverage(&self) -> VerifiedCoverage {
        VerifiedCoverage {
            covered: self
                .obligations
                .iter()
                .filter(|obligation| obligation.is_verified())
                .count(),
            total: self.obligations.len(),
        }
    }
}
