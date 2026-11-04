//! The Rust client against a real broker on a local port: a provider announces and keeps its
//! lease, a consumer discovers, watches and reports; the broker forgets the lease and the
//! provider announces again by itself; the provider says goodbye.

use std::time::Duration;

use futures_util::StreamExt;
use pacod_broker::app::App;
use pacod_broker::config;
use pacod_broker::env::Vars;
use pacod_broker::server::Server;
use pacod_broker_client::{
    Announcement, BrokerClient, ClientReportKind, Outcome, Provider, ProviderEvent, Query, UsageReport, WatchEvent,
};
use pacod_broker_model::LiveStatus;

async fn broker() -> Server {
    let cfg = config::read(&Vars::from_pairs([
        ("BROKER_LISTEN", "127.0.0.1:0"),
        ("BROKER_METRICS_LISTEN", "off"),
        ("BROKER_REBUILD_MS", "0"),
        ("BROKER_LEASE_TTL_MIN", "3"),
    ]))
    .unwrap();
    Server::start(App::build(cfg, None).unwrap()).await.unwrap()
}

const ANNOUNCEMENT: &str = r#"
ttl_s: 3
provider:
  name: rust-client-test
  summary: A provider in the client's test
  inventory:
    visibility: public
    hardware: [{ id: g, type: gpu, model: L40S, count: 2, memory_gb: 48 }]
resources:
  - key: coder
    kind: model
    name: test-coder-7b
    summary: A small coding model
    interfaces: [{ protocol: openai-chat, url: "https://test.example/v1" }]
    runs_on: { hardware: [g] }
    capabilities: { tools: true }
    pricing: { currency: EUR, periods: [{ valid_from: "2026-01-01T00:00:00Z", rates: { input: 0.1 } }] }
    availability: { latency: { ttft_ms: 150 } }
    access: { visibility: public }
"#;

