//! A mock of pacod-id for the oidc scenarios: exactly the internal API the vendored
//! `pacod-id-client` calls, answered from the scenario's `identity`.
//!
//! A real pacod-id needs PostgreSQL and a configuration file; the suite needs principals with
//! roles in organisations, and to stop pacod-id mid-scenario. The shapes here come from the
//! client's own parsing (vendor/pacod-id/crates/pacod-id-client/src: `cache.rs`,
//! `introspect.rs`, `verifier.rs`, `client.rs`), and tests/idmock_contract.rs runs the vendored
//! client against this mock, so a drift fails a test.
//!
//! | Endpoint | What |
//! |---|---|
//! | `POST /oidc/token` | Client credentials: any client may call the internal API |
//! | `GET /oidc/jwks` | The mock's ES256 key |
//! | `PUT /internal/v1/catalog/{product}` | Takes the manifest after checking it with pacod-id-authz |
//! | `GET /internal/v1/authz/snapshot` | Role bindings of the scenario's principals |
//! | `GET /internal/v1/authz/changes` | No changes (bindings are fixed for a scenario) |
//! | `POST /internal/v1/introspect` | The scenario's service-account tokens (`pidt_…`) |

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::time::Duration;

use axum::Router;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use pacod_id_jose::{JwkSet, SigningKey};
use parking_lot::Mutex;
use serde_json::{Value, json};
use tokio::task::JoinHandle;

use crate::scenario::IdentitySetup;

/// A principal of the mock.
#[derive(Debug, Clone)]
pub struct MockPrincipal {
    pub name: String,
    pub sub: String,
    pub kind: String,
    pub organisation: Option<String>,
    /// (qualified role, path) pairs.
    pub roles: Vec<(String, Vec<String>)>,
}

struct MockState {
    issuer: String,
    key: SigningKey,
    principals: Mutex<HashMap<String, MockPrincipal>>,
    /// Service token → principal name.
    tokens: Mutex<HashMap<String, String>>,
    seq: AtomicI64,
    down: AtomicBool,
    catalog: Mutex<Option<Value>>,
}

pub struct IdMock {
    pub url: String,
    state: Arc<MockState>,
    task: JoinHandle<()>,
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

fn unavailable() -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        axum::Json(json!({ "error": { "type": "unavailable", "message": "the mock is down" } })),
    )
        .into_response()
}

