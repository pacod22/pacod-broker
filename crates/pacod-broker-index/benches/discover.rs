//! Discovery at 10 000 resources: the broker's latency target is a p99 under 1 ms in process
//! (docs/concept.md §1). `cargo bench -p pacod-broker-index`.

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use jiff::Timestamp;
use pacod_broker_index::fixtures::{entry, provider};
use pacod_broker_index::{Ctx, Index, NoScores, Query, Viewer};
use serde_json::json;

fn catalogue(n: usize) -> Index {
    let providers: Vec<_> = (0..100)
        .map(|p| {
            provider(&format!("p{p}"), json!({"name": "x", "summary": "provider", "inventory": {"visibility": "public",
                "hardware": [{"id": "g", "type": "gpu", "model": if p % 3 == 0 { "H100-SXM" } else { "L40S" }, "count": 8, "memory_gb": 80}],
                "software": [{"id": "v", "kind": "serving_engine", "name": "vllm", "version": format!("0.{}.0", p % 9)}]}}))
        })
        .collect();
    let kinds = ["model", "model", "model", "mcp_server", "compute", "embedding"];
    let entries = (0..n)
        .map(|i| {
            let p = &providers[i % providers.len()];
            entry(p, json!({
                "key": format!("r{i}"), "kind": kinds[i % kinds.len()], "name": format!("model-{}", i % 500),
                "summary": format!("Resource {i} serving family {} in region r{}", i % 17, i % 7),
                "description": "A longer description with words like streaming, tools, reasoning and vision that people search for.",
                "tags": [format!("t{}", i % 20), "llm"],
                "interfaces": [{"protocol": if i % 2 == 0 { "openai-chat" } else { "anthropic-messages" }, "url": "https://x.example/v1"}],
                "model": {"family": format!("f{}", i % 17), "context_window": 8192 * (1 + i % 16)},
                "capabilities": {"tools": i % 2 == 0, "streaming": true, "vision": i % 5 == 0},
                "pricing": {"currency": "EUR", "timezone": "Europe/Sofia", "periods": [{"valid_from": "2026-01-01T00:00:00Z",
                    "rates": {"input": (i % 97) as f64 / 10.0, "output": (i % 89) as f64 / 5.0},
                    "windows": [{"name": "night", "from_time": "22:00", "to_time": "06:00", "rates": {"input": 0.1}}]}]},
                "availability": {"regions": [format!("r{}", i % 7)], "capacity": {"max_concurrency": 32},
                    "latency": {"ttft_ms": (i % 1000) as f64},
                    "schedule": {"timezone": "UTC", "rules": [{"from_time": "00:00", "to_time": "24:00"}],
                        "maintenance": [{"from": "2026-11-04T00:00:00Z", "to": "2026-11-04T01:00:00Z"}]}},
                "access": {"visibility": "public"}
            }))
        })
        .collect();
    Index::build(1, providers, entries)
}

fn bench(c: &mut Criterion) {
    let idx = catalogue(10_000);
    let viewer = Viewer::everything();
    let now: Timestamp = "2026-11-03T12:00:00Z".parse().unwrap();
    let ctx = Ctx { now, viewer: &viewer, scores: &NoScores };
    let cases: Vec<(&str, Query)> = vec![
        ("kind+price-sort", serde_json::from_value(json!({"kind": "model", "sort": "price:input", "max_price": {"input": 5.0}})).unwrap()),
        ("text", serde_json::from_value(json!({"q": "family streaming"})).unwrap()),
        ("caps+protocol", serde_json::from_value(json!({"capabilities": ["tools", "vision"], "protocols": ["openai-chat"]})).unwrap()),
        ("hardware+software", serde_json::from_value(json!({"hardware": {"gpu_model": "H100-SXM"}, "software": [{"name": "vllm", "version": ">=0.5"}]})).unwrap()),
        ("during-span", serde_json::from_value(json!({"kind": "model", "tags": ["t3"], "during": {"from": "2026-11-03T00:00:00Z", "to": "2026-11-05T00:00:00Z"}, "availability": {"coverage": "any"}, "sort": "price:input"})).unwrap()),
        ("everything-default", Query::default()),
    ];
    for (name, q) in cases {
        c.bench_function(&format!("discover/10k/{name}"), |b| {
            b.iter(|| black_box(idx.discover(black_box(&q), &ctx).unwrap()))
        });
    }
    c.bench_function("build/10k", |b| {
        let providers: Vec<_> = idx.providers().cloned().collect();
        let entries: Vec<_> = idx.entries().cloned().collect();
        b.iter(|| black_box(Index::build(2, providers.clone(), entries.clone())))
    });
}

criterion_group!(benches, bench);
criterion_main!(benches);
