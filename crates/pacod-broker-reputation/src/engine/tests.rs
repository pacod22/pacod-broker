use std::sync::Arc;

use jiff::{SignedDuration, Timestamp};

use super::*;

fn t0() -> Timestamp {
    "2026-11-03T12:00:00Z".parse().unwrap()
}

fn later(mins: i64) -> Timestamp {
    t0().checked_add(SignedDuration::from_mins(mins)).unwrap()
}

fn usage(resource: &str, client: &str, outcome: Outcome, ttft: f64, at: Timestamp) -> Evidence {
    Evidence::Usage {
        resource: resource.into(),
        provider: format!("p-{resource}"),
        kind: "model".into(),
        client: client.into(),
        client_org: None,
        provider_org: None,
        outcome,
        ttft_ms: Some(ttft),
        ms_per_token: Some(10.0),
        declared_ttft_ms: Some(300.0),
        price_charged: Some(1.0),
        price_listed: Some(1.0),
        at,
        witness: None,
        witness_org: None,
    }
}

#[test]
fn a_witness_counts_for_the_client_but_not_for_its_own_organisation() {
    let e = Engine::new(Settings::default(), None);
    let witnessed = |witness_org: &str| {
        let mut u = usage("r1", "c1", Outcome::Ok, 200.0, t0());
        if let Evidence::Usage { witness, witness_org: w, provider_org, .. } = &mut u {
            *witness = Some("gw".into());
            *w = Some(witness_org.into());
            *provider_org = Some("acme".into());
        }
        u
    };
    assert!(e.record(witnessed("platform")), "a witness outside the provider's organisation counts");
    assert!(!e.record(witnessed("acme")), "a gateway of the provider's own organisation does not");
}

#[test]
fn a_new_subject_sits_at_the_prior_with_no_confidence() {
    let e = Engine::new(Settings::default(), None);
    e.record(usage("r1", "c1", Outcome::Ok, 200.0, t0()));
    let s = e.compute(later(1));
    let r = &s.resources["r1"];
    assert!(r.score > 0.5 && r.score < 0.6, "{}", r.score);
    assert_eq!(r.label, Label::New);
    assert!(r.confidence < 0.3);
}

#[test]
fn good_and_bad_resources_separate_with_evidence() {
    let e = Engine::new(Settings::default(), None);
    for i in 0..60 {
        let c = format!("c{}", i % 10);
        e.record(usage("good", &c, Outcome::Ok, 200.0, later(i)));
        e.record(usage("bad", &c, if i % 3 == 0 { Outcome::Ok } else { Outcome::ServerError }, 900.0, later(i)));
        e.record(Evidence::Uptime {
            resource: "good".into(),
            provider: "p-good".into(),
            up: 3600.0,
            expected: 3600.0,
            at: later(i),
        });
        e.record(Evidence::Uptime {
            resource: "bad".into(),
            provider: "p-bad".into(),
            up: 1800.0,
            expected: 3600.0,
            at: later(i),
        });
    }
    let s = e.compute(later(61));
    let (g, b) = (&s.resources["good"], &s.resources["bad"]);
    assert!(g.score > b.score + 0.2, "good {} bad {}", g.score, b.score);
    assert!(g.confidence > 0.9);
    // Performance is a percentile within the kind: the faster one is first.
    assert!(g.components["performance"].score > b.components["performance"].score);
    // The bad one is slower than it claims (900 ms against 300 ms declared).
    assert!(b.components["accuracy"].score < g.components["accuracy"].score);
    assert!(s.providers["p-good"].score > s.providers["p-bad"].score);
}

#[test]
fn reports_from_the_providers_own_organisation_do_not_count() {
    let e = Engine::new(Settings::default(), None);
    let ev = Evidence::Feedback {
        resource: "r".into(),
        provider: "p".into(),
        client: "c".into(),
        client_org: Some("org".into()),
        provider_org: Some("org".into()),
        rating: 5,
        at: t0(),
    };
    assert!(!e.record(ev));
}

#[test]
fn a_rater_counts_once_per_subject_per_hour() {
    let e = Engine::new(Settings::default(), None);
    let fb = |rating, at| Evidence::Feedback {
        resource: "r".into(),
        provider: "p".into(),
        client: "c".into(),
        client_org: None,
        provider_org: None,
        rating,
        at,
    };
    assert!(e.record(fb(1, t0())));
    assert!(!e.record(fb(1, later(5))));
    assert!(e.record(fb(1, later(65))));
}

