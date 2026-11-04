//! The public and admin APIs end to end, in process, in open mode.

use std::sync::Arc;

use axum::body::Body;
use http::{Request, StatusCode};
use pacod_broker::app::App;
use pacod_broker::config::{self, Config};
use pacod_broker::env::Vars;
use serde_json::{Value, json};
use tower::ServiceExt;

fn app_with(vars: &[(&str, &str)]) -> Arc<App> {
    let mut all = vec![("BROKER_LISTEN", "127.0.0.1:0"), ("BROKER_METRICS_LISTEN", "off"), ("BROKER_REBUILD_MS", "0")];
    all.extend_from_slice(vars);
    let cfg: Config = config::read(&Vars::from_pairs(all)).unwrap();
    App::build(cfg, None).unwrap()
}

async fn call(
    router: &axum::Router,
    method: &str,
    uri: &str,
    headers: &[(&str, &str)],
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut b = Request::builder().method(method).uri(uri);
    for (k, v) in headers {
        b = b.header(*k, *v);
    }
    let req = match body {
        Some(v) => b.header("content-type", "application/json").body(Body::from(v.to_string())).unwrap(),
        None => b.body(Body::empty()).unwrap(),
    };
    let res = router.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let bytes = axum::body::to_bytes(res.into_body(), 1 << 24).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

fn gpu_provider() -> Value {
    json!({
        "ttl_s": 30,
        "provider": {
            "name": "sofia-gpu", "title": "Sofia GPU", "summary": "GPUs in Sofia",
            "description": "## Who we are\nA small GPU farm.",
            "inventory": {"visibility": "public",
                "hardware": [{"id": "h100", "type": "gpu", "vendor": "nvidia", "model": "H100-SXM", "count": 8, "memory_gb": 80}],
                "software": [{"id": "vllm", "kind": "serving_engine", "name": "vllm", "version": "0.7.3"}]}
        },
        "resources": [
            {"key": "llama", "kind": "model", "name": "llama-3.3-70b", "summary": "Llama 3.3 70B on H100s",
             "interfaces": [{"protocol": "openai-chat", "url": "https://gpu.example/v1", "auth": {"scheme": "bearer"}}],
             "runs_on": {"hardware": ["h100"], "software": ["vllm"]},
             "pricing": {"currency": "EUR", "timezone": "UTC", "periods": [{"valid_from": "2026-01-01T00:00:00Z", "rates": {"input": 0.8, "output": 1.2},
                 "windows": [{"name": "night", "from_time": "22:00", "to_time": "06:00", "rates": {"input": 0.3}}]}]},
             "availability": {"latency": {"ttft_ms": 300}, "capacity": {"max_concurrency": 16}},
             "access": {"visibility": "public"}},
            {"key": "gpu-hours", "kind": "compute", "name": "h100x8", "summary": "Eight H100s by the hour",
             "interfaces": [{"protocol": "http", "url": "https://gpu.example/book"}],
             "runs_on": {"hardware": ["h100"]},
             "pricing": {"currency": "EUR", "periods": [{"valid_from": "2026-01-01T00:00:00Z", "rates": {"per_gpu_hour": 2.1}}]},
             "availability": {"valid_from": "2030-01-01T00:00:00Z", "capacity": {"max_concurrency": 8, "unit": "gpus"},
                 "schedule": {"timezone": "UTC", "slots": [{"from": "2030-01-01T00:00:00Z", "to": "2030-01-02T00:00:00Z", "capacity": 8}]}},
             "access": {"visibility": "public"}}
        ]
    })
}

#[tokio::test]
async fn providers_announce_consumers_discover_and_report() {
    let app = app_with(&[]);
    let public = pacod_broker::server::public_router(app.clone());
    let admin = pacod_broker::server::admin_router(app.clone());

    let (s, a) = call(&public, "PUT", "/v1/announce", &[], Some(gpu_provider())).await;
    assert_eq!(s, StatusCode::OK, "{a}");
    let lease = a["lease_id"].as_str().unwrap().to_string();
    app.registry.rebuild();

    // Discovery, with a ticket because the client named itself.
    let me = [("x-broker-client", "agent-1")];
    let (s, d) = call(&public, "POST", "/v1/discover", &me, Some(json!({"kind": "model", "hardware": {"gpu_model": "H100-SXM"}, "software": [{"name": "vllm", "version": ">=0.7"}]}))).await;
    assert_eq!(s, StatusCode::OK, "{d}");
    assert_eq!(d["total"], 1);
    let hit = &d["hits"][0];
    assert_eq!(hit["provider"]["name"], "sofia-gpu");
    assert!(hit["why"]["matched"].as_array().unwrap().iter().any(|m| m == "hardware"));
    let ticket = hit["ticket"].as_str().unwrap().to_string();

    // Prices at night.
    let (_, d) = call(
        &public,
        "POST",
        "/v1/discover",
        &me,
        Some(json!({"kind": "model", "at": "2026-11-03T23:00:00Z", "sort": "price:input"})),
    )
    .await;
    assert_eq!(d["hits"][0]["price"]["rates"]["input"], 0.3);
    assert_eq!(d["hits"][0]["price"]["source"], "window:night");

    // GPU hours bookable in 2030, not now.
    let (_, d) = call(&public, "POST", "/v1/discover", &me, Some(json!({"kind": "compute"}))).await;
    assert_eq!(d["total"], 0);
    let (_, d) = call(&public, "POST", "/v1/discover", &me, Some(json!({"kind": "compute", "during": {"from": "2030-01-01T02:00:00Z", "to": "2030-01-01T10:00:00Z"}, "availability": {"min_capacity": 4}}))).await;
    assert_eq!(d["total"], 1, "{d}");

    // The simple form.
    let (s, d) = call(&public, "GET", "/v1/resources?q=llama&include_unavailable=true", &me, None).await;
    assert_eq!((s, d["total"].as_u64()), (StatusCode::OK, Some(1)));

    // A usage report and a rating with the ticket; without it, or with another client's, nothing counts.
    let (s, r) = call(
        &public,
        "POST",
        "/v1/reports/usage",
        &me,
        Some(json!({"reports": [
            {"ticket": ticket, "outcome": "ok", "ttft_ms": 280, "price_charged": 0.8, "rate": "input"},
            {"ticket": "tkt_forged.x", "outcome": "ok"}
        ]})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert_eq!(r["accepted"], 1);
    assert_eq!(r["refused"].as_array().unwrap().len(), 1);
    let (_, r) = call(
        &public,
        "POST",
        "/v1/reports/feedback",
        &[("x-broker-client", "agent-2")],
        Some(json!({"ticket": ticket, "rating": 5})),
    )
    .await;
    assert_eq!(r["status"], 422, "another client's ticket: {r}");
    let (s, r) = call(
        &public,
        "POST",
        "/v1/reports/feedback",
        &me,
        Some(json!({"ticket": ticket, "rating": 5, "comment": "fast"})),
    )
    .await;
    assert_eq!((s, r["accepted"].as_u64()), (StatusCode::OK, Some(1)), "{r}");

    // The provider checks the ticket and reports good standing.
    let (s, v) = call(&public, "POST", "/v1/tickets/verify", &[], Some(json!({"ticket": ticket}))).await;
    assert_eq!((s, v["client"].as_str()), (StatusCode::OK, Some("anon:agent-1")));
    let (s, _) =
        call(&public, "POST", "/v1/reports/client", &[], Some(json!({"ticket": ticket, "kind": "good_standing"})))
            .await;
    assert_eq!(s, StatusCode::OK);

    app.reputation.compute(jiff::Timestamp::now());
    let (_, me_rep) = call(&public, "GET", "/v1/reputation/me", &me, None).await;
    assert!(me_rep["as_client"]["score"].as_f64().unwrap() > 0.0, "{me_rep}");

    // MCP.
    let (_, m) = call(
        &public,
        "POST",
        "/mcp",
        &me,
        Some(json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}})),
    )
    .await;
    assert_eq!(m["result"]["serverInfo"]["name"], "pacod-broker");
    let (_, m) = call(&public, "POST", "/mcp", &me, Some(json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {"name": "discover_resources", "arguments": {"q": "llama"}}}))).await;
    assert_eq!(m["result"]["structuredContent"]["total"], 1, "{m}");
    let (s, _) =
        call(&public, "POST", "/mcp", &me, Some(json!({"jsonrpc": "2.0", "method": "notifications/initialized"})))
            .await;
    assert_eq!(s, StatusCode::ACCEPTED);

    // The provider, with its inventory.
    let pid = a["provider_id"].as_str().unwrap();
    let (_, p) = call(&public, "GET", &format!("/v1/providers/{pid}"), &me, None).await;
    assert_eq!(p["profile"]["inventory"]["hardware"][0]["model"], "H100-SXM");
    assert_eq!(p["resources"].as_array().unwrap().len(), 2);

    // The admin API sees the lease, the reports, the reputation.
    let (s, st) = call(&admin, "GET", "/api/v1/status", &[], None).await;
    assert_eq!((s, st["resources"].as_u64()), (StatusCode::OK, Some(2)));
    let (_, rows) = call(&admin, "GET", "/api/v1/reputation?side=resource", &[], None).await;
    assert!(!rows.as_array().unwrap().is_empty());
    let (_, reps) = call(&admin, "GET", "/api/v1/reports", &[], None).await;
    assert_eq!(reps["client_reports"].as_array().unwrap().len(), 1);

    // A clean goodbye.
    let (s, _) = call(&public, "DELETE", &format!("/v1/leases/{lease}"), &[], None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    app.registry.rebuild();
    let (_, d) = call(&public, "POST", "/v1/discover", &me, Some(json!({}))).await;
    assert_eq!(d["total"], 0);
}

