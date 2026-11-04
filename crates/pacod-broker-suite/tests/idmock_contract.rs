//! The mock pacod-id against the vendored pacod-id client. The oidc scenarios are only as good
//! as the mock is faithful; this test makes a drift between the two fail here, by name, rather
//! than as a puzzling scenario failure.

use std::collections::HashMap;
use std::time::Duration;

use pacod_broker_suite::idmock::IdMock;
use pacod_broker_suite::scenario::{IdentitySetup, PrincipalSetup};
use pacod_id_client::{AuthError, Authenticator, AuthzCache, Caller, IdClient, IdConfig};

fn identity() -> IdentitySetup {
    IdentitySetup {
        organisations: vec!["acme".into()],
        principals: vec![
            PrincipalSetup {
                name: "farm-svc".into(),
                kind: "service".into(),
                organisation: Some("acme".into()),
                roles: vec!["broker.provider@organisation".into()],
            },
            PrincipalSetup {
                name: "alice".into(),
                kind: "user".into(),
                organisation: Some("acme".into()),
                roles: vec![],
            },
        ],
    }
}

fn client(id: &IdMock) -> IdClient {
    IdClient::new(IdConfig {
        issuers: vec![id.issuer().to_string()],
        internal_url: id.url.clone(),
        client_id: "pacod-broker".into(),
        client_secret: "suite".into(),
        product: "broker".into(),
        audience: "pacod-broker".into(),
    })
}

#[tokio::test]
async fn the_vendored_client_reads_the_mock() {
    let id = IdMock::start(&identity()).await.unwrap();
    let c = client(&id);

    // The service token and the manifest push, which the mock checks as pacod-id does.
    assert!(!c.service_token().await.unwrap().is_empty());
    let manifest = pacod_id_authz::Manifest::from_yaml(include_str!("../../../authz/broker.yaml")).unwrap();
    c.push_catalog(manifest.source(), &HashMap::new()).await.unwrap();
    assert!(id.catalog().is_some());

    // The snapshot and the feed.
    let (_stop, rx) = tokio::sync::watch::channel(false);
    let cache = AuthzCache::spawn(c.clone(), rx);
    let end = tokio::time::Instant::now() + Duration::from_secs(5);
    while !cache.is_ready() {
        assert!(tokio::time::Instant::now() < end, "the cache never read the mock's snapshot");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let svc = id.principal("farm-svc").unwrap();
    let held = cache.bindings_of(&svc.sub);
    assert_eq!(held.len(), 1);
    assert_eq!(held[0].role, "broker.provider");
    assert_eq!(held[0].path.len(), 2, "platform, then the organisation: {:?}", held[0].path);

    let auth = Authenticator::new(c.clone(), cache.clone());

    // A service account's `pidt_` token, by introspection.
    match auth.authenticate(&id.token("farm-svc").unwrap()).await.unwrap() {
        Caller::Service { sub, organisation_id, bindings, .. } => {
            assert_eq!(sub, svc.sub);
            assert_eq!(organisation_id.as_deref(), Some("acme"));
            assert_eq!(bindings.len(), 1);
        }
        other => panic!("expected a service, got {other:?}"),
    }

    // A person's signed access token, by the JWKS.
    let alice = id.principal("alice").unwrap();
    match auth.authenticate(&id.token("alice").unwrap()).await.unwrap() {
        Caller::User { sub, .. } => assert_eq!(sub, alice.sub),
        other => panic!("expected a user, got {other:?}"),
    }

    // The runner's own admin.
    let admin = id.principal("suite-admin").unwrap();
    assert_eq!(cache.bindings_of(&admin.sub)[0].role, "broker.admin");

    // Unknown tokens are refused; with the mock down, a fresh token cannot be checked.
    assert!(matches!(auth.authenticate("pidt_nobody_0000").await, Err(AuthError::Invalid)));
    id.down();
    let fresh = id.fresh_token("farm-svc").unwrap();
    assert!(matches!(auth.authenticate(&fresh).await, Err(AuthError::Unavailable(_))));
    id.up();
    assert!(auth.authenticate(&fresh).await.is_ok());
}