#[test]
fn client_reports_can_be_taken_back() {
    let e = Engine::new(Settings::default(), None);
    e.record(Evidence::ClientIdentity { client: "c".into(), level: 0.7, at: t0() });
    let before = e.compute(later(1)).clients["c"].components["conduct"].score;
    e.record(Evidence::ClientReport {
        client: "c".into(),
        provider: "p".into(),
        report: ClientReportKind::Abuse,
        id: "rep1".into(),
        at: later(2),
    });
    let after = e.compute(later(3)).clients["c"].components["conduct"].score;
    assert!(after < before);
    assert!(e.record(Evidence::ClientReportRejected {
        client: "c".into(),
        report: ClientReportKind::Abuse,
        id: "rep1".into(),
        at: later(4)
    }));
    let restored = e.compute(later(5)).clients["c"].components["conduct"].score;
    assert!(restored > after);
    // A report can be taken back once.
    assert!(!e.record(Evidence::ClientReportRejected {
        client: "c".into(),
        report: ClientReportKind::Abuse,
        id: "rep1".into(),
        at: later(6)
    }));
}

#[test]
fn frozen_scores_do_not_move_and_resets_return_to_the_prior() {
    let e = Engine::new(Settings::default(), None);
    for i in 0..20 {
        e.record(usage("r", &format!("c{i}"), Outcome::Ok, 100.0, later(i)));
    }
    let before = e.compute(later(30)).resources["r"].score;
    e.record(Evidence::Freeze { side: Side::Resource, subject: "r".into(), frozen: true, at: later(31) });
    for i in 0..40 {
        e.record(usage("r", &format!("d{i}"), Outcome::ServerError, 100.0, later(32 + i)));
    }
    let s = e.compute(later(80));
    assert_eq!(s.resources["r"].score, before);
    assert!(s.resources["r"].frozen);
    e.record(Evidence::Reset { side: Side::Resource, subject: "r".into(), at: later(81) });
    assert!(!e.compute(later(82)).resources.contains_key("r"));
}

#[test]
fn the_log_replays_to_the_same_scores() {
    let sink = Arc::new(MemorySink::default());
    let e = Engine::new(Settings::default(), Some(sink.clone()));
    for i in 0..30 {
        e.record(usage(
            "r",
            &format!("c{}", i % 4),
            if i % 5 == 0 { Outcome::Timeout } else { Outcome::Ok },
            150.0 + i as f64,
            later(i),
        ));
    }
    e.record(Evidence::LeaseEnded { provider: "p-r".into(), resources: vec!["r".into()], clean: false, at: later(40) });
    let live = e.compute(later(50));

    let lines = sink.0.lock().clone();
    let replayed = Engine::new(Settings::default(), None);
    let (ok, bad) = replayed.restore(None, lines);
    assert_eq!((ok, bad), (31, 0));
    let again = replayed.compute(later(50));
    assert_eq!(again.resources["r"].score, live.resources["r"].score);
    assert_eq!(again.providers["p-r"].score, live.providers["p-r"].score);

    // A snapshot plus nothing replays to the same too.
    let from_snapshot = Engine::new(Settings::default(), None);
    from_snapshot.restore(Some(e.snapshot()), Vec::<Vec<u8>>::new());
    assert_eq!(from_snapshot.compute(later(50)).resources["r"].score, live.resources["r"].score);
}

#[test]
fn verified_providers_are_trusted_more_and_incidents_cost_trust() {
    let e = Engine::new(Settings::default(), None);
    e.record(Evidence::ProviderVerified { provider: "a".into(), verified: true, at: t0() });
    e.record(Evidence::ProviderVerified { provider: "b".into(), verified: false, at: t0() });
    let s = e.compute(later(1));
    assert!(s.providers["a"].components["trust"].score > s.providers["b"].components["trust"].score);
    e.record(Evidence::ProviderIncident { provider: "a".into(), at: later(2) });
    let s2 = e.compute(later(3));
    assert!(s2.providers["a"].components["trust"].score < s.providers["a"].components["trust"].score);
}