impl IdMock {
    /// Starts the mock on a free local port with the scenario's organisations and principals.
    /// A principal named `suite-admin` with `broker.admin` at the platform is always added, for
    /// the runner's own admin calls.
    pub async fn start(identity: &IdentitySetup) -> anyhow::Result<Self> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let url = format!("http://{}", listener.local_addr()?);
        let (key, _) = SigningKey::generate_es256("suite-1").map_err(|e| anyhow::anyhow!("{e}"))?;
        let mut principals = HashMap::new();
        let mut all = identity.principals.clone();
        all.push(crate::scenario::PrincipalSetup {
            name: "suite-admin".into(),
            kind: "user".into(),
            organisation: None,
            roles: vec!["broker.admin@platform".into()],
        });
        for p in all {
            let mut roles = Vec::new();
            for r in &p.roles {
                let (role, at) = r.split_once('@').unwrap_or((r.as_str(), "platform"));
                let path = match at {
                    "platform" => vec!["platform".to_string()],
                    "organisation" => {
                        let org = p
                            .organisation
                            .clone()
                            .ok_or_else(|| anyhow::anyhow!("{}: {r} needs an organisation", p.name))?;
                        vec!["platform".to_string(), format!("organisation:{org}")]
                    }
                    other => anyhow::bail!("{}: {r}: @platform or @organisation, not @{other}", p.name),
                };
                roles.push((role.to_string(), path));
            }
            let prefix = if p.kind == "service" { "svc" } else { "usr" };
            principals.insert(
                p.name.clone(),
                MockPrincipal {
                    sub: format!("{prefix}_{}", p.name),
                    name: p.name,
                    kind: p.kind,
                    organisation: p.organisation,
                    roles,
                },
            );
        }
        let state = Arc::new(MockState {
            issuer: url.clone(),
            key,
            principals: Mutex::new(principals),
            tokens: Mutex::new(HashMap::new()),
            seq: AtomicI64::new(1),
            down: AtomicBool::new(false),
            catalog: Mutex::new(None),
        });
        let app = Router::new()
            .route("/oidc/token", post(token))
            .route("/oidc/jwks", get(jwks))
            .route("/internal/v1/catalog/{product}", put(catalog))
            .route("/internal/v1/authz/snapshot", get(snapshot))
            .route("/internal/v1/authz/changes", get(changes))
            .route("/internal/v1/introspect", post(introspect))
            .with_state(state.clone());
        let task = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        Ok(Self { url, state, task })
    }

    pub fn issuer(&self) -> &str {
        &self.state.issuer
    }

    pub fn principal(&self, name: &str) -> Option<MockPrincipal> {
        self.state.principals.lock().get(name).cloned()
    }

    /// A bearer for the principal: a signed access token for a person, a `pidt_…` token for a
    /// service account.
    pub fn token(&self, name: &str) -> anyhow::Result<String> {
        let p =
            self.principal(name).ok_or_else(|| anyhow::anyhow!("no principal {name:?} in the scenario's identity"))?;
        if p.kind == "service" {
            let mut tokens = self.state.tokens.lock();
            if let Some((t, _)) = tokens.iter().find(|(_, n)| **n == name) {
                return Ok(t.clone());
            }
            let t = format!("pidt_{}_{:016x}", name.replace(|c: char| !c.is_ascii_alphanumeric(), ""), rand_u64());
            tokens.insert(t.clone(), name.to_string());
            return Ok(t);
        }
        let n = now();
        let claims = json!({
            "iss": self.state.issuer, "aud": "pacod-broker", "sub": p.sub, "pid_kind": "user",
            "sid": format!("sid_{}", p.name), "client_id": "pacod-broker-ui", "scope": "openid",
            "iat": n, "exp": n + 3600, "name": p.name,
        });
        self.state.key.sign("at+jwt", &claims).map_err(|e| anyhow::anyhow!("{e}"))
    }

    /// A new service token for the principal, never introspected before (the broker's cache
    /// cannot answer for it).
    pub fn fresh_token(&self, name: &str) -> anyhow::Result<String> {
        let t = format!("pidt_{}_{:016x}", name.replace(|c: char| !c.is_ascii_alphanumeric(), ""), rand_u64());
        self.state.tokens.lock().insert(t.clone(), name.to_string());
        Ok(t)
    }

    /// Every endpoint answers 503 until `up`.
    pub fn down(&self) {
        self.state.down.store(true, Ordering::Relaxed);
    }

    pub fn up(&self) {
        self.state.down.store(false, Ordering::Relaxed);
    }

    /// Whether the broker pushed its manifest.
    pub fn catalog(&self) -> Option<Value> {
        self.state.catalog.lock().clone()
    }
}

