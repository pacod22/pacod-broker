use std::sync::Arc;

use jiff::{SignedDuration, Timestamp};
use pacod_broker_model::{LiveStatus, Source};
use pacod_broker_reputation::{Engine, Settings};
use serde_json::json;

use super::*;
use crate::config::Config;

fn registry(cfg: Config) -> Arc<Registry> {
    Registry::new(Arc::new(cfg), Arc::new(Engine::new(Settings::default(), None)))
}

fn who(name: &str) -> Announcer {
    Announcer { principal: format!("anon:{name}"), organisation_id: None, verified: false }
}

fn announcement(provider: &str, keys: &[&str], ttl: u64) -> Announcement {
    serde_json::from_value(json!({
        "ttl_s": ttl,
        "provider": {"name": provider, "summary": "A provider"},
        "resources": keys.iter().map(|k| json!({
            "key": k, "kind": "model", "name": format!("model-{k}"), "summary": "A model",
            "interfaces": [{"protocol": "openai-chat", "url": "https://x.example/v1"}],
            "pricing": {"currency": "EUR", "periods": [{"valid_from": "2026-01-01T00:00:00Z", "rates": {"input": 1.0}}]},
            "access": {"visibility": "public"}
        })).collect::<Vec<_>>()
    }))
    .unwrap()
}

#[test]
fn announce_heartbeat_and_expire() {
    let r = registry(Config::for_tests());
    let a = r.announce(&who("p"), announcement("p", &["a", "b"], 5), Source::Lease).unwrap();
    assert_eq!(a.resources.len(), 2);
    assert_eq!(a.heartbeat_every_s, 1);
    assert_eq!(r.rebuild().len(), 2);

    let hb = Heartbeat { status: Some(LiveStatus::Degraded), in_use: Some(3), ..Heartbeat::default() };
    r.heartbeat(Some("anon:p"), &a.lease_id, hb).unwrap();
    let idx = r.index();
    let e = idx.get(&a.resources["a"]).unwrap();
    assert_eq!((e.live.status(), e.live.in_use()), (LiveStatus::Degraded, Some(3)));

    // Another principal may not keep someone else's lease.
    assert!(matches!(r.heartbeat(Some("anon:q"), &a.lease_id, Heartbeat::default()), Err(RegistryError::Forbidden(_))));

    // Past the lease the sweeper ends it and the resources go.
    let later = Timestamp::now().checked_add(SignedDuration::from_secs(10)).unwrap();
    assert_eq!(r.sweep(later), 1);
    assert_eq!(r.rebuild().len(), 0);
    assert!(matches!(r.heartbeat(None, &a.lease_id, Heartbeat::default()), Err(RegistryError::NotFound(_))));
}

#[test]
fn announcing_again_replaces_the_set_and_keeps_ids() {
    let r = registry(Config::for_tests());
    let a = r.announce(&who("p"), announcement("p", &["a", "b"], 30), Source::Lease).unwrap();
    let mut again = announcement("p", &["b", "c"], 30);
    again.lease_id = Some(a.lease_id.clone());
    let b = r.announce(&who("p"), again, Source::Lease).unwrap();
    assert_eq!(a.lease_id, b.lease_id);
    assert_eq!(a.resources["b"], b.resources["b"]);
    let idx = r.rebuild();
    assert_eq!(idx.len(), 2);
    assert!(idx.get(&a.resources["a"]).is_none());
}

#[test]
fn bad_announcements_say_every_problem() {
    let r = registry(Config::for_tests());
    let mut a = announcement("p", &["a", "a"], 1);
    a.provider.name = "Bad Name".into();
    let Err(RegistryError::Invalid(p)) = r.announce(&who("p"), a, Source::Lease) else { panic!("refused") };
    let all = p.to_string();
    for want in ["provider.name", "resources[1].key: not unique", "ttl_s"] {
        assert!(all.contains(want), "{want} in {all}");
    }
}

#[test]
fn prices_in_force_cannot_be_rewritten() {
    let r = registry(Config::for_tests());
    let a = r.announce(&who("p"), announcement("p", &["a"], 30), Source::Lease).unwrap();
    let mut b = announcement("p", &["a"], 30);
    b.lease_id = Some(a.lease_id);
    b.resources[0].pricing.as_mut().unwrap().periods[0].rates.input = Some(2.0);
    let Err(RegistryError::Invalid(p)) = r.announce(&who("p"), b, Source::Lease) else { panic!("refused") };
    assert!(p.to_string().contains("never retroactive"), "{p}");
}

#[test]
fn limits_blocking_and_approval() {
    let mut cfg = Config::for_tests();
    cfg.max_leases_per_provider = 1;
    cfg.approval = crate::config::Approval::NewProviders;
    let r = registry(cfg);
    let Err(RegistryError::Pending(pid)) = r.announce(&who("p"), announcement("p", &["a"], 30), Source::Lease) else {
        panic!("pending")
    };
    assert_eq!(r.pending().len(), 1);
    r.approve(&pid);
    r.announce(&who("p"), announcement("p", &["a"], 30), Source::Lease).unwrap();
    assert!(matches!(
        r.announce(&who("p"), announcement("p", &["b"], 30), Source::Lease),
        Err(RegistryError::Limit(_))
    ));
    r.block(&pid);
    assert_eq!(r.leases().len(), 0);
    assert!(matches!(
        r.announce(&who("p"), announcement("p", &["a"], 30), Source::Lease),
        Err(RegistryError::Forbidden(_))
    ));
}

#[test]
fn static_entries_are_replaced_per_provider() {
    let r = registry(Config::for_tests());
    r.put_static("static:s", true, announcement("s", &["a", "b"], 30)).unwrap();
    r.put_static("static:s", true, announcement("s", &["b"], 30)).unwrap();
    let idx = r.rebuild();
    assert_eq!(idx.len(), 1);
    assert!(idx.entries().all(|e| e.provider.verified && e.record.source == Source::Static));
    assert_eq!(r.static_entries()[0].resources.len(), 1);
    // Static entries never expire.
    let far = Timestamp::now().checked_add(SignedDuration::from_hours(1000)).unwrap();
    assert_eq!(r.sweep(far), 0);
    assert!(idx.entries().all(|e| !e.live.is_expired(far)));
}

#[test]
fn a_dropped_lease_costs_lease_discipline_a_goodbye_does_not() {
    let r = registry(Config::for_tests());
    let a = r.announce(&who("good"), announcement("good", &["a"], 5), Source::Lease).unwrap();
    let b = r.announce(&who("bad"), announcement("bad", &["a"], 5), Source::Lease).unwrap();
    r.deregister(Some("anon:good"), &a.lease_id).unwrap();
    r.sweep(Timestamp::now().checked_add(SignedDuration::from_secs(10)).unwrap());
    let s = r.reputation.compute(Timestamp::now());
    let good = s.providers[&a.provider_id].components["lease_discipline"].score;
    let bad = s.providers[&b.provider_id].components["lease_discipline"].score;
    assert!(good > 0.5 && bad < 0.5, "good {good} bad {bad}");
}
