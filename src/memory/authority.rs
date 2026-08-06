use super::{
    ActorMemoryGrant, AuthorityLevel, AuthorityOrigin, MemoryClaim, MemoryPolicy, MemoryQuery,
    ProvenanceRole,
};

pub(crate) fn origin_ceiling(origin: AuthorityOrigin) -> AuthorityLevel {
    match origin {
        AuthorityOrigin::Inferred | AuthorityOrigin::Tool | AuthorityOrigin::External => {
            AuthorityLevel::Observation
        }
        AuthorityOrigin::User => AuthorityLevel::ProjectFact,
        AuthorityOrigin::Operator | AuthorityOrigin::Reviewer => AuthorityLevel::ProjectConstraint,
    }
}

pub(crate) fn provenance_role_ceiling(role: ProvenanceRole) -> AuthorityLevel {
    match role {
        ProvenanceRole::AgentInference
        | ProvenanceRole::ExternalMaterial
        | ProvenanceRole::ToolObservation
        | ProvenanceRole::UnverifiedThirdPartyStatement => AuthorityLevel::Observation,
        ProvenanceRole::UserRequirement => AuthorityLevel::ProjectFact,
        ProvenanceRole::OperatorInstruction | ProvenanceRole::ReviewedArchitectureDecision => {
            AuthorityLevel::ProjectConstraint
        }
        ProvenanceRole::CanonicalHumanStatement => AuthorityLevel::ProjectAuthority,
    }
}

pub(crate) fn actor_grant<'a>(
    policy: &'a MemoryPolicy,
    query: &MemoryQuery,
) -> Option<&'a ActorMemoryGrant> {
    policy.actor_grants.iter().find(|grant| {
        grant.actor_id == query.requesting_actor_id
            && grant.allowed_audiences.contains(&query.audience)
            && grant.allowed_purposes.contains(&query.purpose)
            && query.scope.project_id.as_ref().map_or(true, |project| {
                grant.project_ids.contains(project) && project == &policy.project_id
            })
    })
}

pub(crate) fn claim_within_grant(claim: &MemoryClaim, grant: &ActorMemoryGrant) -> bool {
    claim.sensitivity <= grant.max_sensitivity && claim.authority_ceiling <= grant.max_authority
}

pub(crate) fn claim_in_scope(claim: &MemoryClaim, query: &MemoryQuery) -> bool {
    if let (Some(claim_case), Some(query_case)) =
        (&claim.scope.case_space_id, &query.scope.case_space_id)
    {
        if claim_case != query_case {
            return false;
        }
    }
    if let (Some(claim_project), Some(query_project)) =
        (&claim.scope.project_id, &query.scope.project_id)
    {
        if claim_project != query_project {
            return false;
        }
    }
    if !claim.scope.actor_ids.is_empty()
        && !claim.scope.actor_ids.contains(&query.requesting_actor_id)
    {
        return false;
    }
    query.scope.actor_ids.is_empty()
        || claim.scope.actor_ids.is_empty()
        || query
            .scope
            .actor_ids
            .iter()
            .any(|actor| claim.scope.actor_ids.contains(actor))
}
