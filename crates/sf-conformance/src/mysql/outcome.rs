use crate::manifest::Case;
use crate::runner::parse_error_outcome;
use crate::sealed_suite::{ClassifiedReport, OutcomeCode};
use crate::{CaseResult, Report, Status};

pub(super) struct CaseOutcome {
    pub(super) status: Status,
    pub(super) code: OutcomeCode,
    pub(super) reason: String,
}

pub(super) fn outcome(status: Status, code: OutcomeCode, reason: impl Into<String>) -> CaseOutcome {
    CaseOutcome {
        status,
        code,
        reason: reason.into(),
    }
}

pub(super) fn classified_error(case: &Case, code: OutcomeCode, detail: String) -> CaseOutcome {
    let (status, reason) = parse_error_outcome(case, &detail);
    outcome(status, code, reason)
}

pub(super) fn classify_comparison(
    (status, reason): (Status, String),
    passed: OutcomeCode,
    failed: OutcomeCode,
) -> CaseOutcome {
    outcome(
        status,
        if status == Status::Passed {
            passed
        } else {
            failed
        },
        reason,
    )
}

pub(super) fn plain_report(report: ClassifiedReport) -> Report {
    Report {
        cases: report
            .cases
            .into_iter()
            .map(|case| CaseResult {
                id: case.id,
                kind: case.kind,
                status: case.status,
                reason: case.reason,
            })
            .collect(),
    }
}
