use higher_graphen_core::ReviewStatus;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum EvidenceTrustBoundary {
    SourceBacked,
    Inferred,
    WorkerOutput,
    ReviewPromoted,
    Rejected,
    Contradicting,
}

impl EvidenceTrustBoundary {
    /// The `metadata.evidence_boundary` spelling the native evaluator parses
    /// back into this boundary. Writers that materialize evidence cells must
    /// use this instead of spelling the strings themselves.
    pub(crate) fn metadata_value(self) -> &'static str {
        match self {
            Self::SourceBacked => "source_backed",
            Self::Inferred => "inferred",
            Self::WorkerOutput => "worker_output",
            Self::ReviewPromoted => "review_promoted",
            Self::Rejected => "rejected",
            Self::Contradicting => "contradicting",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct EvidenceTrustInput {
    pub(crate) boundary: EvidenceTrustBoundary,
    pub(crate) cell_review_status: ReviewStatus,
    pub(crate) latest_review_status: Option<ReviewStatus>,
    pub(crate) has_source: bool,
}

pub(crate) fn evidence_is_acceptable(input: EvidenceTrustInput) -> bool {
    if !input.has_source
        || input.cell_review_status == ReviewStatus::Rejected
        || input.latest_review_status == Some(ReviewStatus::Rejected)
    {
        return false;
    }

    match input.boundary {
        EvidenceTrustBoundary::SourceBacked => true,
        EvidenceTrustBoundary::ReviewPromoted => {
            input.cell_review_status == ReviewStatus::Accepted
                || input.latest_review_status == Some(ReviewStatus::Accepted)
        }
        EvidenceTrustBoundary::Inferred | EvidenceTrustBoundary::WorkerOutput => {
            input.latest_review_status == Some(ReviewStatus::Accepted)
        }
        EvidenceTrustBoundary::Rejected | EvidenceTrustBoundary::Contradicting => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evidence_acceptability_truth_table_matches_hardened_policy() {
        use EvidenceTrustBoundary::{
            Contradicting, Inferred, Rejected, ReviewPromoted, SourceBacked, WorkerOutput,
        };
        use ReviewStatus::{Accepted, Rejected as ReviewRejected, Unreviewed};

        let cases = [
            (SourceBacked, Unreviewed, None, true, true),
            (SourceBacked, Accepted, None, true, true),
            (SourceBacked, ReviewRejected, None, true, false),
            (SourceBacked, Accepted, Some(ReviewRejected), true, false),
            (SourceBacked, Accepted, None, false, false),
            (ReviewPromoted, Unreviewed, None, true, false),
            (ReviewPromoted, Accepted, None, true, true),
            (ReviewPromoted, Unreviewed, Some(Accepted), true, true),
            (ReviewPromoted, Accepted, Some(ReviewRejected), true, false),
            (Inferred, Accepted, None, true, false),
            (Inferred, Unreviewed, Some(Accepted), true, true),
            (Inferred, Unreviewed, Some(ReviewRejected), true, false),
            (WorkerOutput, Accepted, None, true, false),
            (WorkerOutput, Unreviewed, Some(Accepted), true, true),
            (Rejected, Accepted, Some(Accepted), true, false),
            (Contradicting, Accepted, Some(Accepted), true, false),
        ];

        for (boundary, cell_review_status, latest_review_status, has_source, expected) in cases {
            assert_eq!(
                evidence_is_acceptable(EvidenceTrustInput {
                    boundary,
                    cell_review_status,
                    latest_review_status,
                    has_source,
                }),
                expected,
                "unexpected result for {boundary:?}, cell={cell_review_status:?}, \
                 latest={latest_review_status:?}, has_source={has_source}"
            );
        }
    }

    #[test]
    fn metadata_values_round_trip_through_the_native_parser() {
        use crate::native_model::EvidenceBoundary;
        for boundary in [
            EvidenceTrustBoundary::SourceBacked,
            EvidenceTrustBoundary::Inferred,
            EvidenceTrustBoundary::WorkerOutput,
            EvidenceTrustBoundary::ReviewPromoted,
            EvidenceTrustBoundary::Rejected,
            EvidenceTrustBoundary::Contradicting,
        ] {
            let parsed: EvidenceTrustBoundary =
                EvidenceBoundary::from_metadata_value(Some(boundary.metadata_value())).into();
            assert_eq!(parsed, boundary, "metadata spelling does not round-trip");
        }
    }
}