impl Drop for IdMock {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn rand_u64() -> u64 {
    let mut b = [0u8; 8];
    // The tokens need only be unguessable within one test run.
    let _ = getrandom_fill(&mut b);
    u64::from_le_bytes(b)
}

fn getrandom_fill(b: &mut [u8]) -> std::io::Result<()> {
    use std::io::Read;
    std::fs::File::open("/dev/urandom")?.read_exact(b)
}

async fn token(State(s): State<Arc<MockState>>) -> Response {
    if s.down.load(Ordering::Relaxed) {
        return unavailable();
    }
    axum::Json(
        json!({ "access_token": format!("svc-internal-{:x}", rand_u64()), "token_type": "Bearer", "expires_in": 300 }),
    )
    .into_response()
}

async fn jwks(State(s): State<Arc<MockState>>) -> Response {
    if s.down.load(Ordering::Relaxed) {
        return unavailable();
    }
    axum::Json(JwkSet { keys: vec![s.key.public_jwk()] }).into_response()
}

async fn catalog(
    State(s): State<Arc<MockState>>,
    Path(product): Path<String>,
    axum::Json(body): axum::Json<Value>,
) -> Response {
    if s.down.load(Ordering::Relaxed) {
        return unavailable();
    }
    // Checked as pacod-id checks it, so a broken authz/broker.yaml fails the suite too.
    let checked = serde_json::from_value::<pacod_id_authz::ManifestSource>(body["manifest"].clone())
        .map_err(|e| e.to_string())
        .and_then(|src| pacod_id_authz::Manifest::from_source(src).map_err(|e| e.to_string()));
    match checked {
        Ok(m) if m.product() == product => {
            *s.catalog.lock() = Some(body["manifest"].clone());
            let seq = s.seq.fetch_add(1, Ordering::Relaxed) + 1;
            axum::Json(json!({ "version": 1, "changed": true, "seq": seq })).into_response()
        }
        Ok(m) => (
            StatusCode::BAD_REQUEST,
            axum::Json(json!({ "error": { "type": "product_mismatch", "message": m.product() } })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            axum::Json(json!({ "error": { "type": "manifest_invalid", "message": e } })),
        )
            .into_response(),
    }
}

async fn snapshot(State(s): State<Arc<MockState>>) -> Response {
    if s.down.load(Ordering::Relaxed) {
        return unavailable();
    }
    let mut bindings = Vec::new();
    for p in s.principals.lock().values() {
        for (i, (role, path)) in p.roles.iter().enumerate() {
            bindings
                .push(json!({ "id": format!("bnd_{}_{i}", p.name), "principal": p.sub, "role": role, "path": path }));
        }
    }
    axum::Json(json!({
        "seq": s.seq.load(Ordering::Relaxed),
        "catalog": { "version": 1 },
        "bindings": bindings,
        "disabled": [], "deleted": [], "revoked_sids": [], "scopes": [],
    }))
    .into_response()
}

async fn changes(State(s): State<Arc<MockState>>, Query(q): Query<HashMap<String, String>>) -> Response {
    if s.down.load(Ordering::Relaxed) {
        return unavailable();
    }
    // Bindings do not change during a scenario: wait a little, as the long poll would, and say so.
    let wait = q.get("wait").and_then(|w| w.parse::<u64>().ok()).unwrap_or(1).min(1);
    tokio::time::sleep(Duration::from_secs(wait)).await;
    axum::Json(json!({ "seq": s.seq.load(Ordering::Relaxed), "events": [] })).into_response()
}

async fn introspect(State(s): State<Arc<MockState>>, body: Bytes) -> Response {
    if s.down.load(Ordering::Relaxed) {
        return unavailable();
    }
    let body = String::from_utf8_lossy(&body);
    let token = body.split('&').find_map(|kv| kv.strip_prefix("token=")).unwrap_or_default();
    let token = percent_decode(token);
    let name = s.tokens.lock().get(&token).cloned();
    let Some(p) = name.and_then(|n| s.principals.lock().get(&n).cloned()) else {
        return axum::Json(json!({ "active": false })).into_response();
    };
    axum::Json(json!({
        "active": true, "pid_kind": "service", "sub": p.sub, "token_id": format!("tok_{}", p.name),
        "name": p.name, "organisation_id": p.organisation, "exp": now() + 3600,
    }))
    .into_response()
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'%' if i + 2 < b.len() => {
                let hex = std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok());
                match hex {
                    Some(v) => {
                        out.push(v);
                        i += 3;
                    }
                    None => {
                        out.push(b'%');
                        i += 1;
                    }
                }
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}
