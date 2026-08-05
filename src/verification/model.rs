use std::collections::HashMap;

use crate::intent::{FeatureVerification, IdentifiedIntent, ScenarioOrigin};

use super::ids::{IdKind, SourceSpan, StableId};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeclarationStatus {
    Declared,
    DocumentationOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkageStatus {
    Linked,
    Unlinked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindingStatus {
    Bound,
    Unbound,
    Ambiguous,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutabilityStatus {
    Executable,
    Unsupported,
    Blocked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Freshness {
    Current,
    Stale,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssertionResolution {
    Resolved,
    Unknown,
    Unresolved,
}

/// One candidate source of execution evidence for an obligation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvidenceBinding {
    id: String,
    obligation_id: StableId,
    source: SourceSpan,
    declaration: DeclarationStatus,
    linkage: LinkageStatus,
    binding: BindingStatus,
    executability: ExecutabilityStatus,
    disposition: Disposition,
    freshness: Freshness,
    assertion_resolution: AssertionResolution,
    evidence_atoms: usize,
}

impl EvidenceBinding {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn obligation_id(&self) -> &StableId {
        &self.obligation_id
    }

    pub fn source(&self) -> &SourceSpan {
        &self.source
    }

    pub fn declaration(&self) -> DeclarationStatus {
        self.declaration
    }

    pub fn linkage(&self) -> LinkageStatus {
        self.linkage
    }

    pub fn binding(&self) -> BindingStatus {
        self.binding
    }

    pub fn executability(&self) -> ExecutabilityStatus {
        self.executability
    }

    pub fn disposition(&self) -> Disposition {
        self.disposition
    }

    pub fn freshness(&self) -> Freshness {
        self.freshness
    }

    pub fn assertion_resolution(&self) -> AssertionResolution {
        self.assertion_resolution
    }

    pub fn evidence_atoms(&self) -> usize {
        self.evidence_atoms
    }

    /// Passing execution is evidence only when its assertion was resolved and
    /// produced at least one current evidence atom.
    pub fn satisfies_obligation(&self, obligation_id: &StableId) -> bool {
        self.obligation_id.kind() == IdKind::Outcome
            && obligation_id.kind() == IdKind::Outcome
            && self.obligation_id == *obligation_id
            && self.declaration == DeclarationStatus::Declared
            && self.binding == BindingStatus::Bound
            && self.executability == ExecutabilityStatus::Executable
            && self.disposition == Disposition::Passed
            && self.freshness == Freshness::Current
            && self.assertion_resolution == AssertionResolution::Resolved
            && self.evidence_atoms > 0
    }
}

/// A source-located behavioral claim produced from one scenario outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Obligation {
    id: StableId,
    feature_id: StableId,
    scenario_id: StableId,
    statement: String,
    source: SourceSpan,
    declaration: DeclarationStatus,
    linkage: LinkageStatus,
    binding: BindingStatus,
    executability: ExecutabilityStatus,
    disposition: Disposition,
    freshness: Freshness,
    evidence_bindings: Vec<EvidenceBinding>,
}

impl Obligation {
    pub fn id(&self) -> &StableId {
        &self.id
    }

    pub fn feature_id(&self) -> &StableId {
        &self.feature_id
    }

    pub fn scenario_id(&self) -> &StableId {
        &self.scenario_id
    }

    pub fn statement(&self) -> &str {
        &self.statement
    }

    pub fn source(&self) -> &SourceSpan {
        &self.source
    }

    pub fn declaration(&self) -> DeclarationStatus {
        self.declaration
    }

    pub fn linkage(&self) -> LinkageStatus {
        self.linkage
    }

    pub fn binding(&self) -> BindingStatus {
        self.binding
    }

    pub fn executability(&self) -> ExecutabilityStatus {
        self.executability
    }

    pub fn disposition(&self) -> Disposition {
        self.disposition
    }

    pub fn freshness(&self) -> Freshness {
        self.freshness
    }

    pub fn evidence_bindings(&self) -> &[EvidenceBinding] {
        &self.evidence_bindings
    }

    pub fn is_verified(&self) -> bool {
        self.id.kind() == IdKind::Outcome
            && self.feature_id.kind() == IdKind::Feature
            && self.scenario_id.kind() == IdKind::Scenario
            && self.declaration == DeclarationStatus::Declared
            && self.binding == BindingStatus::Bound
            && self.executability == ExecutabilityStatus::Executable
            && self.disposition == Disposition::Passed
            && self.freshness == Freshness::Current
            && !self.evidence_bindings.is_empty()
            && self
                .evidence_bindings
                .iter()
                .all(|binding| binding.satisfies_obligation(&self.id))
    }
}

/// Feature-level truth remains visible even when there are no outcome obligations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeatureTruth {
    id: StableId,
    name: String,
    source: SourceSpan,
    declaration: DeclarationStatus,
    rationale: Option<String>,
    obligation_ids: Vec<StableId>,
    /// Legacy `test:` cases that do not yet have stable obligation identities.
    unrepresented_legacy_tests: usize,
}

impl FeatureTruth {
    pub fn id(&self) -> &StableId {
        &self.id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn source(&self) -> &SourceSpan {
        &self.source
    }

    pub fn declaration(&self) -> DeclarationStatus {
        self.declaration
    }

    pub fn rationale(&self) -> Option<&str> {
        self.rationale.as_deref()
    }

    pub fn obligation_ids(&self) -> &[StableId] {
        &self.obligation_ids
    }

    pub fn unrepresented_legacy_test_count(&self) -> usize {
        self.unrepresented_legacy_tests
    }

    pub fn is_unproven(&self) -> bool {
        self.declaration == DeclarationStatus::Declared
            && (self.obligation_ids.is_empty() || self.unrepresented_legacy_tests > 0)
    }
}

macro_rules! coverage_type {
    ($name:ident) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub struct $name {
            pub covered: usize,
            pub total: usize,
        }
    };
}

coverage_type!(ImplementationCoverage);
coverage_type!(ExecutableCoverage);
coverage_type!(VerifiedCoverage);

/// Slice 1A's read-only in-memory truth model. Rendering and evidence-ingestion
/// authority intentionally live outside this foundation.
///
/// Caller code cannot rewrite the parser-owned denominator or truth status:
///
/// ```compile_fail
/// use ntnt::verification::VerificationTruth;
///
/// fn suppress_required_truth(truth: &mut VerificationTruth) {
///     truth.obligations.clear();
///     truth.features[0].unrepresented_legacy_tests = 0;
/// }
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerificationTruth {
    features: Vec<FeatureTruth>,
    obligations: Vec<Obligation>,
    canonical_features: Vec<CanonicalFeature>,
    canonical_obligations: Vec<CanonicalObligation>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CanonicalFeature {
    id: StableId,
    name: String,
    source: SourceSpan,
    declaration: DeclarationStatus,
    rationale: Option<String>,
    obligation_ids: Vec<StableId>,
    unrepresented_legacy_tests: usize,
}

impl CanonicalFeature {
    fn from_feature(feature: &FeatureTruth) -> Self {
        Self {
            id: feature.id.clone(),
            name: feature.name.clone(),
            source: feature.source.clone(),
            declaration: feature.declaration,
            rationale: feature.rationale.clone(),
            obligation_ids: feature.obligation_ids.clone(),
            unrepresented_legacy_tests: feature.unrepresented_legacy_tests,
        }
    }

    fn matches(&self, feature: &FeatureTruth) -> bool {
        self.id == feature.id
            && self.name == feature.name
            && self.source == feature.source
            && self.declaration == feature.declaration
            && self.rationale == feature.rationale
            && self.obligation_ids == feature.obligation_ids
            && self.unrepresented_legacy_tests == feature.unrepresented_legacy_tests
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CanonicalObligation {
    id: StableId,
    feature_id: StableId,
    scenario_id: StableId,
    statement: String,
    source: SourceSpan,
    declaration: DeclarationStatus,
}

impl CanonicalObligation {
    fn from_obligation(obligation: &Obligation) -> Self {
        Self {
            id: obligation.id.clone(),
            feature_id: obligation.feature_id.clone(),
            scenario_id: obligation.scenario_id.clone(),
            statement: obligation.statement.clone(),
            source: obligation.source.clone(),
            declaration: obligation.declaration,
        }
    }

    fn matches(&self, obligation: &Obligation) -> bool {
        self.id == obligation.id
            && self.feature_id == obligation.feature_id
            && self.scenario_id == obligation.scenario_id
            && self.statement == obligation.statement
            && self.source == obligation.source
            && self.declaration == obligation.declaration
    }
}

impl VerificationTruth {
    pub fn from_intent(intent: &IdentifiedIntent) -> Result<Self, String> {
        let mut features = Vec::new();
        let mut obligations = Vec::new();
        let metadata = intent.metadata();

        if intent.features.len() != metadata.features.len() {
            return Err(format!(
                "feature metadata count {} does not match parsed feature count {}",
                metadata.features.len(),
                intent.features.len()
            ));
        }

        for (feature, feature_metadata) in intent.features.iter().zip(&metadata.features) {
            if feature_metadata.id.kind() != IdKind::Feature {
                return Err(format!(
                    "{}: feature identity '{}' has the wrong stable ID kind",
                    feature_metadata.id_source, feature_metadata.id
                ));
            }
            let feature_id = feature_metadata.id.clone();
            let (declaration, rationale) = match &feature_metadata.verification {
                FeatureVerification::Behavioral => (DeclarationStatus::Declared, None),
                FeatureVerification::DocumentationOnly { rationale } => (
                    DeclarationStatus::DocumentationOnly,
                    Some(rationale.clone()),
                ),
            };
            let mut obligation_ids = Vec::new();

            if feature.scenarios.len() != feature_metadata.scenarios.len() {
                return Err(format!(
                    "{}: scenario metadata count {} does not match parsed scenario count {} for feature '{}'",
                    feature_metadata.source,
                    feature_metadata.scenarios.len(),
                    feature.scenarios.len(),
                    feature.name
                ));
            }

            if declaration == DeclarationStatus::Declared {
                for (scenario, scenario_metadata) in
                    feature.scenarios.iter().zip(&feature_metadata.scenarios)
                {
                    if scenario_metadata.origin != ScenarioOrigin::Feature {
                        continue;
                    }
                    if scenario_metadata.id.kind() != IdKind::Scenario {
                        return Err(format!(
                            "{}: scenario identity '{}' has the wrong stable ID kind",
                            scenario_metadata.id_source, scenario_metadata.id
                        ));
                    }
                    if scenario.outcomes.len() != scenario_metadata.outcomes.len() {
                        return Err(format!(
                            "{}: outcome metadata count {} does not match declared outcome count {} for scenario '{}'",
                            scenario_metadata.source,
                            scenario_metadata.outcomes.len(),
                            scenario.outcomes.len(),
                            scenario.name
                        ));
                    }
                    let scenario_id = scenario_metadata.id.clone();
                    for (statement, outcome_metadata) in
                        scenario.outcomes.iter().zip(&scenario_metadata.outcomes)
                    {
                        if outcome_metadata.id.kind() != IdKind::Outcome {
                            return Err(format!(
                                "{}: outcome identity '{}' has the wrong stable ID kind",
                                outcome_metadata.source, outcome_metadata.id
                            ));
                        }
                        obligation_ids.push(outcome_metadata.id.clone());
                        obligations.push(Obligation {
                            id: outcome_metadata.id.clone(),
                            feature_id: feature_id.clone(),
                            scenario_id: scenario_id.clone(),
                            statement: statement.clone(),
                            source: outcome_metadata.source.clone(),
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

            features.push(FeatureTruth {
                id: feature_id,
                name: feature.name.clone(),
                source: feature_metadata.source.clone(),
                declaration,
                rationale,
                obligation_ids,
                unrepresented_legacy_tests: feature.tests.len(),
            });
        }

        let canonical_features = features
            .iter()
            .map(CanonicalFeature::from_feature)
            .collect();
        let canonical_obligations = obligations
            .iter()
            .map(CanonicalObligation::from_obligation)
            .collect();

        Ok(Self {
            features,
            obligations,
            canonical_features,
            canonical_obligations,
        })
    }

    pub fn features(&self) -> &[FeatureTruth] {
        &self.features
    }

    pub fn obligations(&self) -> &[Obligation] {
        &self.obligations
    }

    pub fn behavioral_feature_count(&self) -> usize {
        self.canonical_features
            .iter()
            .filter(|feature| feature.declaration == DeclarationStatus::Declared)
            .count()
    }

    /// A feature is verified only when it has at least one required obligation,
    /// every obligation belongs to that feature, and all IDs are globally unique.
    pub fn feature_is_verified(&self, feature_id: &str) -> bool {
        if !self.canonical_topology_is_intact() {
            return false;
        }

        let obligations_by_id = self
            .obligations
            .iter()
            .map(|obligation| (&obligation.id, obligation))
            .collect::<HashMap<_, _>>();

        let Some(feature) = self
            .features
            .iter()
            .find(|feature| feature.id.as_str() == feature_id)
        else {
            return false;
        };
        feature.declaration == DeclarationStatus::Declared
            && feature.unrepresented_legacy_tests == 0
            && !feature.obligation_ids.is_empty()
            && feature.obligation_ids.iter().all(|obligation_id| {
                obligations_by_id
                    .get(obligation_id)
                    .is_some_and(|obligation| {
                        obligation.feature_id == feature.id && obligation.is_verified()
                    })
            })
    }

    fn canonical_topology_is_intact(&self) -> bool {
        if self.features.len() != self.canonical_features.len()
            || self.obligations.len() != self.canonical_obligations.len()
        {
            return false;
        }

        let mut features_by_id = HashMap::new();
        for feature in &self.features {
            if features_by_id.insert(&feature.id, feature).is_some() {
                return false;
            }
        }
        if !self.canonical_features.iter().all(|canonical| {
            features_by_id
                .get(&canonical.id)
                .is_some_and(|feature| canonical.matches(feature))
        }) {
            return false;
        }

        let mut obligations_by_id = HashMap::new();
        for obligation in &self.obligations {
            if obligations_by_id
                .insert(&obligation.id, obligation)
                .is_some()
            {
                return false;
            }
        }
        self.canonical_obligations.iter().all(|canonical| {
            obligations_by_id
                .get(&canonical.id)
                .is_some_and(|obligation| canonical.matches(obligation))
        })
    }

    #[cfg(test)]
    fn mark_implementation_linked(&mut self, obligation_id: &str) -> Result<(), String> {
        let match_count = self
            .obligations
            .iter()
            .filter(|obligation| obligation.id.as_str() == obligation_id)
            .count();
        match match_count {
            0 => return Err(format!("unknown obligation ID '{obligation_id}'")),
            1 => {}
            _ => return Err(format!("duplicate obligation ID '{obligation_id}'")),
        }
        let obligation = self
            .obligations
            .iter_mut()
            .find(|obligation| obligation.id.as_str() == obligation_id)
            .expect("exactly one obligation matched above");
        obligation.linkage = LinkageStatus::Linked;
        Ok(())
    }

    pub fn implementation_coverage(&self) -> ImplementationCoverage {
        let total = self.canonical_obligations.len();
        if !self.canonical_topology_is_intact() {
            return ImplementationCoverage { covered: 0, total };
        }
        ImplementationCoverage {
            covered: self
                .obligations
                .iter()
                .filter(|obligation| obligation.linkage == LinkageStatus::Linked)
                .count(),
            total,
        }
    }

    pub fn executable_coverage(&self) -> ExecutableCoverage {
        let total = self.canonical_obligations.len();
        if !self.canonical_topology_is_intact() {
            return ExecutableCoverage { covered: 0, total };
        }
        ExecutableCoverage {
            covered: self
                .obligations
                .iter()
                .filter(|obligation| {
                    obligation.binding == BindingStatus::Bound
                        && obligation.executability == ExecutabilityStatus::Executable
                })
                .count(),
            total,
        }
    }

    pub fn verified_coverage(&self) -> VerifiedCoverage {
        let total = self.canonical_obligations.len();
        if !self.canonical_topology_is_intact() {
            return VerifiedCoverage { covered: 0, total };
        }
        VerifiedCoverage {
            covered: self
                .obligations
                .iter()
                .filter(|obligation| obligation.is_verified())
                .count(),
            total,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::intent::IntentFile;
    use crate::verification::IdMode;

    fn strict_truth(content: &str) -> VerificationTruth {
        let intent = IntentFile::parse_content_with_id_mode(
            content,
            "verification-model-test.intent".to_string(),
            IdMode::Strict,
        )
        .expect("test intent must parse");
        VerificationTruth::from_intent(&intent).expect("test truth must build")
    }

    fn mark_verified(obligation: &mut Obligation, binding_id: &str) {
        obligation.binding = BindingStatus::Bound;
        obligation.executability = ExecutabilityStatus::Executable;
        obligation.disposition = Disposition::Passed;
        obligation.freshness = Freshness::Current;
        obligation.evidence_bindings.push(EvidenceBinding {
            id: binding_id.to_string(),
            obligation_id: obligation.id.clone(),
            source: SourceSpan::single_line("verification.tnt", 1, 1, 2),
            declaration: DeclarationStatus::Declared,
            linkage: LinkageStatus::Unlinked,
            binding: BindingStatus::Bound,
            executability: ExecutabilityStatus::Executable,
            disposition: Disposition::Passed,
            freshness: Freshness::Current,
            assertion_resolution: AssertionResolution::Resolved,
            evidence_atoms: 1,
        });
    }

    fn passing_binding(obligation_id: StableId, binding_id: &str) -> EvidenceBinding {
        EvidenceBinding {
            id: binding_id.to_string(),
            obligation_id,
            source: SourceSpan::single_line("verification.tnt", 1, 1, 2),
            declaration: DeclarationStatus::Declared,
            linkage: LinkageStatus::Unlinked,
            binding: BindingStatus::Bound,
            executability: ExecutabilityStatus::Executable,
            disposition: Disposition::Passed,
            freshness: Freshness::Current,
            assertion_resolution: AssertionResolution::Resolved,
            evidence_atoms: 1,
        }
    }

    #[test]
    fn feature_and_obligation_topology_and_statuses_are_readable() {
        let truth = strict_truth(
            r#"Feature: Required claim
  id: feature.required-claim

  Scenario: One claim
    id: scenario.required-claim.one
    When the behavior runs
    → id: outcome.required-claim.one; the claim holds
"#,
        );
        let feature = &truth.features()[0];
        assert_eq!(feature.id().as_str(), "feature.required-claim");
        assert_eq!(feature.name(), "Required claim");
        assert_eq!(feature.source().path, "verification-model-test.intent");
        assert_eq!(feature.declaration(), DeclarationStatus::Declared);
        assert_eq!(feature.rationale(), None);
        assert_eq!(feature.obligation_ids().len(), 1);
        assert_eq!(feature.unrepresented_legacy_test_count(), 0);

        let obligation = &truth.obligations()[0];
        assert_eq!(obligation.id().as_str(), "outcome.required-claim.one");
        assert_eq!(obligation.feature_id(), feature.id());
        assert_eq!(
            obligation.scenario_id().as_str(),
            "scenario.required-claim.one"
        );
        assert_eq!(obligation.statement(), "the claim holds");
        assert_eq!(obligation.source().path, "verification-model-test.intent");
        assert_eq!(obligation.declaration(), DeclarationStatus::Declared);
        assert_eq!(obligation.linkage(), LinkageStatus::Unlinked);
        assert_eq!(obligation.binding(), BindingStatus::Unbound);
        assert_eq!(obligation.executability(), ExecutabilityStatus::Unsupported);
        assert_eq!(obligation.disposition(), Disposition::NoResult);
        assert_eq!(obligation.freshness(), Freshness::Current);
        assert!(obligation.evidence_bindings().is_empty());
        assert!(!obligation.is_verified());
    }

    #[test]
    fn duplicate_obligations_fail_closed_in_truth_predicates() {
        let mut truth = strict_truth(
            r#"Feature: Required claim
  id: feature.required-claim

  Scenario: One claim
    id: scenario.required-claim.one
    When the behavior runs
    → id: outcome.required-claim.one; the claim holds
"#,
        );
        let feature_id = truth.features[0].id.clone();
        mark_verified(&mut truth.obligations[0], "binding.original");

        let mut conflicting = truth.obligations[0].clone();
        conflicting.binding = BindingStatus::Unbound;
        conflicting.executability = ExecutabilityStatus::Unsupported;
        conflicting.disposition = Disposition::Failed;
        conflicting.evidence_bindings.clear();
        truth.obligations.push(conflicting);

        assert!(!truth.feature_is_verified(feature_id.as_str()));
        assert_eq!(
            truth.verified_coverage(),
            VerifiedCoverage {
                covered: 0,
                total: 1,
            }
        );
    }

    #[test]
    fn linkage_and_execution_coverage_remain_distinct() {
        let mut truth = strict_truth(
            r#"Feature: Required claim
  id: feature.required-claim

  Scenario: One claim
    id: scenario.required-claim.one
    When the behavior runs
    → id: outcome.required-claim.one; the claim holds
"#,
        );
        truth
            .mark_implementation_linked("outcome.required-claim.one")
            .unwrap();

        assert_eq!(
            truth.implementation_coverage(),
            ImplementationCoverage {
                covered: 1,
                total: 1,
            }
        );
        assert_eq!(
            truth.executable_coverage(),
            ExecutableCoverage {
                covered: 0,
                total: 1,
            }
        );
        assert_eq!(
            truth.verified_coverage(),
            VerifiedCoverage {
                covered: 0,
                total: 1,
            }
        );
    }

    #[test]
    fn zero_atoms_and_unknown_or_unresolved_assertions_fail_closed() {
        let obligation_id = StableId::explicit("outcome.example", IdKind::Outcome).unwrap();
        for resolution in [
            AssertionResolution::Unknown,
            AssertionResolution::Unresolved,
        ] {
            let mut binding = passing_binding(obligation_id.clone(), "binding.resolution");
            binding.assertion_resolution = resolution;
            assert!(!binding.satisfies_obligation(&obligation_id));
        }

        let mut binding = passing_binding(obligation_id.clone(), "binding.zero-atoms");
        binding.evidence_atoms = 0;
        assert!(!binding.satisfies_obligation(&obligation_id));
    }

    #[test]
    fn truth_dimensions_remain_orthogonal_and_readable() {
        let obligation_id = StableId::explicit("outcome.example", IdKind::Outcome).unwrap();
        let binding = EvidenceBinding {
            id: "binding.blocked".to_string(),
            obligation_id: obligation_id.clone(),
            source: SourceSpan::single_line("verification.tnt", 7, 1, 12),
            declaration: DeclarationStatus::Declared,
            linkage: LinkageStatus::Linked,
            binding: BindingStatus::Bound,
            executability: ExecutabilityStatus::Blocked,
            disposition: Disposition::Planned,
            freshness: Freshness::Stale,
            assertion_resolution: AssertionResolution::Resolved,
            evidence_atoms: 0,
        };

        assert_eq!(binding.id(), "binding.blocked");
        assert_eq!(binding.obligation_id(), &obligation_id);
        assert_eq!(binding.source().start_line, 7);
        assert_eq!(binding.declaration(), DeclarationStatus::Declared);
        assert_eq!(binding.linkage(), LinkageStatus::Linked);
        assert_eq!(binding.binding(), BindingStatus::Bound);
        assert_eq!(binding.executability(), ExecutabilityStatus::Blocked);
        assert_eq!(binding.disposition(), Disposition::Planned);
        assert_eq!(binding.freshness(), Freshness::Stale);
        assert_eq!(
            binding.assertion_resolution(),
            AssertionResolution::Resolved
        );
        assert_eq!(binding.evidence_atoms(), 0);
        assert!(!binding.satisfies_obligation(&obligation_id));
    }

    #[test]
    fn unlinked_behavioral_evidence_verifies_only_its_exact_outcome_id_and_kind() {
        let mut truth = strict_truth(
            r#"Feature: Required claim
  id: feature.required-claim

  Scenario: One claim
    id: scenario.required-claim.one
    When the behavior runs
    → id: outcome.required-claim.one; the claim holds
"#,
        );
        let obligation = &mut truth.obligations[0];
        let binding = passing_binding(obligation.id.clone(), "binding.behavioral");

        assert_eq!(binding.linkage(), LinkageStatus::Unlinked);
        assert!(binding.satisfies_obligation(&obligation.id));
        let unrelated = StableId::explicit("outcome.unrelated", IdKind::Outcome).unwrap();
        assert!(!binding.satisfies_obligation(&unrelated));
        let wrong_kind = StableId::explicit("scenario.wrong-kind", IdKind::Scenario).unwrap();
        assert!(!binding.satisfies_obligation(&wrong_kind));
        let mut wrong_kind_binding = binding.clone();
        wrong_kind_binding.obligation_id = wrong_kind;
        assert!(!wrong_kind_binding.satisfies_obligation(&obligation.id));

        obligation.linkage = LinkageStatus::Unlinked;
        obligation.binding = BindingStatus::Bound;
        obligation.executability = ExecutabilityStatus::Executable;
        obligation.disposition = Disposition::Passed;
        obligation.freshness = Freshness::Current;
        obligation.evidence_bindings.push(binding);

        assert!(obligation.is_verified());
        obligation.evidence_bindings[0].obligation_id = unrelated;
        assert!(
            !obligation.is_verified(),
            "evidence for another obligation must fail closed"
        );
    }

    #[test]
    fn feature_verification_rejects_wrong_feature_ownership_and_id_kind() {
        let mut truth = strict_truth(
            r#"Feature: Required claim
  id: feature.required-claim

  Scenario: One claim
    id: scenario.required-claim.one
    When the behavior runs
    → id: outcome.required-claim.one; the claim holds
"#,
        );
        let feature_id = truth.features[0].id.clone();
        mark_verified(&mut truth.obligations[0], "binding.required");

        truth.obligations[0].feature_id =
            StableId::explicit("feature.unrelated", IdKind::Feature).unwrap();
        assert!(!truth.feature_is_verified(feature_id.as_str()));

        truth.obligations[0].feature_id =
            StableId::explicit("scenario.wrong-feature-kind", IdKind::Scenario).unwrap();
        assert!(!truth.obligations[0].is_verified());
        assert!(!truth.feature_is_verified(feature_id.as_str()));
    }

    #[test]
    fn feature_verification_rejects_cross_feature_and_wrong_kind_scenario_ownership() {
        let mut truth = strict_truth(
            r#"Feature: First feature
  id: feature.first

  Scenario: First scenario
    id: scenario.first
    When the first action runs
    → id: outcome.first; the first result holds

Feature: Second feature
  id: feature.second

  Scenario: Second scenario
    id: scenario.second
    When the second action runs
    → id: outcome.second; the second result holds
"#,
        );
        let feature_id = truth.features[0].id.clone();
        let other_scenario_id = truth.obligations[1].scenario_id.clone();
        mark_verified(&mut truth.obligations[0], "binding.first");
        assert!(truth.feature_is_verified(feature_id.as_str()));

        truth.obligations[0].scenario_id = other_scenario_id;
        assert!(
            !truth.feature_is_verified(feature_id.as_str()),
            "a valid scenario owned by another feature must fail closed"
        );

        truth.obligations[0].scenario_id =
            StableId::explicit("feature.wrong-scenario-kind", IdKind::Feature).unwrap();
        assert!(!truth.obligations[0].is_verified());
        assert!(
            !truth.feature_is_verified(feature_id.as_str()),
            "an ownership edge with the wrong scenario ID kind must fail closed"
        );
    }

    #[test]
    fn unrepresented_legacy_tests_prevent_verification() {
        let mut truth = strict_truth(
            r#"Feature: Mixed authoring
  id: feature.mixed-authoring

  Scenario: Native obligation
    id: scenario.mixed-authoring.native
    When the native path runs
    → id: outcome.mixed-authoring.native; the native claim holds

  test:
    - request: GET /health
      assert:
        - status: 200
"#,
        );
        let feature_id = truth.features[0].id.clone();
        mark_verified(&mut truth.obligations[0], "binding.native");

        assert_eq!(truth.features[0].unrepresented_legacy_test_count(), 1);
        assert!(
            !truth.feature_is_verified(feature_id.as_str()),
            "legacy tests without obligation representation must fail closed"
        );
    }

    #[test]
    fn same_scenario_outcome_id_and_evidence_swap_fails_closed() {
        let mut truth = strict_truth(
            r#"Feature: Identity integrity
  id: feature.identity-integrity

  Scenario: Two distinct claims
    id: scenario.identity-integrity.two-claims
    When identity is checked
    → id: outcome.identity-integrity.first; the first claim holds
    → id: outcome.identity-integrity.second; the second claim holds
"#,
        );
        let feature_id = truth.features[0].id.clone();
        for (index, obligation) in truth.obligations.iter_mut().enumerate() {
            mark_verified(obligation, &format!("binding.{index}"));
        }
        assert!(truth.feature_is_verified(feature_id.as_str()));

        let (first, second) = truth.obligations.split_at_mut(1);
        std::mem::swap(&mut first[0].id, &mut second[0].id);
        std::mem::swap(
            &mut first[0].evidence_bindings[0].obligation_id,
            &mut second[0].evidence_bindings[0].obligation_id,
        );

        assert!(
            !truth.feature_is_verified(feature_id.as_str()),
            "an outcome ID and its evidence cannot move to another statement"
        );
        assert_eq!(truth.verified_coverage().covered, 0);
    }

    #[test]
    fn removing_an_obligation_cannot_shrink_the_parser_owned_denominator() {
        let mut truth = strict_truth(
            r#"Feature: Required claim
  id: feature.required-claim

  Scenario: One claim
    id: scenario.required-claim.one
    When the behavior runs
    → id: outcome.required-claim.one; the claim holds
"#,
        );
        truth.obligations.clear();

        assert_eq!(
            truth.verified_coverage(),
            VerifiedCoverage {
                covered: 0,
                total: 1,
            }
        );
        assert!(!truth.feature_is_verified("feature.required-claim"));
    }

    #[test]
    fn declaration_tampering_invalidates_all_coverage() {
        let mut truth = strict_truth(
            r#"Feature: Required claim
  id: feature.required-claim

  Scenario: One claim
    id: scenario.required-claim.one
    When the behavior runs
    → id: outcome.required-claim.one; the claim holds
"#,
        );
        truth
            .mark_implementation_linked("outcome.required-claim.one")
            .unwrap();
        mark_verified(&mut truth.obligations[0], "binding.required");
        truth.features[0].declaration = DeclarationStatus::DocumentationOnly;

        assert_eq!(truth.behavioral_feature_count(), 1);
        assert_eq!(truth.implementation_coverage().covered, 0);
        assert_eq!(truth.executable_coverage().covered, 0);
        assert_eq!(truth.verified_coverage().covered, 0);

        truth.features[0].declaration = DeclarationStatus::Declared;
        truth.obligations[0].declaration = DeclarationStatus::DocumentationOnly;
        assert_eq!(truth.implementation_coverage().covered, 0);
        assert_eq!(truth.executable_coverage().covered, 0);
        assert_eq!(truth.verified_coverage().covered, 0);
    }

    #[test]
    fn zeroing_the_legacy_denominator_cannot_make_a_feature_verified() {
        let mut truth = strict_truth(
            r#"Feature: Mixed authoring
  id: feature.mixed-authoring

  Scenario: Native obligation
    id: scenario.mixed-authoring.native
    When the native path runs
    → id: outcome.mixed-authoring.native; the native claim holds

  test:
    - request: GET /health
      assert:
        - status: 200
"#,
        );
        let feature_id = truth.features[0].id.clone();
        mark_verified(&mut truth.obligations[0], "binding.native");
        assert!(!truth.feature_is_verified(feature_id.as_str()));

        truth.features[0].unrepresented_legacy_tests = 0;
        assert!(
            !truth.feature_is_verified(feature_id.as_str()),
            "caller-editable state cannot erase the parser-owned legacy denominator"
        );
    }

    #[test]
    fn removing_a_feature_cannot_change_the_behavioral_feature_count() {
        let mut truth = strict_truth(
            r#"Feature: Required claim
  id: feature.required-claim

  Scenario: One claim
    id: scenario.required-claim.one
    When the behavior runs
    → id: outcome.required-claim.one; the claim holds
"#,
        );
        truth
            .mark_implementation_linked("outcome.required-claim.one")
            .unwrap();
        mark_verified(&mut truth.obligations[0], "binding.required");
        truth.features.clear();

        assert_eq!(truth.behavioral_feature_count(), 1);
        assert!(!truth.feature_is_verified("feature.required-claim"));
        assert_eq!(truth.implementation_coverage().covered, 0);
        assert_eq!(truth.executable_coverage().covered, 0);
        assert_eq!(truth.verified_coverage().covered, 0);
        assert_eq!(truth.verified_coverage().total, 1);
    }
}
