//! S01 exit evidence: deny-by-default policy engine / action matrix coverage.
//!
//! Authority: docs/v1/DELIVERY.md S01, docs/v1/SECURITY.md §3.

use gump_types::{Action, DecisionEffect, PolicyEngine, PrincipalId, Role};

fn principal(id: &str) -> PrincipalId {
    PrincipalId::new(id).unwrap()
}

#[test]
fn deny_by_default_covers_full_action_matrix() {
    let mut engine = PolicyEngine::new();
    let nobody = principal("oidc:anonymous");
    for action in Action::coverage_matrix() {
        let d = engine.authorize(&nobody, &action);
        assert_eq!(
            d.effect,
            DecisionEffect::Deny,
            "{} must be deny-by-default",
            action.as_str()
        );
        assert_eq!(d.reason, "deny_by_default");
        assert!(!d.allowed());
    }
}

#[test]
fn explicit_grant_allows_only_that_action() {
    let mut engine = PolicyEngine::new();
    let p = principal("oidc:alice");
    engine.grant(p.clone(), Action::WorkloadRead);
    assert!(engine.authorize(&p, &Action::WorkloadRead).allowed());
    assert!(!engine.authorize(&p, &Action::WorkloadDeploy).allowed());
    assert!(!engine.authorize(&p, &Action::PolicyManage).allowed());
}

#[test]
fn roles_are_bundles_enforcement_checks_actions() {
    let mut engine = PolicyEngine::new();
    let p = principal("oidc:ops");
    engine.bind_role(p.clone(), Role::Operator);
    assert!(engine.authorize(&p, &Action::ClusterManage).allowed());
    assert!(engine.authorize(&p, &Action::PolicyManage).allowed());
    // Operator bundle does not include workload.deploy.
    assert!(!engine.authorize(&p, &Action::WorkloadDeploy).allowed());
}

#[test]
fn deployer_and_reader_role_matrix() {
    let mut engine = PolicyEngine::new();
    let dep = principal("oidc:deployer");
    let reader = principal("oidc:reader");
    engine.bind_role(dep.clone(), Role::Deployer);
    engine.bind_role(reader.clone(), Role::Reader);

    assert!(engine.authorize(&dep, &Action::WorkloadDeploy).allowed());
    assert!(engine
        .authorize(
            &dep,
            &Action::PublicationUse {
                provider: "kismet".into()
            }
        )
        .allowed());
    assert!(!engine.authorize(&dep, &Action::ClusterUnseal).allowed());

    assert!(engine.authorize(&reader, &Action::WorkloadRead).allowed());
    assert!(!engine.authorize(&reader, &Action::WorkloadDeploy).allowed());
    assert!(!engine.authorize(&reader, &Action::SecretDeliver).allowed());
}

#[test]
fn agent_wildcard_scopes_connector_and_hiccup_topics() {
    let mut engine = PolicyEngine::new();
    let agent = principal("node:agent-1");
    engine.bind_role(agent.clone(), Role::Agent);
    assert!(engine
        .authorize(
            &agent,
            &Action::ConnectorUse {
                name: "s3".into()
            }
        )
        .allowed());
    assert!(engine
        .authorize(
            &agent,
            &Action::HiccupPublish {
                topic: "peers".into()
            }
        )
        .allowed());
    assert!(!engine.authorize(&agent, &Action::WorkloadDeploy).allowed());
}

#[test]
fn parameterized_grant_does_not_cross_scope() {
    let mut engine = PolicyEngine::new();
    let p = principal("oidc:alice");
    engine.grant(
        p.clone(),
        Action::HiccupPublish {
            topic: "a".into(),
        },
    );
    assert!(engine
        .authorize(
            &p,
            &Action::HiccupPublish {
                topic: "a".into()
            }
        )
        .allowed());
    assert!(!engine
        .authorize(
            &p,
            &Action::HiccupPublish {
                topic: "b".into()
            }
        )
        .allowed());
}

#[test]
fn decision_carries_policy_revision_and_id() {
    let mut engine = PolicyEngine::new();
    let p = principal("oidc:alice");
    engine.grant(p.clone(), Action::AuditRead);
    let rev = engine.revision();
    let d = engine.authorize(&p, &Action::AuditRead);
    assert!(d.decision_id.starts_with("pd-"));
    assert_eq!(d.policy_revision, rev);
}

#[test]
fn revoke_grant_returns_to_deny() {
    let mut engine = PolicyEngine::new();
    let p = principal("oidc:alice");
    engine.grant(p.clone(), Action::TelemetrySubscribe);
    assert!(engine.authorize(&p, &Action::TelemetrySubscribe).allowed());
    assert!(engine.revoke_grant(&p, &Action::TelemetrySubscribe));
    assert!(!engine.authorize(&p, &Action::TelemetrySubscribe).allowed());
}

#[test]
fn action_as_str_matches_security_contract_names() {
    assert_eq!(Action::WorkloadDeploy.as_str(), "workload.deploy");
    assert_eq!(Action::PolicyManage.as_str(), "policy.manage");
    assert_eq!(
        Action::HiccupPublish {
            topic: "t".into()
        }
        .as_str(),
        "hiccup.publish:t"
    );
    assert_eq!(
        Action::coverage_matrix().len(),
        26,
        "SECURITY.md §3 lists 26 action forms in the coverage matrix"
    );
}
