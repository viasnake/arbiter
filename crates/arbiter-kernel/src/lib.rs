use chrono::{DateTime, Utc};
use serde_json::Value;
use sha2::{Digest, Sha256};

pub mod policy {
    use arbiter_contracts::DecisionEffect;
    use serde_json::Value;

    #[derive(Debug, Clone)]
    pub struct PolicyInput {
        pub provider: String,
        pub capability: String,
        pub intent_type: String,
        pub risk_level: String,
        pub metadata: Value,
    }

    #[derive(Debug, Clone)]
    pub struct PolicyConfig {
        pub allowed_providers: Vec<String>,
        pub capability_allowlist: Vec<String>,
        pub capability_denylist: Vec<String>,
        pub require_approval_for_write_external: bool,
        pub require_approval_for_notify: bool,
        pub require_approval_for_start_job: bool,
        pub require_approval_for_production: bool,
    }

    #[derive(Debug, Clone)]
    pub struct PolicyDecision {
        pub effect: DecisionEffect,
        pub applied_policies: Vec<String>,
        pub rationale: String,
        pub required_approvers: Vec<String>,
        pub permit_constraints: Value,
    }

    #[derive(Debug, Clone)]
    pub struct ApproverResolverConfig {
        pub default_approvers: Vec<String>,
        pub production_approvers: Vec<String>,
    }

    pub fn resolve_approvers(environment: &str, config: &ApproverResolverConfig) -> Vec<String> {
        if environment == "prod" && !config.production_approvers.is_empty() {
            return config.production_approvers.clone();
        }
        config.default_approvers.clone()
    }

    pub fn evaluate(
        input: &PolicyInput,
        environment: &str,
        config: &PolicyConfig,
        approvers: Vec<String>,
    ) -> PolicyDecision {
        if !config
            .allowed_providers
            .iter()
            .any(|v| v == &input.provider)
        {
            return PolicyDecision {
                effect: DecisionEffect::Deny,
                applied_policies: vec!["provider.allowed_list".to_string()],
                rationale: format!("provider '{}' is not allowed", input.provider),
                required_approvers: vec![],
                permit_constraints: serde_json::json!({}),
            };
        }

        if config
            .capability_denylist
            .iter()
            .any(|v| v == &input.capability)
        {
            return PolicyDecision {
                effect: DecisionEffect::Deny,
                applied_policies: vec!["capability.denylist".to_string()],
                rationale: format!("capability '{}' is denied", input.capability),
                required_approvers: vec![],
                permit_constraints: serde_json::json!({}),
            };
        }

        if !config.capability_allowlist.is_empty()
            && !config
                .capability_allowlist
                .iter()
                .any(|v| v == &input.capability)
        {
            return PolicyDecision {
                effect: DecisionEffect::Deny,
                applied_policies: vec!["capability.allowlist".to_string()],
                rationale: format!("capability '{}' is not allowed", input.capability),
                required_approvers: vec![],
                permit_constraints: serde_json::json!({}),
            };
        }

        let risky = input.risk_level == "write"
            || input.risk_level == "external"
            || input.risk_level == "high";
        let notify = input.intent_type == "notify";
        let start_job = input.intent_type == "start_job";
        if (config.require_approval_for_write_external && risky)
            || (config.require_approval_for_notify && notify)
            || (config.require_approval_for_start_job && start_job)
            || (config.require_approval_for_production && environment == "prod")
        {
            return PolicyDecision {
                effect: DecisionEffect::RequireApproval,
                applied_policies: vec!["approval.required".to_string()],
                rationale: "step requires approval by policy".to_string(),
                required_approvers: approvers,
                permit_constraints: serde_json::json!({"approval_required": true}),
            };
        }

        PolicyDecision {
            effect: DecisionEffect::Allow,
            applied_policies: vec!["default.allow".to_string()],
            rationale: "step allowed by policy".to_string(),
            required_approvers: vec![],
            permit_constraints: serde_json::json!({"approval_required": false}),
        }
    }
}

pub mod state_machine {
    use arbiter_contracts::{ApprovalStatus, RunStatus, StepStatus};

    pub fn can_transition_run(current: &RunStatus, next: &RunStatus) -> bool {
        matches!(
            (current, next),
            (RunStatus::Accepted, RunStatus::Planning)
                | (RunStatus::Planning, RunStatus::WaitingForApproval)
                | (RunStatus::Planning, RunStatus::Ready)
                | (RunStatus::WaitingForApproval, RunStatus::Ready)
                | (RunStatus::WaitingForApproval, RunStatus::Blocked)
                | (RunStatus::Ready, RunStatus::Running)
                | (RunStatus::Ready, RunStatus::Blocked)
                | (RunStatus::Running, RunStatus::Succeeded)
                | (RunStatus::Running, RunStatus::Failed)
                | (RunStatus::Running, RunStatus::Blocked)
        ) || matches!(next, RunStatus::Cancelled)
    }

