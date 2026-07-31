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
                evidence.push(
                    EvidenceResult::recorded(
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
                    )
                    .qualify(profile),
                );
            } else {
                for binding in &obligation.evidence_bindings {
                    // Slice 1A stores only a summary atom count. It is intentionally
                    // not expanded into evidence: only executor/live compatibility
                    // paths carrying concrete assertion atoms may verify coverage.
                    evidence.push(
                        EvidenceResult::recorded(
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
                        )
                        .qualify(profile),
                    );
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

        for (feature_index, feature) in truth.features.iter_mut().enumerate() {
            let (Some(intent_feature), Some(live_feature)) = (
                intent.features.get(feature_index),
                live.features.get(feature_index),
            ) else {
                continue;
            };
            let expected_live_id = intent_feature.id.as_deref().unwrap_or("unknown");
            if live_feature.feature_id != expected_live_id
                || live_feature.feature_name != intent_feature.name
            {
                continue;
            }
            let linkage = if live_feature.has_implementation {
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

            for scenario in &intent_feature.scenarios {
                let matching: Vec<&LiveScenarioResult> = live_feature
                    .scenarios
                    .iter()
                    .filter(|candidate| {
                        candidate.name == scenario.name
                            || candidate.name.starts_with(&format!("{} [", scenario.name))
                    })
                    .collect();
                for (run_index, live_scenario) in matching.iter().enumerate() {
                    append_live_scenario_evidence(
                        &mut evidence,
                        scenario,
                        live_scenario,
                        linkage,
                        profile,
                        run_index,
                    );
                }
            }
        }

        let evidenced: BTreeSet<String> = evidence
            .iter()
            .map(|result| result.obligation_id.clone())
            .collect();
        for obligation in &truth.obligations {
            if !evidenced.contains(obligation.id.as_str()) {
                evidence.push(
                    EvidenceResult::recorded(
                        format!("binding.unbound.{}", obligation.id),
                        obligation.id.to_string(),
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
                    )
                    .qualify(profile),
                );
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
        report.coverage =
            coverage_from_ledger(&report.features, &report.obligations, &report.evidence);
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

fn append_live_scenario_evidence(
    evidence: &mut Vec<EvidenceResult>,
    scenario: &crate::intent::Scenario,
    live: &LiveScenarioResult,
    linkage: LinkageStatus,
    profile: &str,
    run_index: usize,
) {
    let assertions = live
        .test_result
        .as_ref()
        .map(|result| result.assertions.as_slice())
        .unwrap_or_default();
    let exact_mapping = assertions.len() == scenario.outcome_metadata.len();

    for (outcome_index, metadata) in scenario.outcome_metadata.iter().enumerate() {
        let (binding, executability, resolution, attempts) = match live.status.as_str() {
            "skip" => (
                BindingStatus::Bound,
                ExecutabilityStatus::Executable,
                AssertionResolution::Resolved,
                vec![EvidenceAttempt {
                    sequence: 1,
                    disposition: Disposition::Skipped,
                    evidence_atoms: Vec::new(),
                    diagnostic: Some("precondition was not met".to_string()),
                }],
            ),
            "pass" | "fail" if exact_mapping => {
                let assertion = &assertions[outcome_index];
                let disposition = if assertion.passed {
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
                        evidence_atoms: vec![EvidenceAtom {
                            id: format!(
                                "atom.{}.{}.{}",
                                metadata.id,
                                run_index + 1,
                                outcome_index + 1
                            ),
                            assertion: assertion.assertion_text.clone(),
                            passed: assertion.passed,
                            diagnostic: assertion.message.clone(),
                        }],
                        diagnostic: None,
                    }],
                )
            }
            "pass" | "fail" => (
                BindingStatus::Bound,
                ExecutabilityStatus::Executable,
                AssertionResolution::Unresolved,
                vec![EvidenceAttempt {
                    sequence: 1,
                    disposition: Disposition::NoResult,
                    evidence_atoms: Vec::new(),
                    diagnostic: Some(format!(
                        "cannot safely map {} assertions to {} outcomes",
                        assertions.len(),
                        scenario.outcome_metadata.len()
                    )),
                }],
            ),
            _ => (
                BindingStatus::Unbound,
                ExecutabilityStatus::Unsupported,
                AssertionResolution::Unresolved,
                Vec::new(),
            ),
        };
        evidence.push(
            EvidenceResult::recorded(
                format!(
                    "binding.compat.{}.{}.{}",
                    scenario.verification_id,
                    run_index + 1,
                    outcome_index + 1
                ),
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
            )
            .qualify(profile),
        );
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