async fn eventually<F, Fut>(what: &str, mut f: F)
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    for _ in 0..100 {
        if f().await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("{what} did not happen");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_provider_and_a_consumer_through_the_rust_client() {
    let server = broker().await;
    let base = format!("http://{}", server.public_addr);
    let consumer = BrokerClient::new(&base).unwrap().with_client_name("rust-consumer");
    assert!(consumer.ready().await);

    // The consumer watches from the start.
    let mut watch = Box::pin(consumer.watch(None).await.unwrap());
    assert!(matches!(watch.next().await, Some(Ok(WatchEvent::Hello { .. }))));

    // The provider checks, announces and keeps its lease.
    let announcement = Announcement::parse(ANNOUNCEMENT).unwrap();
    announcement.validate(16 * 1024).unwrap();
    let provider = Provider::new(BrokerClient::new(&base).unwrap(), announcement);
    let live = provider.live();
    live.set_in_use(1);
    let events = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen = events.clone();
    let running = provider.start_with(move |e| seen.lock().unwrap().push(e)).await.unwrap();
    let first = running.announced().await;
    let rid = first.resources["coder"].clone();

    // The watch stream says so.
    let upserted = tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(e) = watch.next().await {
            if let Ok(WatchEvent::Upserted { resource_id, .. }) = e {
                return resource_id;
            }
        }
        String::new()
    })
    .await
    .unwrap();
    assert_eq!(upserted, rid);

    // The consumer finds it, typed, with a ticket.
    let q = Query {
        kind: Some("model".into()),
        capabilities: vec!["tools".into()],
        currency: Some("EUR".into()),
        ..Query::default()
    };
    let mut found = None;
    for _ in 0..100 {
        let r = consumer.discover(&q).await.unwrap();
        if r.total == 1 {
            found = r.hits.into_iter().next();
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let hit = found.unwrap();
    assert_eq!(hit.interface(Some("openai-chat")).unwrap().url, "https://test.example/v1");
    let ticket = hit.ticket.clone().unwrap();

    // Heartbeats carry the live state.
    live.set_resource("coder", Some(LiveStatus::Degraded), Some(2), None);
    eventually("live state", || {
        let consumer = consumer.clone();
        let rid = rid.clone();
        async move { consumer.resource(&rid).await.unwrap().live.in_use == Some(2) }
    })
    .await;
    let view = consumer.resource(&rid).await.unwrap();
    assert_eq!((view.record.spec.name.as_str(), view.live.status), ("test-coder-7b", LiveStatus::Degraded));
    let pv = consumer.provider(&first.provider_id).await.unwrap();
    assert_eq!(pv.profile.inventory.unwrap().hardware[0].model.as_deref(), Some("L40S"));

    // Reports with the ticket count; a rating out of range is refused before it is sent.
    let acc = consumer
        .report_usage(&[UsageReport::new(&ticket, Outcome::Ok).latency(140.0, Some(12.0)).charged("input", 0.1)])
        .await
        .unwrap();
    assert_eq!(acc.accepted, 1);
    assert_eq!(consumer.rate(&ticket, 5, Some("quick")).await.unwrap().accepted, 1);
    assert!(consumer.rate(&ticket, 9, None).await.is_err());
    let someone_else = BrokerClient::new(&base).unwrap().with_client_name("someone-else");
    assert_eq!(someone_else.rate(&ticket, 1, None).await.unwrap_err().status(), Some(422));

    // The provider side of tickets.
    let providers_view = BrokerClient::new(&base).unwrap();
    assert_eq!(providers_view.verify_ticket(&ticket).await.unwrap().client, "anon:rust-consumer");
    assert!(providers_view.report_client(&ticket, ClientReportKind::GoodStanding, None).await.unwrap().counted);
    assert_eq!(consumer.me().await.unwrap().client, "anon:rust-consumer");

    // The broker forgets the lease; the provider announces again on its own.
    server.app.registry.end_lease(&first.lease_id, false, "test");
    eventually("re-announcement", || {
        let events = events.clone();
        async move { events.lock().unwrap().iter().any(|e| matches!(e, ProviderEvent::Reannounced(_))) }
    })
    .await;
    let again = running.announced().await;
    assert_ne!(again.lease_id, first.lease_id);
    assert_eq!(again.resources["coder"], rid, "ids are derived, so they survive");

    // The price feed, in pacod-gw's format.
    let feed = consumer.price_feed(&[("provider", "rust-client-test")]).await.unwrap();
    assert_eq!(feed["currency"], "EUR");
    assert_eq!(feed["models"]["test-coder-7b"]["input"], 0.1);

    // An update under the same lease: a second resource, a new price; ids stay.
    let mut changed = Announcement::parse(ANNOUNCEMENT).unwrap();
    let mut second = changed.resources[0].clone();
    second.key = "chat".into();
    second.name = "test-chat-7b".into();
    changed.resources.push(second);
    let updated = running.update(changed).await.unwrap();
    assert_eq!(updated.lease_id, again.lease_id, "the same lease");
    assert_eq!(updated.resources["coder"], rid);
    assert!(updated.resources.contains_key("chat"));
    eventually("the second resource", || {
        let consumer = consumer.clone();
        let q = q.clone();
        async move { consumer.discover(&q).await.unwrap().total == 2 }
    })
    .await;

    // Goodbye: the resources go.
    running.stop().await.unwrap();
    eventually("removal", || {
        let consumer = consumer.clone();
        let q = q.clone();
        async move { consumer.discover(&q).await.unwrap().total == 0 }
    })
    .await;
    server.shutdown().await;
}

#[tokio::test]
async fn problems_come_back_typed() {
    let server = broker().await;
    let c = BrokerClient::new(format!("http://{}", server.public_addr)).unwrap();
    let bad = Query { sort: "reputation".parse().unwrap(), ..Query::default() };
    let e = c.discover(&bad).await.unwrap_err();
    assert_eq!(e.status(), Some(422));
    assert!(c.resource("res_nope").await.unwrap_err().is_not_found());
    let mut a = Announcement::parse(ANNOUNCEMENT).unwrap();
    a.resources[0].kind = "nonsense".into();
    assert!(a.validate(16 * 1024).is_err());
    match c.announce(&a).await.unwrap_err() {
        pacod_broker_client::Error::Problem(p) => assert!(p.errors[0].contains("resources[0].kind"), "{p}"),
        e => panic!("{e}"),
    }
    server.shutdown().await;
}