    pub fn can_transition_step(current: &StepStatus, next: &StepStatus) -> bool {
        matches!(
            (current, next),
            (StepStatus::Declared, StepStatus::Evaluating)
                | (StepStatus::Evaluating, StepStatus::ApprovalRequired)
                | (StepStatus::Evaluating, StepStatus::Permitted)
                | (StepStatus::Permitted, StepStatus::Executing)
                | (StepStatus::Executing, StepStatus::Completed)
                | (StepStatus::ApprovalRequired, StepStatus::Permitted)
                | (StepStatus::ApprovalRequired, StepStatus::Rejected)
                | (StepStatus::Permitted, StepStatus::Failed)
                | (StepStatus::Executing, StepStatus::Failed)
        ) || matches!(next, StepStatus::Cancelled)
    }

    pub fn can_transition_approval(current: &ApprovalStatus, next: &ApprovalStatus) -> bool {
        matches!(
            (current, next),
            (ApprovalStatus::Requested, ApprovalStatus::Granted)
                | (ApprovalStatus::Requested, ApprovalStatus::Denied)
                | (ApprovalStatus::Requested, ApprovalStatus::Cancelled)
        )
    }
}

pub fn parse_rfc3339(ts: &str) -> Option<DateTime<Utc>> {
    chrono::DateTime::parse_from_rfc3339(ts)
        .ok()
        .map(|v| v.with_timezone(&Utc))
}

pub fn jcs_sha256_hex(value: &Value) -> Result<String, String> {
    let canonical = serde_jcs::to_string(value)
        .map_err(|err| format!("failed to canonicalize JSON via JCS: {err}"))?;
    Ok(sha256_hex(canonical.as_bytes()))
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy::{
        evaluate, resolve_approvers, ApproverResolverConfig, PolicyConfig, PolicyInput,
    };
    use crate::state_machine::{can_transition_approval, can_transition_run, can_transition_step};
    use arbiter_contracts::{ApprovalStatus, DecisionEffect, RunStatus, StepStatus};
    use serde_json::json;

    #[test]
    fn jcs_hash_is_order_independent() {
        let a = json!({"b":1,"a":2});
        let b = json!({"a":2,"b":1});
        assert_eq!(jcs_sha256_hex(&a).unwrap(), jcs_sha256_hex(&b).unwrap());
    }

    #[test]
    fn parse_rfc3339_works() {
        assert!(parse_rfc3339("2026-01-01T00:00:00Z").is_some());
    }

    #[test]
    fn state_machine_validates_run_and_step() {
        assert!(can_transition_run(
            &RunStatus::Accepted,
            &RunStatus::Planning
        ));
        assert!(!can_transition_run(
            &RunStatus::Accepted,
            &RunStatus::Succeeded
        ));
        assert!(can_transition_step(
            &StepStatus::Declared,
            &StepStatus::Evaluating
        ));
        assert!(!can_transition_step(
            &StepStatus::Declared,
            &StepStatus::Completed
        ));
        assert!(can_transition_approval(
            &ApprovalStatus::Requested,
            &ApprovalStatus::Granted
        ));
        assert!(!can_transition_approval(
            &ApprovalStatus::Granted,
            &ApprovalStatus::Denied
        ));
    }

    #[test]
    fn policy_requires_approval_on_prod_or_high_risk() {
        let cfg = PolicyConfig {
            allowed_providers: vec!["generic".to_string()],
            capability_allowlist: vec![],
            capability_denylist: vec![],
            require_approval_for_write_external: true,
            require_approval_for_notify: false,
            require_approval_for_start_job: false,
            require_approval_for_production: true,
        };
        let approver_cfg = ApproverResolverConfig {
            default_approvers: vec!["team-lead".to_string()],
            production_approvers: vec!["prod-owner".to_string()],
        };
        let approvers = resolve_approvers("prod", &approver_cfg);
        let decision = evaluate(
            &PolicyInput {
                provider: "generic".to_string(),
                capability: "write_db".to_string(),
                intent_type: "change".to_string(),
                risk_level: "write".to_string(),
                metadata: json!({}),
            },
            "prod",
            &cfg,
            approvers,
        );
        assert_eq!(decision.effect, DecisionEffect::RequireApproval);
    }

    #[test]
    fn policy_can_require_approval_for_notify() {
        let cfg = PolicyConfig {
            allowed_providers: vec!["generic".to_string()],
            capability_allowlist: vec![],
            capability_denylist: vec![],
            require_approval_for_write_external: false,
            require_approval_for_notify: true,
            require_approval_for_start_job: false,
            require_approval_for_production: false,
        };
        let approver_cfg = ApproverResolverConfig {
            default_approvers: vec!["team-lead".to_string()],
            production_approvers: vec![],
        };
        let decision = evaluate(
            &PolicyInput {
                provider: "generic".to_string(),
                capability: "notify".to_string(),
                intent_type: "notify".to_string(),
                risk_level: "low".to_string(),
                metadata: json!({}),
            },
            "dev",
            &cfg,
            resolve_approvers("dev", &approver_cfg),
        );
        assert_eq!(decision.effect, DecisionEffect::RequireApproval);
    }
}