#[tokio::test]
async fn problems_are_problem_json() {
    let app = app_with(&[]);
    let public = pacod_broker::server::public_router(app);
    let (s, p) = call(
        &public,
        "PUT",
        "/v1/announce",
        &[],
        Some(json!({"provider": {"name": "x"}, "resources": [{"key": "a", "kind": "nope", "name": "a"}]})),
    )
    .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(p["type"], "invalid");
    assert!(p["errors"][0].as_str().unwrap().contains("resources[0].kind"), "{p}");
    let (s, p) = call(&public, "POST", "/v1/discover", &[], Some(json!({"sort": "reputation"}))).await;
    assert_eq!((s, p["type"].as_str()), (StatusCode::UNPROCESSABLE_ENTITY, Some("invalid")));
    let (s, _) = call(&public, "POST", "/v1/discover", &[], Some(json!({"colour": "red"}))).await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn the_admin_token_guards_the_admin_api() {
    let app = app_with(&[("BROKER_ADMIN_TOKEN", "a-long-enough-admin-token-123")]);
    let admin = pacod_broker::server::admin_router(app);
    let (s, _) = call(&admin, "GET", "/api/v1/status", &[], None).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    let (s, _) =
        call(&admin, "GET", "/api/v1/status", &[("authorization", "Bearer a-long-enough-admin-token-123")], None).await;
    assert_eq!(s, StatusCode::OK);
}

#[tokio::test]
async fn the_state_directory_keeps_static_entries_and_reputation() {
    let dir = std::env::temp_dir().join(format!("broker-state-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let d = dir.to_str().unwrap().to_string();
    {
        let app = app_with(&[("BROKER_STATE_DIR", &d)]);
        let admin = pacod_broker::server::admin_router(app.clone());
        let mut p = gpu_provider();
        p.as_object_mut().unwrap().remove("ttl_s");
        let (s, r) = call(&admin, "PUT", "/api/v1/static", &[], Some(p)).await;
        assert_eq!(s, StatusCode::OK, "{r}");
        let rid = r["resources"]["llama"].as_str().unwrap().to_string();
        app.reputation.record(pacod_broker_reputation::Evidence::Uptime {
            resource: rid,
            provider: r["provider_id"].as_str().unwrap().into(),
            up: 3600.0,
            expected: 3600.0,
            at: jiff::Timestamp::now(),
        });
        app.persist();
        app.compact();
    }
    let app = app_with(&[("BROKER_STATE_DIR", &d)]);
    assert_eq!(app.registry.index().len(), 2);
    let scores = app.reputation.compute(jiff::Timestamp::now());
    assert_eq!(scores.resources.len(), 1);
    let _ = std::fs::remove_dir_all(&dir);
}
