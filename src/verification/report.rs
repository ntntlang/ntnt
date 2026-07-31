use std::collections::BTreeSet;

use serde::Serialize;

use crate::intent::{CoverageReport, IntentFile, LiveScenarioResult, LiveTestResults};

use super::model::{
    AssertionResolution, BindingStatus, DeclarationStatus, Disposition, ExecutabilityStatus,
    FeatureTruth, Freshness, LinkageStatus, Obligation, VerificationTruth,
};
use super::SourceSpan;

pub const REPORT_SCHEMA_VERSION: u32 = 1;
pub const REPORT_SCHEMA_ID: &str = "https://ntnt.dev/schemas/verification/run-report-v1.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum EvidenceRequirement {
    Required,
    Advisory,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum EvidenceSelection {
    Selected,
    Excluded,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EvidenceAtom {
    id: String,
    assertion: String,
    passed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    diagnostic: Option<String>,
}

impl EvidenceAtom {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn assertion(&self) -> &str {
        &self.assertion
    }

    pub fn passed(&self) -> bool {
        self.passed
    }

    pub fn diagnostic(&self) -> Option<&str> {
        self.diagnostic.as_deref()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EvidenceAttempt {
    sequence: usize,
    disposition: Disposition,
    evidence_atoms: Vec<EvidenceAtom>,
    #[serde(skip_serializing_if = "Option::is_none")]
    diagnostic: Option<String>,
}

impl EvidenceAttempt {
    pub fn sequence(&self) -> usize {
        self.sequence
    }

    pub fn disposition(&self) -> Disposition {
        self.disposition
    }

    pub fn evidence_atoms(&self) -> &[EvidenceAtom] {
        &self.evidence_atoms
    }

    pub fn diagnostic(&self) -> Option<&str> {
        self.diagnostic.as_deref()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EvidenceResult {
    id: String,
    obligation_id: String,
    source: SourceSpan,
    profile: String,
    requirement: EvidenceRequirement,
    selection: EvidenceSelection,
    declaration: DeclarationStatus,
    linkage: LinkageStatus,
    binding: BindingStatus,
    executability: ExecutabilityStatus,
    disposition: Disposition,
    freshness: Freshness,
    assertion_resolution: AssertionResolution,
    evidence_atoms: usize,
    attempts: Vec<EvidenceAttempt>,
}

impl EvidenceResult {
    #[allow(clippy::too_many_arguments)]
    fn recorded(
        id: impl Into<String>,
        obligation_id: impl Into<String>,
        source: SourceSpan,
        requirement: EvidenceRequirement,
        selection: EvidenceSelection,
        declaration: DeclarationStatus,
        linkage: LinkageStatus,
        binding: BindingStatus,
        executability: ExecutabilityStatus,
        freshness: Freshness,
        assertion_resolution: AssertionResolution,
        attempts: Vec<EvidenceAttempt>,
    ) -> Self {
        let disposition = effective_disposition(&attempts);
        let evidence_atoms = attempts
            .last()
            .map_or(0, |attempt| attempt.evidence_atoms.len());
        Self {
            id: id.into(),
            obligation_id: obligation_id.into(),
            source,
            profile: String::new(),
            requirement,
            selection,
            declaration,
            linkage,
            binding,
            executability,
            disposition,
            freshness,
            assertion_resolution,
            evidence_atoms,
            attempts,
        }
    }

    fn qualify(mut self, profile: &str) -> Self {
        self.profile = profile.to_string();
        self
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn obligation_id(&self) -> &str {
        &self.obligation_id
    }

    pub fn profile(&self) -> &str {
        &self.profile
    }

    pub fn requirement(&self) -> EvidenceRequirement {
        self.requirement
    }

    pub fn selection(&self) -> EvidenceSelection {
        self.selection
    }

    pub fn freshness(&self) -> Freshness {
        self.freshness
    }

    pub fn evidence_atom_count(&self) -> usize {
        self.evidence_atoms
    }

    pub fn disposition(&self) -> Disposition {
        self.disposition
    }

    pub fn attempts(&self) -> &[EvidenceAttempt] {
        &self.attempts
    }

    pub fn satisfies_required(&self) -> bool {
        self.requirement == EvidenceRequirement::Required
            && self.selection == EvidenceSelection::Selected
            && self.declaration == DeclarationStatus::Declared
            && self.binding == BindingStatus::Bound
            && self.executability == ExecutabilityStatus::Executable
            && self.disposition == Disposition::Passed
            && self.freshness == Freshness::Current
            && self.assertion_resolution == AssertionResolution::Resolved
            && self.evidence_atoms > 0
            && self.attempts.last().is_some_and(|attempt| {
                !attempt.evidence_atoms.is_empty()
                    && attempt.evidence_atoms.iter().all(|atom| atom.passed)
            })
    }
}

fn effective_disposition(attempts: &[EvidenceAttempt]) -> Disposition {
    let Some(latest) = attempts.last() else {
        return Disposition::NoResult;
    };
    let latest_disposition = effective_attempt_disposition(latest);
    if latest_disposition == Disposition::Passed {
        if attempts[..attempts.len() - 1]
            .iter()
            .any(|attempt| effective_attempt_disposition(attempt) == Disposition::Failed)
        {
            return Disposition::Flaky;
        }
    }
    latest_disposition
}

fn effective_attempt_disposition(attempt: &EvidenceAttempt) -> Disposition {
    if attempt.disposition != Disposition::Passed {
        return attempt.disposition;
    }
    if attempt.evidence_atoms.is_empty() {
        Disposition::NoResult
    } else if attempt.evidence_atoms.iter().any(|atom| !atom.passed) {
        Disposition::Failed
    } else {
        Disposition::Passed
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Default)]
pub struct CoverageMetric {
    pub covered: usize,
    pub total: usize,
}

impl CoverageMetric {
    pub fn percentage(self) -> f64 {
        if self.total == 0 {
            0.0
        } else {
            (self.covered as f64 / self.total as f64) * 100.0
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default)]
pub struct CoverageSummary {
    pub implementation: CoverageMetric,
    pub executable: CoverageMetric,
    pub verified: CoverageMetric,
    pub required_bindings: CoverageMetric,
    pub documentation_only_features: usize,
    pub advisory_bindings: usize,
    pub excluded_bindings: usize,
    pub evidence_atoms: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct CoverageThresholds {
    implementation: f64,
    executable: f64,
    verified: f64,
}

impl CoverageThresholds {
    pub fn new(implementation: f64, executable: f64, verified: f64) -> Result<Self, String> {
        for (name, value) in [
            ("implementation", implementation),
            ("executable", executable),
            ("verified", verified),
        ] {
            if !value.is_finite() || !(0.0..=100.0).contains(&value) {
                return Err(format!("{name} threshold must be between 0 and 100"));
            }
        }
        Ok(Self {
            implementation,
            executable,
            verified,
        })
    }
}

impl Default for CoverageThresholds {
    fn default() -> Self {
        Self {
            implementation: 0.0,
            executable: 0.0,
            verified: 0.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
enum ExitReason {
    Success,
    VerificationFailed,
    ThresholdFailed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
struct ExitDecision {
    code: i32,
    reason: ExitReason,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct ReportObligation {
    id: String,
    feature_id: String,
    scenario_id: String,
    statement: String,
    source: SourceSpan,
    declaration: DeclarationStatus,
    linkage: LinkageStatus,
    binding: BindingStatus,
    executability: ExecutabilityStatus,
    disposition: Disposition,
    freshness: Freshness,
}

impl From<&Obligation> for ReportObligation {
    fn from(obligation: &Obligation) -> Self {
        Self {
            id: obligation.id.to_string(),
            feature_id: obligation.feature_id.to_string(),
            scenario_id: obligation.scenario_id.to_string(),
            statement: obligation.statement.clone(),
            source: obligation.source.clone(),
            declaration: obligation.declaration,
            linkage: obligation.linkage,
            binding: obligation.binding,
            executability: obligation.executability,
            disposition: obligation.disposition,
            freshness: obligation.freshness,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct ReportFeature {
    id: String,
    name: String,
    source: SourceSpan,
    declaration: DeclarationStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    rationale: Option<String>,
}

impl From<&FeatureTruth> for ReportFeature {
    fn from(feature: &FeatureTruth) -> Self {
        Self {
            id: feature.id.to_string(),
            name: feature.name.clone(),
            source: feature.source.clone(),
            declaration: feature.declaration,
            rationale: feature.rationale.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RunReport {
    schema: &'static str,
    schema_version: u32,
    ntnt_version: &'static str,
    ntnt_commit: Option<&'static str>,
    profile: String,
    profile_qualification: String,
    freshness: Freshness,
    strict: bool,
    coverage: CoverageSummary,
    thresholds: CoverageThresholds,
    features: Vec<ReportFeature>,
    obligations: Vec<ReportObligation>,
    evidence: Vec<EvidenceResult>,
    unmet_required_binding_ids: Vec<String>,
    exit: ExitDecision,
}

impl RunReport {
    /// Execute the legacy live checker and convert only its concrete assertion
    /// results into evidence. Callers cannot provide summary counters or
    /// preconstructed live results through this compatibility path.
    pub fn run_compatibility_live(
        intent: &IntentFile,
        port: u16,
        source_files: &[(String, String)],
    ) -> Self {
        let live = crate::intent::run_tests_against_server(intent, port, source_files);
        Self::from_live_results(intent, &live, "legacy-live", true)
    }

    fn from_truth(truth: &VerificationTruth, profile: &str, strict: bool) -> Self {
        let mut evidence = Vec::new();
        for obligation in &truth.obligations {
            if obligation.evidence_bindings.is_empty() {
                evidence.push(EvidenceResult::recorded(
                    format!("binding.unbound.{}", obligation.id),
                    obligation.id.to_string(),
                    obligation.source.clone(),
                    EvidenceRequirement::Required,
                    EvidenceSelection::Selected,
                    obligation.declaration,
                    obligation.linkage,
                    BindingStatus::Unbound,
                    obligation.executability,
                    obligation.freshness,
                    AssertionResolution::Unresolved,
                    Vec::new(),
                ));
            } else {
                for binding in &obligation.evidence_bindings {
                    // Slice 1A stores only a summary atom count. It is intentionally
                    // not expanded into evidence: only executor/live compatibility
                    // paths carrying concrete assertion atoms may verify coverage.
                    evidence.push(EvidenceResult::recorded(
                        binding.id.clone(),
                        obligation.id.to_string(),
                        binding.source.clone(),
                        EvidenceRequirement::Required,
                        EvidenceSelection::Selected,
                        binding.declaration,
                        binding.linkage,
                        binding.binding,
                        binding.executability,
                        binding.freshness,
                        binding.assertion_resolution,
                        Vec::new(),
                    ));
                }
            }
        }
        Self::assemble(
            profile,
            strict,
            evidence,
            truth.features.iter().map(ReportFeature::from).collect(),
            truth
                .obligations
                .iter()
                .map(ReportObligation::from)
                .collect(),
            CoverageThresholds::default(),
        )
    }

    fn from_live_results(
        intent: &IntentFile,
        live: &LiveTestResults,
        profile: &str,
        strict: bool,
    ) -> Self {
        let mut truth = VerificationTruth::from_intent(intent);
        let mut evidence = Vec::new();
        let mut compatibility_obligations = Vec::new();
        let mut report_features = truth
            .features
            .iter()
            .map(ReportFeature::from)
            .collect::<Vec<_>>();

        for (feature_index, feature) in truth.features.iter_mut().enumerate() {
            let Some(intent_feature) = intent.features.get(feature_index) else {
                continue;
            };
            let live_feature = live.features.get(feature_index);
            let expected_live_id = intent_feature.id.as_deref().unwrap_or("unknown");
            let exact_feature_mapping = live_feature.is_some_and(|candidate| {
                candidate.feature_id == expected_live_id
                    && candidate.feature_name == intent_feature.name
            });
            let linkage = if exact_feature_mapping
                && live_feature.is_some_and(|candidate| candidate.has_implementation)
            {
                LinkageStatus::Linked
            } else {
                LinkageStatus::Unlinked
            };
            for obligation in truth
                .obligations
                .iter_mut()
                .filter(|obligation| obligation.feature_id == feature.id)
            {
                obligation.linkage = linkage;
            }

            append_legacy_test_projection(
                &mut evidence,
                &mut compatibility_obligations,
                intent_feature,
                live_feature.filter(|_| exact_feature_mapping),
                linkage,
            );

            let Some(live_feature) = live_feature else {
                continue;
            };
            if !exact_feature_mapping {
                append_unmapped_feature_results(
                    &mut evidence,
                    &mut compatibility_obligations,
                    intent_feature,
                    live_feature,
                    feature_index,
                    "live feature identity does not match parsed feature declaration",
                );
                continue;
            }

            let mut consumed_live_scenarios = BTreeSet::new();
            for scenario in &intent_feature.scenarios {
                let matching: Vec<(usize, &LiveScenarioResult)> = live_feature
                    .scenarios
                    .iter()
                    .enumerate()
                    .filter(|candidate| {
                        live_scenario_name_matches(&candidate.1.name, &scenario.name)
                            && intent_feature
                                .scenarios
                                .iter()
                                .filter(|parsed| {
                                    live_scenario_name_matches(&candidate.1.name, &parsed.name)
                                })
                                .count()
                                == 1
                    })
                    .collect();
                for (run_index, (live_index, live_scenario)) in matching.iter().enumerate() {
                    consumed_live_scenarios.insert(*live_index);
                    if scenario.outcome_metadata.is_empty() {
                        append_mapping_failure(
                            &mut evidence,
                            &mut compatibility_obligations,
                            format!(
                                "outcome.compat.{}.run.{}.undeclared",
                                scenario.verification_id,
                                run_index + 1
                            ),
                            format!(
                                "binding.compat.{}.run.{}.undeclared",
                                scenario.verification_id,
                                run_index + 1
                            ),
                            intent_feature.verification_id.to_string(),
                            scenario.source.clone(),
                            format!("scenario result '{}'", live_scenario.name),
                            linkage,
                            "live scenario result has no declared outcome obligation",
                            live_scenario
                                .test_result
                                .as_ref()
                                .map(live_test_atoms)
                                .unwrap_or_default(),
                        );
                    } else {
                        append_live_scenario_evidence(
                            &mut evidence,
                            scenario,
                            live_scenario,
                            linkage,
                            run_index,
                        );
                    }
                }
            }

            for (live_index, live_scenario) in live_feature.scenarios.iter().enumerate() {
                if !consumed_live_scenarios.contains(&live_index) {
                    append_mapping_failure(
                        &mut evidence,
                        &mut compatibility_obligations,
                        format!(
                            "outcome.compat.{}.unmapped-scenario.{}",
                            intent_feature.verification_id,
                            live_index + 1
                        ),
                        format!(
                            "binding.compat.{}.unmapped-scenario.{}",
                            intent_feature.verification_id,
                            live_index + 1
                        ),
                        intent_feature.verification_id.to_string(),
                        intent_feature.source.clone(),
                        format!("unmapped live scenario '{}'", live_scenario.name),
                        linkage,
                        "live scenario cannot map exactly to a parsed scenario declaration",
                        live_scenario
                            .test_result
                            .as_ref()
                            .map(live_test_atoms)
                            .unwrap_or_default(),
                    );
                }
            }
        }

        for (feature_index, live_feature) in
            live.features.iter().enumerate().skip(intent.features.len())
        {
            let source = SourceSpan::single_line(&intent.source_path, 1, 1, 1);
            append_unmapped_live_feature(
                &mut evidence,
                &mut compatibility_obligations,
                live_feature,
                feature_index,
                source,
                "live feature has no parsed feature declaration",
            );
        }

        for (component_index, component) in intent.components.iter().enumerate() {
            if component.scenarios.is_empty() {
                continue;
            }
            let component_feature_id = compatibility_component_id(component, component_index);
            let component_source = component
                .scenarios
                .first()
                .map(|scenario| scenario.source.clone())
                .unwrap_or_else(|| SourceSpan::single_line(&intent.source_path, 1, 1, 1));
            report_features.push(ReportFeature {
                id: component_feature_id.clone(),
                name: format!("Component: {}", component.name),
                source: component_source,
                declaration: DeclarationStatus::Declared,
                rationale: None,
            });
            for scenario in &component.scenarios {
                for (statement, metadata) in
                    scenario.outcomes.iter().zip(&scenario.outcome_metadata)
                {
                    compatibility_obligations.push(ReportObligation {
                        id: metadata.id.to_string(),
                        feature_id: component_feature_id.clone(),
                        scenario_id: scenario.verification_id.to_string(),
                        statement: statement.clone(),
                        source: metadata.source.clone(),
                        declaration: DeclarationStatus::Declared,
                        linkage: LinkageStatus::Linked,
                        binding: BindingStatus::Unbound,
                        executability: ExecutabilityStatus::Unsupported,
                        disposition: Disposition::NoResult,
                        freshness: Freshness::Current,
                    });
                }
            }

            let live_component = live.components.get(component_index);
            let exact_component_mapping = live_component.is_some_and(|candidate| {
                candidate.component_id == component.id && candidate.component_name == component.name
            });
            let Some(live_component) = live_component else {
                continue;
            };
            if !exact_component_mapping {
                append_unmapped_component_results(
                    &mut evidence,
                    &mut compatibility_obligations,
                    &component_feature_id,
                    component,
                    live_component,
                    component_index,
                    "live component identity does not match parsed component declaration",
                );
                continue;
            }

            let mut consumed_live_scenarios = BTreeSet::new();
            for scenario in &component.scenarios {
                let matching = live_component
                    .scenarios
                    .iter()
                    .enumerate()
                    .filter(|(_, candidate)| {
                        live_scenario_name_matches(&candidate.name, &scenario.name)
                            && component
                                .scenarios
                                .iter()
                                .filter(|parsed| {
                                    live_scenario_name_matches(&candidate.name, &parsed.name)
                                })
                                .count()
                                == 1
                    })
                    .collect::<Vec<_>>();
                for (run_index, (live_index, live_scenario)) in matching.iter().enumerate() {
                    consumed_live_scenarios.insert(*live_index);
                    if scenario.outcome_metadata.is_empty() {
                        append_mapping_failure(
                            &mut evidence,
                            &mut compatibility_obligations,
                            format!(
                                "outcome.compat.{}.run.{}.undeclared",
                                scenario.verification_id,
                                run_index + 1
                            ),
                            format!(
                                "binding.compat.{}.run.{}.undeclared",
                                scenario.verification_id,
                                run_index + 1
                            ),
                            component_feature_id.clone(),
                            scenario.source.clone(),
                            format!("component scenario result '{}'", live_scenario.name),
                            LinkageStatus::Linked,
                            "live component scenario result has no declared outcome obligation",
                            live_scenario
                                .test_result
                                .as_ref()
                                .map(live_test_atoms)
                                .unwrap_or_default(),
                        );
                    } else {
                        append_live_scenario_evidence(
                            &mut evidence,
                            scenario,
                            live_scenario,
                            LinkageStatus::Linked,
                            run_index,
                        );
                    }
                }
            }
            for (live_index, live_scenario) in live_component.scenarios.iter().enumerate() {
                if !consumed_live_scenarios.contains(&live_index) {
                    append_mapping_failure(
                        &mut evidence,
                        &mut compatibility_obligations,
                        format!(
                            "outcome.compat.{}.unmapped-scenario.{}",
                            component_feature_id,
                            live_index + 1
                        ),
                        format!(
                            "binding.compat.{}.unmapped-scenario.{}",
                            component_feature_id,
                            live_index + 1
                        ),
                        component_feature_id.clone(),
                        component
                            .scenarios
                            .first()
                            .map(|scenario| scenario.source.clone())
                            .unwrap_or_else(|| {
                                SourceSpan::single_line(&intent.source_path, 1, 1, 1)
                            }),
                        format!("unmapped live scenario '{}'", live_scenario.name),
                        LinkageStatus::Linked,
                        "live component scenario cannot map exactly to a parsed declaration",
                        live_scenario
                            .test_result
                            .as_ref()
                            .map(live_test_atoms)
                            .unwrap_or_default(),
                    );
                }
            }
        }

        for (component_index, live_component) in live
            .components
            .iter()
            .enumerate()
            .skip(intent.components.len())
        {
            append_unmapped_live_component(
                &mut evidence,
                &mut compatibility_obligations,
                live_component,
                component_index,
                SourceSpan::single_line(&intent.source_path, 1, 1, 1),
                "live component has no parsed component declaration",
            );
        }

        let mut report_obligations = truth
            .obligations
            .iter()
            .map(ReportObligation::from)
            .collect::<Vec<_>>();
        report_obligations.extend(compatibility_obligations);
        let evidenced: BTreeSet<String> = evidence
            .iter()
            .map(|result| result.obligation_id.clone())
            .collect();
        for obligation in &report_obligations {
            if !evidenced.contains(&obligation.id) {
                evidence.push(EvidenceResult::recorded(
                    format!("binding.unbound.{}", obligation.id),
                    obligation.id.clone(),
                    obligation.source.clone(),
                    EvidenceRequirement::Required,
                    EvidenceSelection::Selected,
                    obligation.declaration,
                    obligation.linkage,
                    BindingStatus::Unbound,
                    ExecutabilityStatus::Unsupported,
                    Freshness::Current,
                    AssertionResolution::Unresolved,
                    Vec::new(),
                ));
            }
        }

        Self::assemble(
            profile,
            strict,
            evidence,
            report_features,
            report_obligations,
            CoverageThresholds::default(),
        )
    }

    /// Build a non-verifying implementation-coverage report from concrete
    /// annotations discovered in the supplied source files. The compatibility
    /// profile is fixed so callers cannot relabel this report as full evidence.
    pub fn implementation_coverage(
        intent: &IntentFile,
        source_files: &[(String, String)],
        thresholds: CoverageThresholds,
    ) -> Self {
        let legacy = crate::intent::generate_coverage_report(intent, source_files);
        Self::from_implementation_coverage(intent, &legacy, thresholds)
    }

    fn from_implementation_coverage(
        intent: &IntentFile,
        legacy: &CoverageReport,
        thresholds: CoverageThresholds,
    ) -> Self {
        let mut truth = VerificationTruth::from_intent(intent);
        for feature in &legacy.features {
            if !feature.implementations.is_empty() {
                for obligation in truth
                    .obligations
                    .iter_mut()
                    .filter(|obligation| obligation.feature_id.as_str() == feature.feature_id)
                {
                    obligation.linkage = LinkageStatus::Linked;
                }
            }
        }
        let mut report = Self::from_truth(&truth, "implementation", false);
        report.thresholds = thresholds;
        let has_unproven_behavioral_feature = report.features.iter().any(|feature| {
            feature.declaration == DeclarationStatus::Declared
                && !report
                    .obligations
                    .iter()
                    .any(|obligation| obligation.feature_id == feature.id)
        });
        report.exit = decide_exit(
            false,
            true,
            false,
            has_unproven_behavioral_feature,
            &report.coverage,
            thresholds,
            &report.evidence,
        );
        report
    }

    fn assemble(
        profile: &str,
        strict: bool,
        evidence: Vec<EvidenceResult>,
        features: Vec<ReportFeature>,
        obligations: Vec<ReportObligation>,
        thresholds: CoverageThresholds,
    ) -> Self {
        let evidence: Vec<_> = evidence
            .into_iter()
            .map(|result| result.qualify(profile))
            .collect();
        let obligations = obligations
            .into_iter()
            .map(|obligation| summarize_obligation(obligation, &evidence))
            .collect::<Vec<_>>();
        let coverage = coverage_from_ledger(&features, &obligations, &evidence);
        let unmet_required_binding_ids = evidence
            .iter()
            .filter(|result| {
                result.requirement == EvidenceRequirement::Required
                    && ((result.selection == EvidenceSelection::Selected
                        && !result.satisfies_required())
                        || (profile == "full" && result.selection == EvidenceSelection::Excluded))
            })
            .map(|result| result.id.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let has_unproven_behavioral_feature = features.iter().any(|feature| {
            feature.declaration == DeclarationStatus::Declared
                && !obligations
                    .iter()
                    .any(|obligation| obligation.feature_id == feature.id)
        });
        let exit = decide_exit(
            strict,
            false,
            profile == "full",
            has_unproven_behavioral_feature,
            &coverage,
            thresholds,
            &evidence,
        );
        Self {
            schema: REPORT_SCHEMA_ID,
            schema_version: REPORT_SCHEMA_VERSION,
            ntnt_version: env!("CARGO_PKG_VERSION"),
            ntnt_commit: option_env!("NTNT_GIT_COMMIT"),
            profile: profile.to_string(),
            profile_qualification: format!("profile:{profile}"),
            freshness: if evidence
                .iter()
                .any(|result| result.freshness == Freshness::Stale)
            {
                Freshness::Stale
            } else {
                Freshness::Current
            },
            strict,
            coverage,
            thresholds,
            features,
            obligations,
            evidence,
            unmet_required_binding_ids,
            exit,
        }
    }

    #[cfg(test)]
    fn assemble_for_tests(profile: &str, strict: bool, evidence: Vec<EvidenceResult>) -> Self {
        let obligation_ids: BTreeSet<String> = evidence
            .iter()
            .map(|result| result.obligation_id.clone())
            .collect();
        let obligations = obligation_ids
            .into_iter()
            .map(|id| ReportObligation {
                id,
                feature_id: "feature.report".to_string(),
                scenario_id: "scenario.report".to_string(),
                statement: "report truth".to_string(),
                source: SourceSpan::single_line("report.intent", 1, 1, 1),
                declaration: DeclarationStatus::Declared,
                linkage: LinkageStatus::Linked,
                binding: BindingStatus::Unbound,
                executability: ExecutabilityStatus::Unsupported,
                disposition: Disposition::NoResult,
                freshness: Freshness::Current,
            })
            .collect();
        Self::assemble(
            profile,
            strict,
            evidence,
            vec![ReportFeature {
                id: "feature.report".to_string(),
                name: "Report".to_string(),
                source: SourceSpan::single_line("report.intent", 1, 1, 1),
                declaration: DeclarationStatus::Declared,
                rationale: None,
            }],
            obligations,
            CoverageThresholds::default(),
        )
    }

    pub fn evidence(&self) -> &[EvidenceResult] {
        &self.evidence
    }

    pub fn coverage(&self) -> &CoverageSummary {
        &self.coverage
    }

    pub fn exit_code(&self) -> i32 {
        self.exit.code
    }

    pub fn profile_qualification(&self) -> &str {
        &self.profile_qualification
    }

    pub fn render_human(&self, verbosity: usize) -> String {
        let passed = self.coverage.required_bindings.covered;
        let failed = self.coverage.required_bindings.total - passed;
        let mut output = String::new();
        output.push_str("=== NTNT Intent Verification ===\n");
        output.push_str(&format!(
            "Profile: {} ({})\n",
            self.profile, self.profile_qualification
        ));
        if verbosity > 0 {
            for result in &self.evidence {
                output.push_str(&format!(
                    "- {} [{}]: {:?}\n",
                    result.obligation_id, result.id, result.disposition
                ));
                if verbosity > 1 {
                    for attempt in &result.attempts {
                        output.push_str(&format!(
                            "  attempt {}: {:?}, {} evidence atoms\n",
                            attempt.sequence,
                            attempt.disposition,
                            attempt.evidence_atoms.len()
                        ));
                        for atom in &attempt.evidence_atoms {
                            output.push_str(&format!(
                                "    {} {}\n",
                                if atom.passed { "pass" } else { "fail" },
                                atom.assertion
                            ));
                            if let Some(diagnostic) = &atom.diagnostic {
                                output.push_str(&format!("      {diagnostic}\n"));
                            }
                        }
                    }
                }
            }
        }
        output.push_str(&format!(
            "Obligations: {} verified, {} unmet; features: {} documentation-only\n",
            self.coverage.verified.covered,
            self.coverage.verified.total - self.coverage.verified.covered,
            self.coverage.documentation_only_features
        ));
        output.push_str(&format!(
            "Required bindings: {passed} passed, {failed} failed\n"
        ));
        output.push_str(&format!(
            "Coverage: implementation {:.1}%, executable {:.1}%, verified {:.1}%\n",
            self.coverage.implementation.percentage(),
            self.coverage.executable.percentage(),
            self.coverage.verified.percentage()
        ));
        if self.coverage.advisory_bindings > 0 || self.coverage.excluded_bindings > 0 {
            output.push_str(&format!(
                "Visible non-satisfying bindings: {} advisory, {} excluded\n",
                self.coverage.advisory_bindings, self.coverage.excluded_bindings
            ));
        }
        output.push_str(&format!(
            "Exit: {} ({:?})\n",
            self.exit.code, self.exit.reason
        ));
        output
    }
}

fn live_scenario_name_matches(live_name: &str, parsed_name: &str) -> bool {
    live_name == parsed_name || live_name.starts_with(&format!("{parsed_name} ["))
}

fn legacy_test_obligation_id(
    feature: &crate::intent::Feature,
    test_index: usize,
    assertion_index: usize,
) -> String {
    format!(
        "outcome.compat.{}.test.{}.assertion.{}",
        feature.verification_id,
        test_index + 1,
        assertion_index + 1
    )
}

fn legacy_test_binding_id(
    feature: &crate::intent::Feature,
    test_index: usize,
    assertion_index: usize,
) -> String {
    format!(
        "binding.compat.{}.test.{}.assertion.{}",
        feature.verification_id,
        test_index + 1,
        assertion_index + 1
    )
}

fn append_legacy_test_projection(
    evidence: &mut Vec<EvidenceResult>,
    obligations: &mut Vec<ReportObligation>,
    feature: &crate::intent::Feature,
    live_feature: Option<&crate::intent::LiveFeatureResult>,
    linkage: LinkageStatus,
) {
    for (test_index, test) in feature.tests.iter().enumerate() {
        let assertion_count = test.assertions.len().max(1);
        let live_test = live_feature.and_then(|candidate| candidate.tests.get(test_index));
        let exact_test_mapping =
            live_test.is_some_and(|candidate| {
                candidate.method == test.method
                    && candidate.path == test.path
                    && candidate.preconditions.len() == test.preconditions.len()
                    && test.preconditions.iter().zip(&candidate.preconditions).all(
                        |(expected, actual)| {
                            crate::intent::format_assertion(expected) == actual.assertion_text
                        },
                    )
                    && candidate.assertions.len() == test.assertions.len()
                    && test.assertions.iter().zip(&candidate.assertions).all(
                        |(expected, actual)| {
                            crate::intent::format_assertion(expected) == actual.assertion_text
                        },
                    )
            });

        for assertion_index in 0..assertion_count {
            let obligation_id = legacy_test_obligation_id(feature, test_index, assertion_index);
            let binding_id = legacy_test_binding_id(feature, test_index, assertion_index);
            let statement = test
                .assertions
                .get(assertion_index)
                .map(crate::intent::format_assertion)
                .unwrap_or_else(|| {
                    format!(
                        "{} {} produces concrete assertion evidence",
                        test.method, test.path
                    )
                });
            obligations.push(ReportObligation {
                id: obligation_id.clone(),
                feature_id: feature.verification_id.to_string(),
                scenario_id: format!(
                    "scenario.compat.{}.test.{}",
                    feature.verification_id,
                    test_index + 1
                ),
                statement,
                source: feature.source.clone(),
                declaration: DeclarationStatus::Declared,
                linkage,
                binding: BindingStatus::Unbound,
                executability: ExecutabilityStatus::Unsupported,
                disposition: Disposition::NoResult,
                freshness: Freshness::Current,
            });

            let Some(live_test) = live_test else {
                evidence.push(EvidenceResult::recorded(
                    binding_id,
                    obligation_id,
                    feature.source.clone(),
                    EvidenceRequirement::Required,
                    EvidenceSelection::Selected,
                    DeclarationStatus::Declared,
                    linkage,
                    BindingStatus::Unbound,
                    ExecutabilityStatus::Unsupported,
                    Freshness::Current,
                    AssertionResolution::Unresolved,
                    vec![EvidenceAttempt {
                        sequence: 1,
                        disposition: Disposition::NoResult,
                        evidence_atoms: Vec::new(),
                        diagnostic: Some(
                            "parsed technical test has no corresponding live result".to_string(),
                        ),
                    }],
                ));
                continue;
            };

            if !exact_test_mapping {
                let expected = test.assertions.len();
                let actual = live_test.assertions.len();
                evidence.push(EvidenceResult::recorded(
                    binding_id,
                    obligation_id,
                    feature.source.clone(),
                    EvidenceRequirement::Required,
                    EvidenceSelection::Selected,
                    DeclarationStatus::Declared,
                    linkage,
                    BindingStatus::Ambiguous,
                    ExecutabilityStatus::Executable,
                    Freshness::Current,
                    AssertionResolution::Unresolved,
                    vec![EvidenceAttempt {
                        sequence: 1,
                        disposition: Disposition::NoResult,
                        evidence_atoms: live_test_atoms(live_test),
                        diagnostic: Some(format!(
                            "cannot safely map live technical test {} {} with {actual} assertions \
                             to parsed declaration {} {} with {expected} assertions",
                            live_test.method, live_test.path, test.method, test.path
                        )),
                    }],
                ));
                continue;
            }

            let Some(live_assertion) = live_test.assertions.get(assertion_index) else {
                // A zero-assertion technical test has no concrete evidence atom
                // and therefore cannot satisfy its compatibility obligation.
                evidence.push(EvidenceResult::recorded(
                    binding_id,
                    obligation_id,
                    feature.source.clone(),
                    EvidenceRequirement::Required,
                    EvidenceSelection::Selected,
                    DeclarationStatus::Declared,
                    linkage,
                    BindingStatus::Bound,
                    ExecutabilityStatus::Executable,
                    Freshness::Current,
                    AssertionResolution::Unresolved,
                    vec![EvidenceAttempt {
                        sequence: 1,
                        disposition: Disposition::NoResult,
                        evidence_atoms: live_test_atoms(live_test),
                        diagnostic: Some(
                            "technical test produced no concrete assertion evidence".to_string(),
                        ),
                    }],
                ));
                continue;
            };

            let mut atoms = live_precondition_atoms(live_test, &binding_id);
            atoms.push(EvidenceAtom {
                id: format!("{binding_id}.assertion.{}", assertion_index + 1),
                assertion: live_assertion.assertion_text.clone(),
                passed: live_assertion.passed,
                diagnostic: live_assertion.message.clone(),
            });
            let failed_precondition = live_test
                .preconditions
                .iter()
                .any(|precondition| !precondition.passed);
            let disposition = if failed_precondition || !live_assertion.passed {
                Disposition::Failed
            } else {
                Disposition::Passed
            };
            evidence.push(EvidenceResult::recorded(
                binding_id,
                obligation_id,
                feature.source.clone(),
                EvidenceRequirement::Required,
                EvidenceSelection::Selected,
                DeclarationStatus::Declared,
                linkage,
                BindingStatus::Bound,
                if failed_precondition {
                    ExecutabilityStatus::Blocked
                } else {
                    ExecutabilityStatus::Executable
                },
                Freshness::Current,
                AssertionResolution::Resolved,
                vec![EvidenceAttempt {
                    sequence: 1,
                    disposition,
                    evidence_atoms: atoms,
                    diagnostic: failed_precondition
                        .then(|| "one or more test preconditions failed".to_string()),
                }],
            ));
        }
    }

    let Some(live_feature) = live_feature else {
        return;
    };
    for (test_index, live_test) in live_feature
        .tests
        .iter()
        .enumerate()
        .skip(feature.tests.len())
    {
        append_mapping_failure(
            evidence,
            obligations,
            format!(
                "outcome.compat.{}.unmapped-test.{}",
                feature.verification_id,
                test_index + 1
            ),
            format!(
                "binding.compat.{}.unmapped-test.{}",
                feature.verification_id,
                test_index + 1
            ),
            feature.verification_id.to_string(),
            feature.source.clone(),
            format!(
                "unmapped live technical test {} {}",
                live_test.method, live_test.path
            ),
            linkage,
            "live technical test has no parsed test declaration",
            live_test_atoms(live_test),
        );
    }
}

fn live_precondition_atoms(
    live_test: &crate::intent::LiveTestResult,
    binding_id: &str,
) -> Vec<EvidenceAtom> {
    live_test
        .preconditions
        .iter()
        .enumerate()
        .map(|(index, result)| EvidenceAtom {
            id: format!("{binding_id}.precondition.{}", index + 1),
            assertion: result.assertion_text.clone(),
            passed: result.passed,
            diagnostic: result.message.clone(),
        })
        .collect()
}

fn live_test_atoms(live_test: &crate::intent::LiveTestResult) -> Vec<EvidenceAtom> {
    let mut atoms = live_precondition_atoms(live_test, "atom.compat.unmapped");
    atoms.extend(
        live_test
            .assertions
            .iter()
            .enumerate()
            .map(|(index, result)| EvidenceAtom {
                id: format!("atom.compat.unmapped.assertion.{}", index + 1),
                assertion: result.assertion_text.clone(),
                passed: result.passed,
                diagnostic: result.message.clone(),
            }),
    );
    atoms
}

#[allow(clippy::too_many_arguments)]
fn append_mapping_failure(
    evidence: &mut Vec<EvidenceResult>,
    obligations: &mut Vec<ReportObligation>,
    obligation_id: String,
    binding_id: String,
    feature_id: String,
    source: SourceSpan,
    statement: String,
    linkage: LinkageStatus,
    diagnostic: &str,
    atoms: Vec<EvidenceAtom>,
) {
    obligations.push(ReportObligation {
        id: obligation_id.clone(),
        feature_id,
        scenario_id: format!("scenario.{obligation_id}"),
        statement,
        source: source.clone(),
        declaration: DeclarationStatus::Declared,
        linkage,
        binding: BindingStatus::Ambiguous,
        executability: ExecutabilityStatus::Unsupported,
        disposition: Disposition::NoResult,
        freshness: Freshness::Current,
    });
    evidence.push(EvidenceResult::recorded(
        binding_id,
        obligation_id,
        source,
        EvidenceRequirement::Required,
        EvidenceSelection::Selected,
        DeclarationStatus::Declared,
        linkage,
        BindingStatus::Ambiguous,
        ExecutabilityStatus::Unsupported,
        Freshness::Current,
        AssertionResolution::Unresolved,
        vec![EvidenceAttempt {
            sequence: 1,
            disposition: Disposition::NoResult,
            evidence_atoms: atoms,
            diagnostic: Some(diagnostic.to_string()),
        }],
    ));
}

fn append_unmapped_feature_results(
    evidence: &mut Vec<EvidenceResult>,
    obligations: &mut Vec<ReportObligation>,
    feature: &crate::intent::Feature,
    live_feature: &crate::intent::LiveFeatureResult,
    feature_index: usize,
    diagnostic: &str,
) {
    append_unmapped_live_feature(
        evidence,
        obligations,
        live_feature,
        feature_index,
        feature.source.clone(),
        diagnostic,
    );
}

fn append_unmapped_live_feature(
    evidence: &mut Vec<EvidenceResult>,
    obligations: &mut Vec<ReportObligation>,
    live_feature: &crate::intent::LiveFeatureResult,
    feature_index: usize,
    source: SourceSpan,
    diagnostic: &str,
) {
    for (test_index, live_test) in live_feature.tests.iter().enumerate() {
        append_mapping_failure(
            evidence,
            obligations,
            format!(
                "outcome.compat.unmapped-feature.{}.test.{}",
                feature_index + 1,
                test_index + 1
            ),
            format!(
                "binding.compat.unmapped-feature.{}.test.{}",
                feature_index + 1,
                test_index + 1
            ),
            if live_feature.feature_id.is_empty() {
                format!("feature.compat.unmapped.{}", feature_index + 1)
            } else {
                live_feature.feature_id.clone()
            },
            source.clone(),
            format!(
                "unmapped live technical test {} {}",
                live_test.method, live_test.path
            ),
            LinkageStatus::Unlinked,
            diagnostic,
            live_test_atoms(live_test),
        );
    }
    for (scenario_index, live_scenario) in live_feature.scenarios.iter().enumerate() {
        append_mapping_failure(
            evidence,
            obligations,
            format!(
                "outcome.compat.unmapped-feature.{}.scenario.{}",
                feature_index + 1,
                scenario_index + 1
            ),
            format!(
                "binding.compat.unmapped-feature.{}.scenario.{}",
                feature_index + 1,
                scenario_index + 1
            ),
            if live_feature.feature_id.is_empty() {
                format!("feature.compat.unmapped.{}", feature_index + 1)
            } else {
                live_feature.feature_id.clone()
            },
            source.clone(),
            format!("unmapped live scenario '{}'", live_scenario.name),
            LinkageStatus::Unlinked,
            diagnostic,
            live_scenario
                .test_result
                .as_ref()
                .map(live_test_atoms)
                .unwrap_or_default(),
        );
    }
}

fn compatibility_component_id(component: &crate::intent::Component, index: usize) -> String {
    if component.id.is_empty() {
        format!("component.compat.{}", index + 1)
    } else {
        component.id.clone()
    }
}

fn append_unmapped_component_results(
    evidence: &mut Vec<EvidenceResult>,
    obligations: &mut Vec<ReportObligation>,
    component_feature_id: &str,
    component: &crate::intent::Component,
    live_component: &crate::intent::LiveComponentResult,
    component_index: usize,
    diagnostic: &str,
) {
    let source = component
        .scenarios
        .first()
        .map(|scenario| scenario.source.clone())
        .unwrap_or_else(|| SourceSpan::single_line("", 1, 1, 1));
    append_unmapped_live_component_with_id(
        evidence,
        obligations,
        live_component,
        component_index,
        component_feature_id,
        source,
        diagnostic,
    );
}

fn append_unmapped_live_component(
    evidence: &mut Vec<EvidenceResult>,
    obligations: &mut Vec<ReportObligation>,
    live_component: &crate::intent::LiveComponentResult,
    component_index: usize,
    source: SourceSpan,
    diagnostic: &str,
) {
    append_unmapped_live_component_with_id(
        evidence,
        obligations,
        live_component,
        component_index,
        &format!("component.compat.unmapped.{}", component_index + 1),
        source,
        diagnostic,
    );
}

#[allow(clippy::too_many_arguments)]
fn append_unmapped_live_component_with_id(
    evidence: &mut Vec<EvidenceResult>,
    obligations: &mut Vec<ReportObligation>,
    live_component: &crate::intent::LiveComponentResult,
    component_index: usize,
    component_feature_id: &str,
    source: SourceSpan,
    diagnostic: &str,
) {
    for (scenario_index, live_scenario) in live_component.scenarios.iter().enumerate() {
        append_mapping_failure(
            evidence,
            obligations,
            format!(
                "outcome.compat.{}.unmapped-component.{}.scenario.{}",
                component_feature_id,
                component_index + 1,
                scenario_index + 1
            ),
            format!(
                "binding.compat.{}.unmapped-component.{}.scenario.{}",
                component_feature_id,
                component_index + 1,
                scenario_index + 1
            ),
            component_feature_id.to_string(),
            source.clone(),
            format!("unmapped live component scenario '{}'", live_scenario.name),
            LinkageStatus::Linked,
            diagnostic,
            live_scenario
                .test_result
                .as_ref()
                .map(live_test_atoms)
                .unwrap_or_default(),
        );
    }
}

fn append_live_scenario_evidence(
    evidence: &mut Vec<EvidenceResult>,
    scenario: &crate::intent::Scenario,
    live: &LiveScenarioResult,
    linkage: LinkageStatus,
    run_index: usize,
) {
    let assertions = live
        .test_result
        .as_ref()
        .map(|result| result.assertions.as_slice())
        .unwrap_or_default();
    let exact_mapping =
        assertions.len() == scenario.outcome_metadata.len() && live.outcomes == scenario.outcomes;

    for (outcome_index, metadata) in scenario.outcome_metadata.iter().enumerate() {
        let binding_id = format!(
            "binding.compat.{}.{}.{}",
            scenario.verification_id,
            run_index + 1,
            outcome_index + 1
        );
        let failed_precondition = live.test_result.as_ref().is_some_and(|result| {
            result
                .preconditions
                .iter()
                .any(|precondition| !precondition.passed)
        });
        let all_atoms = live
            .test_result
            .as_ref()
            .map(|test| {
                let mut atoms = live_precondition_atoms(test, &binding_id);
                if let Some(assertion) = test.assertions.get(outcome_index) {
                    atoms.push(EvidenceAtom {
                        id: format!("{binding_id}.assertion.{}", outcome_index + 1),
                        assertion: assertion.assertion_text.clone(),
                        passed: assertion.passed,
                        diagnostic: assertion.message.clone(),
                    });
                }
                atoms
            })
            .unwrap_or_default();
        let (binding, executability, resolution, attempts) = if failed_precondition {
            (
                BindingStatus::Bound,
                ExecutabilityStatus::Blocked,
                AssertionResolution::Resolved,
                vec![EvidenceAttempt {
                    sequence: 1,
                    disposition: Disposition::Failed,
                    evidence_atoms: all_atoms,
                    diagnostic: Some("one or more scenario preconditions failed".to_string()),
                }],
            )
        } else {
            match live.status.as_str() {
                "pass" | "fail" if exact_mapping => {
                    let assertion = &assertions[outcome_index];
                    let disposition = if live.status == "pass" && assertion.passed {
                        Disposition::Passed
                    } else {
                        Disposition::Failed
                    };
                    (
                        BindingStatus::Bound,
                        ExecutabilityStatus::Executable,
                        AssertionResolution::Resolved,
                        vec![EvidenceAttempt {
                            sequence: 1,
                            disposition,
                            evidence_atoms: all_atoms,
                            diagnostic: None,
                        }],
                    )
                }
                "pass" | "fail" => (
                    BindingStatus::Ambiguous,
                    ExecutabilityStatus::Executable,
                    AssertionResolution::Unresolved,
                    vec![EvidenceAttempt {
                        sequence: 1,
                        disposition: Disposition::NoResult,
                        evidence_atoms: live
                            .test_result
                            .as_ref()
                            .map(live_test_atoms)
                            .unwrap_or_default(),
                        diagnostic: Some(format!(
                            "cannot safely map {} assertions to {} outcomes",
                            assertions.len(),
                            scenario.outcome_metadata.len()
                        )),
                    }],
                ),
                "warning" => (
                    BindingStatus::Bound,
                    ExecutabilityStatus::Executable,
                    AssertionResolution::Unresolved,
                    vec![EvidenceAttempt {
                        sequence: 1,
                        disposition: Disposition::NoResult,
                        evidence_atoms: live
                            .test_result
                            .as_ref()
                            .map(live_test_atoms)
                            .unwrap_or_default(),
                        diagnostic: Some(if live.unresolved_outcomes.is_empty() {
                            "scenario completed with unresolved warning status".to_string()
                        } else {
                            format!(
                                "scenario has unresolved outcomes: {}",
                                live.unresolved_outcomes.join(", ")
                            )
                        }),
                    }],
                ),
                "skip" => (
                    BindingStatus::Bound,
                    ExecutabilityStatus::Blocked,
                    AssertionResolution::Resolved,
                    vec![EvidenceAttempt {
                        sequence: 1,
                        disposition: Disposition::Skipped,
                        evidence_atoms: live
                            .test_result
                            .as_ref()
                            .map(live_test_atoms)
                            .unwrap_or_default(),
                        diagnostic: Some("precondition was not met".to_string()),
                    }],
                ),
                status => (
                    BindingStatus::Unbound,
                    ExecutabilityStatus::Unsupported,
                    AssertionResolution::Unresolved,
                    vec![EvidenceAttempt {
                        sequence: 1,
                        disposition: Disposition::NoResult,
                        evidence_atoms: live
                            .test_result
                            .as_ref()
                            .map(live_test_atoms)
                            .unwrap_or_default(),
                        diagnostic: Some(format!(
                            "scenario produced non-verifying status '{status}'"
                        )),
                    }],
                ),
            }
        };
        evidence.push(EvidenceResult::recorded(
            binding_id,
            metadata.id.to_string(),
            metadata.source.clone(),
            EvidenceRequirement::Required,
            EvidenceSelection::Selected,
            DeclarationStatus::Declared,
            linkage,
            binding,
            executability,
            Freshness::Current,
            resolution,
            attempts,
        ));
    }
}

fn summarize_obligation(
    mut obligation: ReportObligation,
    evidence: &[EvidenceResult],
) -> ReportObligation {
    let selected = evidence
        .iter()
        .filter(|result| {
            result.obligation_id == obligation.id
                && result.requirement == EvidenceRequirement::Required
                && result.selection == EvidenceSelection::Selected
        })
        .collect::<Vec<_>>();
    if selected.is_empty() {
        return obligation;
    }

    obligation.linkage = if selected
        .iter()
        .any(|result| result.linkage == LinkageStatus::Linked)
    {
        LinkageStatus::Linked
    } else {
        LinkageStatus::Unlinked
    };
    obligation.binding = if selected
        .iter()
        .any(|result| result.binding == BindingStatus::Ambiguous)
    {
        BindingStatus::Ambiguous
    } else if selected
        .iter()
        .all(|result| result.binding == BindingStatus::Bound)
    {
        BindingStatus::Bound
    } else {
        BindingStatus::Unbound
    };
    obligation.executability = if selected
        .iter()
        .all(|result| result.executability == ExecutabilityStatus::Executable)
    {
        ExecutabilityStatus::Executable
    } else if selected
        .iter()
        .any(|result| result.executability == ExecutabilityStatus::Blocked)
    {
        ExecutabilityStatus::Blocked
    } else {
        ExecutabilityStatus::Unsupported
    };
    obligation.disposition = aggregate_disposition(&selected);
    obligation.freshness = if selected
        .iter()
        .any(|result| result.freshness == Freshness::Stale)
    {
        Freshness::Stale
    } else {
        Freshness::Current
    };
    obligation
}

fn aggregate_disposition(evidence: &[&EvidenceResult]) -> Disposition {
    for disposition in [
        Disposition::Failed,
        Disposition::Flaky,
        Disposition::Cancelled,
        Disposition::Skipped,
        Disposition::NoResult,
        Disposition::Running,
        Disposition::Planned,
    ] {
        if evidence
            .iter()
            .any(|result| result.disposition == disposition)
        {
            return disposition;
        }
    }
    Disposition::Passed
}

fn coverage_from_ledger(
    features: &[ReportFeature],
    obligations: &[ReportObligation],
    evidence: &[EvidenceResult],
) -> CoverageSummary {
    let behavioral_obligations: Vec<_> = obligations
        .iter()
        .filter(|obligation| obligation.declaration == DeclarationStatus::Declared)
        .collect();
    let total = behavioral_obligations.len();
    let implementation = behavioral_obligations
        .iter()
        .filter(|obligation| obligation.linkage == LinkageStatus::Linked)
        .count();
    let executable = behavioral_obligations
        .iter()
        .filter(|obligation| {
            let selected = evidence
                .iter()
                .filter(|result| {
                    result.obligation_id == obligation.id
                        && result.requirement == EvidenceRequirement::Required
                        && result.selection == EvidenceSelection::Selected
                })
                .collect::<Vec<_>>();
            !selected.is_empty()
                && selected.iter().all(|result| {
                    result.binding == BindingStatus::Bound
                        && result.executability == ExecutabilityStatus::Executable
                })
        })
        .count();
    let verified = behavioral_obligations
        .iter()
        .filter(|obligation| {
            let selected: Vec<_> = evidence
                .iter()
                .filter(|result| {
                    result.obligation_id == obligation.id
                        && result.requirement == EvidenceRequirement::Required
                        && result.selection == EvidenceSelection::Selected
                })
                .collect();
            !selected.is_empty() && selected.iter().all(|result| result.satisfies_required())
        })
        .count();
    let selected_required: Vec<_> = evidence
        .iter()
        .filter(|result| {
            result.requirement == EvidenceRequirement::Required
                && result.selection == EvidenceSelection::Selected
        })
        .collect();

    CoverageSummary {
        implementation: CoverageMetric {
            covered: implementation,
            total,
        },
        executable: CoverageMetric {
            covered: executable,
            total,
        },
        verified: CoverageMetric {
            covered: verified,
            total,
        },
        required_bindings: CoverageMetric {
            covered: selected_required
                .iter()
                .filter(|result| result.satisfies_required())
                .count(),
            total: selected_required.len(),
        },
        documentation_only_features: features
            .iter()
            .filter(|feature| feature.declaration == DeclarationStatus::DocumentationOnly)
            .count(),
        advisory_bindings: evidence
            .iter()
            .filter(|result| result.requirement == EvidenceRequirement::Advisory)
            .count(),
        excluded_bindings: evidence
            .iter()
            .filter(|result| result.selection == EvidenceSelection::Excluded)
            .count(),
        evidence_atoms: evidence.iter().map(|result| result.evidence_atoms).sum(),
    }
}

fn decide_exit(
    strict: bool,
    require_implementation: bool,
    require_all_obligations: bool,
    has_unproven_behavioral_feature: bool,
    coverage: &CoverageSummary,
    thresholds: CoverageThresholds,
    evidence: &[EvidenceResult],
) -> ExitDecision {
    let selected_required: Vec<_> = evidence
        .iter()
        .filter(|result| {
            result.requirement == EvidenceRequirement::Required
                && result.selection == EvidenceSelection::Selected
        })
        .collect();
    if ((strict || require_implementation) && has_unproven_behavioral_feature)
        || (strict
            && ((require_all_obligations && coverage.verified.covered != coverage.verified.total)
                || (coverage.verified.total > 0 && selected_required.is_empty())
                || selected_required
                    .iter()
                    .any(|result| !result.satisfies_required())))
    {
        return ExitDecision {
            code: 1,
            reason: ExitReason::VerificationFailed,
        };
    }
    if require_implementation
        && coverage.implementation.total > 0
        && coverage.implementation.covered == 0
    {
        return ExitDecision {
            code: 1,
            reason: ExitReason::VerificationFailed,
        };
    }
    if coverage.implementation.percentage() < thresholds.implementation
        || coverage.executable.percentage() < thresholds.executable
        || coverage.verified.percentage() < thresholds.verified
    {
        return ExitDecision {
            code: 1,
            reason: ExitReason::ThresholdFailed,
        };
    }
    ExitDecision {
        code: 0,
        reason: ExitReason::Success,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::intent::{
        LiveAssertionResult, LiveComponentResult, LiveFeatureResult, LiveTestResult,
    };
    use crate::verification::{
        AssertionResolution, BindingStatus, DeclarationStatus, Disposition, EvidenceBinding,
        ExecutabilityStatus, Freshness, IdMode, LinkageStatus, SourceSpan, VerificationTruth,
    };

    fn atom(id: &str, passed: bool) -> EvidenceAtom {
        EvidenceAtom {
            id: id.to_string(),
            assertion: format!("assertion {id}"),
            passed,
            diagnostic: (!passed).then(|| format!("{id} failed")),
        }
    }

    fn attempt(
        sequence: usize,
        disposition: Disposition,
        atoms: Vec<EvidenceAtom>,
    ) -> EvidenceAttempt {
        EvidenceAttempt {
            sequence,
            disposition,
            evidence_atoms: atoms,
            diagnostic: None,
        }
    }

    fn evidence(
        id: &str,
        requirement: EvidenceRequirement,
        selection: EvidenceSelection,
        binding: BindingStatus,
        executability: ExecutabilityStatus,
        freshness: Freshness,
        attempts: Vec<EvidenceAttempt>,
    ) -> EvidenceResult {
        EvidenceResult::recorded(
            id,
            "outcome.report.example",
            SourceSpan::single_line("verification.tnt", 7, 1, 20),
            requirement,
            selection,
            DeclarationStatus::Declared,
            LinkageStatus::Linked,
            binding,
            executability,
            freshness,
            AssertionResolution::Resolved,
            attempts,
        )
    }

    fn report(profile: &str, evidence: Vec<EvidenceResult>) -> RunReport {
        RunReport::assemble_for_tests(profile, true, evidence)
    }

    fn live_assertion(text: &str, passed: bool) -> LiveAssertionResult {
        LiveAssertionResult {
            assertion_text: text.to_string(),
            passed,
            message: (!passed).then(|| format!("{text} failed")),
            resolution_trace: None,
            checks: Vec::new(),
        }
    }

    fn live_test(
        method: &str,
        path: &str,
        passed: bool,
        assertions: Vec<LiveAssertionResult>,
    ) -> LiveTestResult {
        LiveTestResult {
            method: method.to_string(),
            path: path.to_string(),
            passed,
            assertions,
            preconditions: Vec::new(),
            scenario_name: None,
        }
    }

    fn live_scenario(
        name: &str,
        status: &str,
        assertions: Option<Vec<LiveAssertionResult>>,
    ) -> LiveScenarioResult {
        LiveScenarioResult {
            name: name.to_string(),
            description: None,
            given_clause: None,
            when_clause: "the check runs".to_string(),
            outcomes: assertions
                .as_ref()
                .map(|assertions| {
                    assertions
                        .iter()
                        .map(|assertion| assertion.assertion_text.clone())
                        .collect()
                })
                .unwrap_or_default(),
            status: status.to_string(),
            test_result: assertions
                .map(|assertions| live_test("GET", "/", status == "pass", assertions)),
            unresolved_outcomes: Vec::new(),
            component_refs: Vec::new(),
        }
    }

    fn live_results(
        features: Vec<LiveFeatureResult>,
        components: Vec<LiveComponentResult>,
    ) -> LiveTestResults {
        LiveTestResults {
            features,
            components,
            // Deliberately nonsensical: compatibility projection must consume
            // concrete results, never these cached summaries.
            total_assertions: 0,
            passed_assertions: 0,
            failed_assertions: 999,
            linked_features: 0,
            total_features: 999,
            title: None,
            glossary: None,
            summary: None,
        }
    }

    #[test]
    fn every_task_1b_disposition_is_truthful() {
        let cases = [
            (
                "planned",
                evidence(
                    "binding.planned",
                    EvidenceRequirement::Required,
                    EvidenceSelection::Selected,
                    BindingStatus::Bound,
                    ExecutabilityStatus::Executable,
                    Freshness::Current,
                    vec![attempt(1, Disposition::Planned, vec![])],
                ),
                Disposition::Planned,
                false,
            ),
            (
                "running",
                evidence(
                    "binding.running",
                    EvidenceRequirement::Required,
                    EvidenceSelection::Selected,
                    BindingStatus::Bound,
                    ExecutabilityStatus::Executable,
                    Freshness::Current,
                    vec![attempt(1, Disposition::Running, vec![])],
                ),
                Disposition::Running,
                false,
            ),
            (
                "unbound",
                evidence(
                    "binding.unbound",
                    EvidenceRequirement::Required,
                    EvidenceSelection::Selected,
                    BindingStatus::Unbound,
                    ExecutabilityStatus::Unsupported,
                    Freshness::Current,
                    vec![],
                ),
                Disposition::NoResult,
                false,
            ),
            (
                "unsupported",
                evidence(
                    "binding.unsupported",
                    EvidenceRequirement::Required,
                    EvidenceSelection::Selected,
                    BindingStatus::Bound,
                    ExecutabilityStatus::Unsupported,
                    Freshness::Current,
                    vec![],
                ),
                Disposition::NoResult,
                false,
            ),
            (
                "blocked",
                evidence(
                    "binding.blocked",
                    EvidenceRequirement::Required,
                    EvidenceSelection::Selected,
                    BindingStatus::Bound,
                    ExecutabilityStatus::Blocked,
                    Freshness::Current,
                    vec![],
                ),
                Disposition::NoResult,
                false,
            ),
            (
                "skipped",
                evidence(
                    "binding.skipped",
                    EvidenceRequirement::Required,
                    EvidenceSelection::Selected,
                    BindingStatus::Bound,
                    ExecutabilityStatus::Executable,
                    Freshness::Current,
                    vec![attempt(1, Disposition::Skipped, vec![])],
                ),
                Disposition::Skipped,
                false,
            ),
            (
                "stale",
                evidence(
                    "binding.stale",
                    EvidenceRequirement::Required,
                    EvidenceSelection::Selected,
                    BindingStatus::Bound,
                    ExecutabilityStatus::Executable,
                    Freshness::Stale,
                    vec![attempt(1, Disposition::Passed, vec![atom("stale", true)])],
                ),
                Disposition::Passed,
                false,
            ),
            (
                "failed",
                evidence(
                    "binding.failed",
                    EvidenceRequirement::Required,
                    EvidenceSelection::Selected,
                    BindingStatus::Bound,
                    ExecutabilityStatus::Executable,
                    Freshness::Current,
                    vec![attempt(1, Disposition::Failed, vec![atom("failed", false)])],
                ),
                Disposition::Failed,
                false,
            ),
            (
                "flaky",
                evidence(
                    "binding.flaky",
                    EvidenceRequirement::Required,
                    EvidenceSelection::Selected,
                    BindingStatus::Bound,
                    ExecutabilityStatus::Executable,
                    Freshness::Current,
                    vec![
                        attempt(1, Disposition::Failed, vec![atom("first", false)]),
                        attempt(2, Disposition::Passed, vec![atom("second", true)]),
                    ],
                ),
                Disposition::Flaky,
                false,
            ),
            (
                "cancelled",
                evidence(
                    "binding.cancelled",
                    EvidenceRequirement::Required,
                    EvidenceSelection::Selected,
                    BindingStatus::Bound,
                    ExecutabilityStatus::Executable,
                    Freshness::Current,
                    vec![attempt(1, Disposition::Cancelled, vec![])],
                ),
                Disposition::Cancelled,
                false,
            ),
            (
                "no-result",
                evidence(
                    "binding.no-result",
                    EvidenceRequirement::Required,
                    EvidenceSelection::Selected,
                    BindingStatus::Bound,
                    ExecutabilityStatus::Executable,
                    Freshness::Current,
                    vec![attempt(1, Disposition::Passed, vec![])],
                ),
                Disposition::NoResult,
                false,
            ),
            (
                "current-passed",
                evidence(
                    "binding.current-passed",
                    EvidenceRequirement::Required,
                    EvidenceSelection::Selected,
                    BindingStatus::Bound,
                    ExecutabilityStatus::Executable,
                    Freshness::Current,
                    vec![attempt(1, Disposition::Passed, vec![atom("pass", true)])],
                ),
                Disposition::Passed,
                true,
            ),
        ];

        for (name, evidence, disposition, satisfies) in cases {
            let report = report("full", vec![evidence]);
            assert_eq!(report.evidence()[0].disposition(), disposition, "{name}");
            assert_eq!(
                report.evidence()[0].satisfies_required(),
                satisfies,
                "{name}"
            );
            assert_eq!(report.exit_code(), if satisfies { 0 } else { 1 }, "{name}");
        }
    }

    #[test]
    fn strict_exit_fails_each_unmet_required_binding_and_all_required_can_pass() {
        let passed = evidence(
            "binding.passed",
            EvidenceRequirement::Required,
            EvidenceSelection::Selected,
            BindingStatus::Bound,
            ExecutabilityStatus::Executable,
            Freshness::Current,
            vec![attempt(1, Disposition::Passed, vec![atom("pass", true)])],
        );
        let failed = evidence(
            "binding.failed",
            EvidenceRequirement::Required,
            EvidenceSelection::Selected,
            BindingStatus::Bound,
            ExecutabilityStatus::Executable,
            Freshness::Current,
            vec![attempt(1, Disposition::Failed, vec![atom("fail", false)])],
        );

        assert_eq!(report("full", vec![passed.clone(), failed]).exit_code(), 1);
        assert_eq!(report("full", vec![passed.clone(), passed]).exit_code(), 0);
    }

    #[test]
    fn fail_then_pass_is_flaky_and_retains_both_diagnostics() {
        let evidence = evidence(
            "binding.retry",
            EvidenceRequirement::Required,
            EvidenceSelection::Selected,
            BindingStatus::Bound,
            ExecutabilityStatus::Executable,
            Freshness::Current,
            vec![
                attempt(1, Disposition::Failed, vec![atom("first", false)]),
                attempt(2, Disposition::Passed, vec![atom("second", true)]),
            ],
        );
        let report = report("full", vec![evidence]);
        let result = &report.evidence()[0];

        assert_eq!(result.disposition(), Disposition::Flaky);
        assert_eq!(result.attempts().len(), 2);
        assert_eq!(
            result.attempts()[0].evidence_atoms[0].diagnostic.as_deref(),
            Some("first failed")
        );
        assert_eq!(report.exit_code(), 1);
    }

    #[test]
    fn concrete_atoms_override_live_summary_booleans_for_compatibility_ids() {
        let intent = IntentFile::parse_content_with_id_mode(
            "Feature: Legacy report\n\n  Scenario: Execute\n    When execution is recorded\n    → result is observed\n",
            "legacy-report.intent".to_string(),
            IdMode::Compatibility,
        )
        .unwrap();
        let live_assertion = crate::intent::LiveAssertionResult {
            assertion_text: "result is observed".to_string(),
            passed: true,
            message: None,
            resolution_trace: None,
            checks: Vec::new(),
        };
        let live_test = crate::intent::LiveTestResult {
            method: "GET".to_string(),
            path: "/".to_string(),
            passed: false,
            assertions: vec![live_assertion],
            preconditions: Vec::new(),
            scenario_name: Some("Execute".to_string()),
        };
        let live = LiveTestResults {
            features: vec![crate::intent::LiveFeatureResult {
                feature_id: "unknown".to_string(),
                feature_name: "Legacy report".to_string(),
                description: None,
                passed: false,
                tests: Vec::new(),
                scenarios: vec![LiveScenarioResult {
                    name: "Execute".to_string(),
                    description: None,
                    given_clause: None,
                    when_clause: "execution is recorded".to_string(),
                    outcomes: vec!["result is observed".to_string()],
                    status: "pass".to_string(),
                    test_result: Some(live_test),
                    unresolved_outcomes: Vec::new(),
                    component_refs: Vec::new(),
                }],
                has_implementation: false,
            }],
            components: Vec::new(),
            total_assertions: 0,
            passed_assertions: 0,
            failed_assertions: 99,
            linked_features: 0,
            total_features: 0,
            title: None,
            glossary: None,
            summary: None,
        };

        let report = RunReport::from_live_results(&intent, &live, "legacy-live", true);

        assert_eq!(report.exit_code(), 0);
        assert_eq!(
            report.coverage().verified,
            CoverageMetric {
                covered: 1,
                total: 1,
            }
        );
        assert!(report.evidence()[0].satisfies_required());
    }

    #[test]
    fn legacy_technical_tests_are_required_with_deterministic_qualified_ids() {
        let intent = IntentFile::parse_content_with_id_mode(
            r#"Feature: Legacy technical
  id: feature.legacy-technical
  test:
    - request: GET /health
      assert:
        - status: 200
        - body contains "ok"
"#,
            "legacy-technical.intent".to_string(),
            IdMode::Compatibility,
        )
        .unwrap();
        let feature = |assertions: Vec<LiveAssertionResult>| {
            let test_passed = assertions.iter().all(|assertion| assertion.passed);
            LiveFeatureResult {
                feature_id: "feature.legacy-technical".to_string(),
                feature_name: "Legacy technical".to_string(),
                description: None,
                // This cached feature boolean deliberately lies.
                passed: false,
                tests: vec![live_test("GET", "/health", test_passed, assertions)],
                scenarios: Vec::new(),
                has_implementation: true,
            }
        };

        let passing = RunReport::from_live_results(
            &intent,
            &live_results(
                vec![feature(vec![
                    live_assertion("status: 200", true),
                    live_assertion("body contains \"ok\"", true),
                ])],
                Vec::new(),
            ),
            "legacy-live",
            true,
        );
        assert_eq!(passing.exit_code(), 0);
        assert_eq!(passing.evidence().len(), 2);
        assert!(passing
            .evidence()
            .iter()
            .all(EvidenceResult::satisfies_required));
        assert_eq!(
            passing.evidence()[0].obligation_id(),
            "outcome.compat.feature.legacy-technical.test.1.assertion.1"
        );
        assert_eq!(
            passing.evidence()[0].id(),
            "binding.compat.feature.legacy-technical.test.1.assertion.1"
        );
        assert_eq!(passing.evidence()[0].profile(), "legacy-live");
        assert_eq!(passing.evidence()[0].source.path, "legacy-technical.intent");

        let failing = RunReport::from_live_results(
            &intent,
            &live_results(
                vec![feature(vec![
                    live_assertion("status: 200", true),
                    live_assertion("body contains \"ok\"", false),
                ])],
                Vec::new(),
            ),
            "legacy-live",
            true,
        );
        assert_eq!(failing.exit_code(), 1);
        assert_eq!(failing.coverage().required_bindings.total, 2);
        assert_eq!(failing.coverage().required_bindings.covered, 1);
    }

    #[test]
    fn legacy_technical_mapping_is_fail_closed_for_missing_extra_and_mismatched_results() {
        let intent = IntentFile::parse_content_with_id_mode(
            r#"Feature: Mapping
  id: feature.mapping
  test:
    - request: GET /mapped
      assert:
        - status: 200
        - body contains "mapped"
"#,
            "mapping.intent".to_string(),
            IdMode::Compatibility,
        )
        .unwrap();
        let report_for = |tests| {
            RunReport::from_live_results(
                &intent,
                &live_results(
                    vec![LiveFeatureResult {
                        feature_id: "feature.mapping".to_string(),
                        feature_name: "Mapping".to_string(),
                        description: None,
                        passed: true,
                        tests,
                        scenarios: Vec::new(),
                        has_implementation: true,
                    }],
                    Vec::new(),
                ),
                "legacy-live",
                true,
            )
        };

        for (case, tests) in [
            (
                "missing assertion",
                vec![live_test(
                    "GET",
                    "/mapped",
                    true,
                    vec![live_assertion("status: 200", true)],
                )],
            ),
            (
                "extra assertion",
                vec![live_test(
                    "GET",
                    "/mapped",
                    true,
                    vec![
                        live_assertion("status: 200", true),
                        live_assertion("body contains \"mapped\"", true),
                        live_assertion("unexpected", true),
                    ],
                )],
            ),
            (
                "mismatched test",
                vec![live_test(
                    "POST",
                    "/different",
                    true,
                    vec![
                        live_assertion("status: 200", true),
                        live_assertion("body contains \"mapped\"", true),
                    ],
                )],
            ),
            (
                "mismatched assertions",
                vec![live_test(
                    "GET",
                    "/mapped",
                    true,
                    vec![
                        live_assertion("body contains \"mapped\"", true),
                        live_assertion("status: 200", true),
                    ],
                )],
            ),
            ("missing test", Vec::new()),
            (
                "extra test",
                vec![
                    live_test(
                        "GET",
                        "/mapped",
                        true,
                        vec![
                            live_assertion("status: 200", true),
                            live_assertion("body contains \"mapped\"", true),
                        ],
                    ),
                    live_test(
                        "GET",
                        "/extra",
                        true,
                        vec![live_assertion("extra result", true)],
                    ),
                ],
            ),
        ] {
            let report = report_for(tests);
            assert_eq!(report.exit_code(), 1, "{case}");
            assert!(
                report.evidence().iter().any(|result| {
                    result.disposition() == Disposition::NoResult
                        || result.assertion_resolution == AssertionResolution::Unresolved
                }),
                "{case}: {:?}",
                report.evidence()
            );
        }
    }

    #[test]
    fn legacy_technical_preconditions_must_map_exactly() {
        let mut intent = IntentFile::parse_content_with_id_mode(
            r#"Feature: Preconditions
  id: feature.technical-preconditions
  test:
    - request: GET /guarded
      assert:
        - status: 200
"#,
            "technical-preconditions.intent".to_string(),
            IdMode::Compatibility,
        )
        .unwrap();
        intent.features[0].tests[0].preconditions = vec![crate::intent::Assertion::Status(204)];

        let report_for = |preconditions: Vec<LiveAssertionResult>| {
            let mut test = live_test(
                "GET",
                "/guarded",
                true,
                vec![live_assertion("status: 200", true)],
            );
            test.preconditions = preconditions;
            RunReport::from_live_results(
                &intent,
                &live_results(
                    vec![LiveFeatureResult {
                        feature_id: "feature.technical-preconditions".to_string(),
                        feature_name: "Preconditions".to_string(),
                        description: None,
                        passed: true,
                        tests: vec![test],
                        scenarios: Vec::new(),
                        has_implementation: true,
                    }],
                    Vec::new(),
                ),
                "legacy-live",
                true,
            )
        };

        assert_eq!(
            report_for(vec![live_assertion("status: 204", true)]).exit_code(),
            0
        );
        assert_eq!(report_for(Vec::new()).exit_code(), 1);
        assert_eq!(
            report_for(vec![
                live_assertion("status: 204", true),
                live_assertion("extra", true),
            ])
            .exit_code(),
            1
        );
        assert_eq!(
            report_for(vec![live_assertion("status: 205", true)]).exit_code(),
            1
        );
    }

    #[test]
    fn failed_preconditions_override_passing_outcome_atoms() {
        let intent = IntentFile::parse_content_with_id_mode(
            r#"Feature: Preconditions
  id: feature.preconditions

  Scenario: Protected action
    id: scenario.preconditions.protected
    Given access is allowed
    When the check runs
    → id: outcome.preconditions.completed; action completes
"#,
            "preconditions.intent".to_string(),
            IdMode::Strict,
        )
        .unwrap();
        let mut scenario = live_scenario(
            "Protected action",
            "fail",
            Some(vec![live_assertion("action completes", true)]),
        );
        scenario.test_result.as_mut().unwrap().preconditions =
            vec![live_assertion("access is allowed", false)];
        let report = RunReport::from_live_results(
            &intent,
            &live_results(
                vec![LiveFeatureResult {
                    feature_id: "feature.preconditions".to_string(),
                    feature_name: "Preconditions".to_string(),
                    description: None,
                    passed: false,
                    tests: Vec::new(),
                    scenarios: vec![scenario],
                    has_implementation: true,
                }],
                Vec::new(),
            ),
            "legacy-live",
            true,
        );

        assert_eq!(report.exit_code(), 1);
        assert_eq!(report.evidence()[0].disposition(), Disposition::Failed);
        assert!(report.evidence()[0].attempts()[0]
            .evidence_atoms()
            .iter()
            .any(|atom| !atom.passed()));
    }

    #[test]
    fn warning_pending_and_unresolved_scenarios_never_become_no_evidence_successes() {
        let intent = IntentFile::parse_content_with_id_mode(
            r#"Feature: Incomplete
  id: feature.incomplete

  Scenario: Resolve me
    id: scenario.incomplete.resolve
    When the check runs
    → id: outcome.incomplete.resolved; result resolves
"#,
            "incomplete.intent".to_string(),
            IdMode::Strict,
        )
        .unwrap();
        for status in ["warning", "pending", "unresolved", "unknown"] {
            let mut scenario = live_scenario(
                "Resolve me",
                status,
                (status == "warning").then(|| vec![live_assertion("result resolves", true)]),
            );
            scenario.unresolved_outcomes = vec!["result resolves".to_string()];
            let report = RunReport::from_live_results(
                &intent,
                &live_results(
                    vec![LiveFeatureResult {
                        feature_id: "feature.incomplete".to_string(),
                        feature_name: "Incomplete".to_string(),
                        description: None,
                        passed: true,
                        tests: Vec::new(),
                        scenarios: vec![scenario],
                        has_implementation: true,
                    }],
                    Vec::new(),
                ),
                "legacy-live",
                true,
            );
            assert_eq!(report.exit_code(), 1, "{status}");
            assert!(!report.evidence()[0].satisfies_required(), "{status}");
        }
    }

    #[test]
    fn every_expanded_scenario_run_is_a_required_binding() {
        let intent = IntentFile::parse_content_with_id_mode(
            r#"Feature: Expanded
  id: feature.expanded

  Scenario: Row check
    id: scenario.expanded.row-check
    When the check runs
    → id: outcome.expanded.row-valid; row is valid
"#,
            "expanded.intent".to_string(),
            IdMode::Strict,
        )
        .unwrap();
        let feature = |second_passes| LiveFeatureResult {
            feature_id: "feature.expanded".to_string(),
            feature_name: "Expanded".to_string(),
            description: None,
            passed: second_passes,
            tests: Vec::new(),
            scenarios: vec![
                live_scenario(
                    "Row check [first]",
                    "pass",
                    Some(vec![live_assertion("row is valid", true)]),
                ),
                live_scenario(
                    "Row check [second]",
                    if second_passes { "pass" } else { "fail" },
                    Some(vec![live_assertion("row is valid", second_passes)]),
                ),
            ],
            has_implementation: true,
        };

        let passing = RunReport::from_live_results(
            &intent,
            &live_results(vec![feature(true)], Vec::new()),
            "legacy-live",
            true,
        );
        assert_eq!(passing.exit_code(), 0);
        assert_eq!(passing.coverage().required_bindings.total, 2);

        let failing = RunReport::from_live_results(
            &intent,
            &live_results(vec![feature(false)], Vec::new()),
            "legacy-live",
            true,
        );
        assert_eq!(failing.exit_code(), 1);
        assert_eq!(failing.coverage().required_bindings.covered, 1);
    }

    #[test]
    fn component_scenario_outcomes_use_explicit_ids_and_fail_closed() {
        let intent = IntentFile::parse_content_with_id_mode(
            r#"Component: Reusable
  id: component.reusable

  Scenario: Component check
    id: scenario.component.reusable-check
    When the check runs
    → id: outcome.component.reusable-valid; component is valid

Feature: Host
  id: feature.host
  verification: documentation-only
  rationale: Component-only compatibility fixture
"#,
            "component.intent".to_string(),
            IdMode::Strict,
        )
        .unwrap();
        let component = |status, assertion_passes| LiveComponentResult {
            component_id: "component.reusable".to_string(),
            component_name: "Reusable".to_string(),
            description: String::new(),
            inherent_behavior: Vec::new(),
            passed: assertion_passes,
            scenarios: vec![live_scenario(
                "Component check",
                status,
                Some(vec![live_assertion("component is valid", assertion_passes)]),
            )],
        };

        let passing = RunReport::from_live_results(
            &intent,
            &live_results(Vec::new(), vec![component("pass", true)]),
            "legacy-live",
            true,
        );
        assert_eq!(passing.exit_code(), 0);
        assert_eq!(
            passing.evidence()[0].obligation_id(),
            "outcome.component.reusable-valid"
        );
        assert_eq!(passing.evidence()[0].profile(), "legacy-live");
        assert_eq!(passing.evidence()[0].source.start_line, 7);

        for failing_component in [
            component("fail", false),
            component("warning", true),
            LiveComponentResult {
                scenarios: Vec::new(),
                ..component("pass", true)
            },
        ] {
            let failing = RunReport::from_live_results(
                &intent,
                &live_results(Vec::new(), vec![failing_component]),
                "legacy-live",
                true,
            );
            assert_eq!(failing.exit_code(), 1);
        }
    }

    #[test]
    fn component_mapping_preconditions_and_repeated_runs_are_all_required() {
        let intent = IntentFile::parse_content_with_id_mode(
            r#"Component: Expanded component
  id: component.expanded

  Scenario: Component row
    id: scenario.component.expanded-row
    Given component access is allowed
    When the check runs
    → id: outcome.component.expanded-valid; component row is valid

Feature: Host
  id: feature.host
  verification: documentation-only
  rationale: Component-only compatibility fixture
"#,
            "component-expanded.intent".to_string(),
            IdMode::Strict,
        )
        .unwrap();
        let component_report = |scenarios| {
            RunReport::from_live_results(
                &intent,
                &live_results(
                    Vec::new(),
                    vec![LiveComponentResult {
                        component_id: "component.expanded".to_string(),
                        component_name: "Expanded component".to_string(),
                        description: String::new(),
                        inherent_behavior: Vec::new(),
                        passed: true,
                        scenarios,
                    }],
                ),
                "legacy-live",
                true,
            )
        };

        let passing = component_report(vec![
            live_scenario(
                "Component row [first]",
                "pass",
                Some(vec![live_assertion("component row is valid", true)]),
            ),
            live_scenario(
                "Component row [second]",
                "pass",
                Some(vec![live_assertion("component row is valid", true)]),
            ),
        ]);
        assert_eq!(passing.exit_code(), 0);
        assert_eq!(passing.coverage().required_bindings.total, 2);

        let mut failed_precondition = live_scenario(
            "Component row",
            "fail",
            Some(vec![live_assertion("component row is valid", true)]),
        );
        failed_precondition
            .test_result
            .as_mut()
            .unwrap()
            .preconditions = vec![live_assertion("component access is allowed", false)];
        assert_eq!(component_report(vec![failed_precondition]).exit_code(), 1);

        for assertions in [
            Vec::new(),
            vec![
                live_assertion("component row is valid", true),
                live_assertion("extra", true),
            ],
        ] {
            assert_eq!(
                component_report(vec![live_scenario(
                    "Component row",
                    "pass",
                    Some(assertions),
                )])
                .exit_code(),
                1
            );
        }
    }

    #[test]
    fn failed_atom_then_pass_is_flaky_even_if_attempt_summary_claimed_pass() {
        let evidence = evidence(
            "binding.retry-summary",
            EvidenceRequirement::Required,
            EvidenceSelection::Selected,
            BindingStatus::Bound,
            ExecutabilityStatus::Executable,
            Freshness::Current,
            vec![
                attempt(1, Disposition::Passed, vec![atom("first", false)]),
                attempt(2, Disposition::Passed, vec![atom("second", true)]),
            ],
        );

        let report = report("full", vec![evidence]);

        assert_eq!(report.evidence()[0].disposition(), Disposition::Flaky);
        assert_eq!(report.exit_code(), 1);
    }

    #[test]
    fn profile_qualification_does_not_promote_fast_evidence_to_full() {
        let fast = evidence(
            "binding.fast",
            EvidenceRequirement::Required,
            EvidenceSelection::Selected,
            BindingStatus::Bound,
            ExecutabilityStatus::Executable,
            Freshness::Current,
            vec![attempt(1, Disposition::Passed, vec![atom("fast", true)])],
        );
        let excluded_from_full = evidence(
            "binding.fast",
            EvidenceRequirement::Required,
            EvidenceSelection::Excluded,
            BindingStatus::Bound,
            ExecutabilityStatus::Executable,
            Freshness::Current,
            vec![attempt(1, Disposition::Passed, vec![atom("fast", true)])],
        );

        let fast_report = report("fast", vec![fast]);
        assert_eq!(fast_report.exit_code(), 0);
        assert_eq!(fast_report.profile_qualification(), "profile:fast");

        let full_report = report("full", vec![excluded_from_full]);
        assert_eq!(full_report.exit_code(), 1);
        assert_eq!(full_report.coverage().verified.covered, 0);
    }

    #[test]
    fn full_profile_cannot_hide_an_obligation_behind_other_selected_evidence() {
        let selected = evidence(
            "binding.selected",
            EvidenceRequirement::Required,
            EvidenceSelection::Selected,
            BindingStatus::Bound,
            ExecutabilityStatus::Executable,
            Freshness::Current,
            vec![attempt(
                1,
                Disposition::Passed,
                vec![atom("selected", true)],
            )],
        );
        let mut excluded = evidence(
            "binding.excluded-other",
            EvidenceRequirement::Required,
            EvidenceSelection::Excluded,
            BindingStatus::Bound,
            ExecutabilityStatus::Executable,
            Freshness::Current,
            vec![attempt(
                1,
                Disposition::Passed,
                vec![atom("excluded", true)],
            )],
        );
        excluded.obligation_id = "outcome.report.other".to_string();

        let fast = report("fast", vec![selected.clone(), excluded.clone()]);
        assert_eq!(fast.exit_code(), 0);
        assert_eq!(fast.profile_qualification(), "profile:fast");

        let full = report("full", vec![selected, excluded]);
        assert_eq!(
            full.coverage().verified,
            CoverageMetric {
                covered: 1,
                total: 2,
            }
        );
        assert_eq!(full.exit_code(), 1);
        assert_eq!(
            serde_json::to_value(&full).unwrap()["unmet_required_binding_ids"],
            serde_json::json!(["binding.excluded-other"])
        );
    }

    #[test]
    fn executable_coverage_requires_every_selected_required_binding() {
        let executable = evidence(
            "binding.executable",
            EvidenceRequirement::Required,
            EvidenceSelection::Selected,
            BindingStatus::Bound,
            ExecutabilityStatus::Executable,
            Freshness::Current,
            vec![attempt(1, Disposition::Passed, vec![atom("pass", true)])],
        );
        let unsupported = evidence(
            "binding.unsupported",
            EvidenceRequirement::Required,
            EvidenceSelection::Selected,
            BindingStatus::Bound,
            ExecutabilityStatus::Unsupported,
            Freshness::Current,
            Vec::new(),
        );

        let report = report("full", vec![executable, unsupported]);

        assert_eq!(
            report.coverage().executable,
            CoverageMetric {
                covered: 0,
                total: 1,
            }
        );
        assert_eq!(report.exit_code(), 1);
    }

    #[test]
    fn every_selected_binding_requires_its_own_current_evidence_atom() {
        let with_atom = evidence(
            "binding.with-atom",
            EvidenceRequirement::Required,
            EvidenceSelection::Selected,
            BindingStatus::Bound,
            ExecutabilityStatus::Executable,
            Freshness::Current,
            vec![attempt(1, Disposition::Passed, vec![atom("present", true)])],
        );
        let without_atom = evidence(
            "binding.without-atom",
            EvidenceRequirement::Required,
            EvidenceSelection::Selected,
            BindingStatus::Bound,
            ExecutabilityStatus::Executable,
            Freshness::Current,
            vec![attempt(1, Disposition::Passed, vec![])],
        );

        let report = report("full", vec![with_atom, without_atom]);
        assert_eq!(report.exit_code(), 1);
        assert_eq!(report.coverage().required_bindings.total, 2);
        assert_eq!(report.coverage().required_bindings.covered, 1);
    }

    #[test]
    fn advisory_and_excluded_bindings_remain_visible_but_never_satisfy() {
        let advisory = evidence(
            "binding.advisory",
            EvidenceRequirement::Advisory,
            EvidenceSelection::Selected,
            BindingStatus::Bound,
            ExecutabilityStatus::Executable,
            Freshness::Current,
            vec![attempt(
                1,
                Disposition::Passed,
                vec![atom("advisory", true)],
            )],
        );
        let excluded = evidence(
            "binding.excluded",
            EvidenceRequirement::Required,
            EvidenceSelection::Excluded,
            BindingStatus::Bound,
            ExecutabilityStatus::Executable,
            Freshness::Current,
            vec![attempt(
                1,
                Disposition::Passed,
                vec![atom("excluded", true)],
            )],
        );

        let report = report("full", vec![advisory, excluded]);
        assert_eq!(report.evidence().len(), 2);
        assert_eq!(report.coverage().advisory_bindings, 1);
        assert_eq!(report.coverage().excluded_bindings, 1);
        assert_eq!(report.coverage().required_bindings.covered, 0);
        assert_eq!(report.exit_code(), 1);
    }

    #[test]
    fn human_totals_and_exit_status_come_from_the_serialized_ledger() {
        let passed = evidence(
            "binding.pass",
            EvidenceRequirement::Required,
            EvidenceSelection::Selected,
            BindingStatus::Bound,
            ExecutabilityStatus::Executable,
            Freshness::Current,
            vec![attempt(1, Disposition::Passed, vec![atom("pass", true)])],
        );
        let failed = evidence(
            "binding.fail",
            EvidenceRequirement::Required,
            EvidenceSelection::Selected,
            BindingStatus::Bound,
            ExecutabilityStatus::Executable,
            Freshness::Current,
            vec![attempt(1, Disposition::Failed, vec![atom("fail", false)])],
        );
        let report = report("full", vec![passed, failed]);
        let human = report.render_human(0);
        let json = serde_json::to_value(&report).unwrap();

        assert!(human.contains("1 passed, 1 failed"), "{human}");
        assert!(
            human.contains("Obligations: 0 verified, 1 unmet"),
            "{human}"
        );
        assert!(human.contains("Exit: 1"), "{human}");
        assert_eq!(json["coverage"]["required_bindings"]["covered"], 1);
        assert_eq!(json["coverage"]["required_bindings"]["total"], 2);
        assert_eq!(json["exit"]["code"], 1);
        assert_eq!(json["obligations"][0]["binding"], "Bound");
        assert_eq!(json["obligations"][0]["executability"], "Executable");
        assert_eq!(json["obligations"][0]["disposition"], "Failed");
        assert_eq!(json["obligations"][0]["freshness"], "Current");
    }

    #[test]
    fn implementation_coverage_fails_closed_for_unproven_behavioral_feature() {
        let intent = IntentFile::parse_content_with_id_mode(
            "Feature: Empty behavior\n  id: feature.empty-behavior\n",
            "empty-behavior.intent".to_string(),
            IdMode::Strict,
        )
        .unwrap();

        let report =
            RunReport::implementation_coverage(&intent, &[], CoverageThresholds::default());
        let json = serde_json::to_value(&report).unwrap();

        assert_eq!(report.profile_qualification(), "profile:implementation");
        assert_eq!(report.exit_code(), 1);
        assert_eq!(json["profile"], "implementation");
        assert_eq!(json["exit"]["reason"], "verification-failed");
    }

    #[test]
    fn summary_only_truth_cannot_fabricate_verified_evidence() {
        let intent = IntentFile::parse_content_with_id_mode(
            "Feature: Safe report\n  id: feature.safe-report\n\n  Scenario: Execute\n    id: scenario.safe-report.execute\n    When execution is recorded\n    → id: outcome.safe-report.executed; result is observed\n",
            "safe-report.intent".to_string(),
            IdMode::Strict,
        )
        .unwrap();
        let mut truth = VerificationTruth::from_intent(&intent);
        let obligation = &mut truth.obligations[0];
        obligation.binding = BindingStatus::Bound;
        obligation.executability = ExecutabilityStatus::Executable;
        obligation.disposition = Disposition::Passed;
        obligation.evidence_bindings.push(EvidenceBinding {
            id: "binding.summary-only".to_string(),
            obligation_id: obligation.id.to_string(),
            source: SourceSpan::single_line("verification.tnt", 1, 1, 10),
            declaration: DeclarationStatus::Declared,
            linkage: LinkageStatus::Linked,
            binding: BindingStatus::Bound,
            executability: ExecutabilityStatus::Executable,
            disposition: Disposition::Passed,
            freshness: Freshness::Current,
            assertion_resolution: AssertionResolution::Resolved,
            evidence_atoms: 99,
        });

        let report = RunReport::from_truth(&truth, "full", true);
        assert_eq!(report.coverage().verified.covered, 0);
        assert_eq!(report.evidence()[0].disposition(), Disposition::NoResult);
        assert_eq!(report.exit_code(), 1);
    }
}
