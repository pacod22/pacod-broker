//! pacod-id, for `BROKER_AUTH=oidc` (docs/concept.md §5).
//!
//! Built on pacod-id's own client library (vendor/pacod-id/crates/pacod-id-client), as pacod-gw
//! does it: the broker pushes its manifest (authz/broker.yaml) at start, follows pacod-id's
//! change feed for role bindings, checks people's access tokens locally against the keys and
//! service accounts' `pidt_…` tokens by introspection (cached, dropped when the feed says the
//! token was revoked).
//!
//! Discovery never waits for pacod-id: until the copy of the bindings is ready, callers are
//! treated as anonymous, so they see `public` resources if the broker allows that and nothing
//! else; the admin API answers 503 `identity_unavailable`.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use pacod_id_authz::{Catalog, Manifest, ScopeRef, Subject};
use pacod_id_client::{AuthError as IdAuthError, Authenticator, AuthzCache, Caller as IdCaller, IdClient, IdConfig};
use tokio::sync::watch;

use crate::auth::{AuthError, Principal, PrincipalKind};
use crate::config::Config;

/// The broker's manifest, as built into the binary.
pub const MANIFEST: &str = include_str!("../../../authz/broker.yaml");

/// The permissions of the manifest, local names.
pub const PERMISSIONS: &[&str] = &[
    "resources.discover",
    "resources.announce",
    "reports.usage",
    "reports.feedback",
    "reports.client",
    "reports.witness",
    "broker.read",
    "broker.manage",
    "reputation.moderate",
    "members.manage",
];

pub struct Identity {
    auth: Authenticator,
    cache: Arc<AuthzCache>,
    catalog: Catalog,
    client: IdClient,
    _stop: watch::Sender<bool>,
}

impl Identity {
    pub fn start(cfg: &Config) -> anyhow::Result<Arc<Self>> {
        let s =
            cfg.id_settings.as_ref().ok_or_else(|| anyhow::anyhow!("BROKER_AUTH=oidc without pacod-id settings"))?;
        let manifest = Manifest::from_yaml(MANIFEST).map_err(|e| anyhow::anyhow!("authz/broker.yaml: {e}"))?;
        let client = IdClient::new(IdConfig {
            issuers: s.issuers.clone(),
            internal_url: s.internal_url.trim_end_matches('/').to_string(),
            client_id: s.client_id.clone(),
            client_secret: s.client_secret.expose().to_string(),
            product: "broker".into(),
            audience: s.audience.clone(),
        });
        let (stop, rx) = watch::channel(false);
        let cache = AuthzCache::spawn(client.clone(), rx.clone());
        let auth = Authenticator::new(client.clone(), cache.clone());
        tokio::spawn(push_manifest(client.clone(), manifest.source().clone(), rx));
        Ok(Arc::new(Self { auth, cache, catalog: Catalog::new().with(manifest), client, _stop: stop }))
    }

    /// Whether the copy of the bindings is current enough to decide on access.
    pub fn ready(&self) -> bool {
        self.cache.is_ready() && self.cache.feed_age().is_some_and(|a| a <= Duration::from_secs(120))
    }

    pub fn client(&self) -> &IdClient {
        &self.client
    }

    /// Who the token belongs to and what they may do.
    pub async fn authenticate(&self, token: &str) -> Result<Principal, AuthError> {
        if !self.cache.is_ready() {
            return Err(AuthError::Unavailable);
        }
        let caller = self.auth.authenticate(token).await.map_err(|e| match e {
            IdAuthError::Unavailable(_) => AuthError::Unavailable,
            _ => AuthError::Invalid,
        })?;
        let (sub, kind, name, organisation_id, subject) = match &caller {
            IdCaller::User { sub, name, email, bindings, .. } => {
                // A person's organisation, for `organisation:` visibility, is the first one they
                // hold a role in.
                let org = bindings.iter().find_map(|b| org_of(&b.path));
                (sub.clone(), PrincipalKind::User, name.clone().or(email.clone()), org, Subject::user(bindings.clone()))
            }
            IdCaller::Service { sub, name, organisation_id, bindings, .. } => (
                sub.clone(),
                PrincipalKind::Service,
                (!name.is_empty()).then(|| name.clone()),
                organisation_id.clone().or_else(|| bindings.iter().find_map(|b| org_of(&b.path))),
                Subject::service(bindings.clone()),
            ),
        };
        let platform = [ScopeRef::platform()];
        let in_org = organisation_id.as_ref().map(|o| [ScopeRef::platform(), ScopeRef::organisation(o.clone())]);
        let permissions = PERMISSIONS
            .iter()
            .filter(|p| {
                let q = format!("broker.{p}");
                self.catalog.allowed(&subject, &q, &platform)
                    || in_org.as_ref().is_some_and(|path| self.catalog.allowed(&subject, &q, path))
            })
            .map(|p| p.to_string())
            .collect();
        Ok(Principal { sub, kind, name, organisation_id, verified: false, permissions })
    }
}

fn org_of(path: &[ScopeRef]) -> Option<String> {
    path.iter().find(|s| s.kind == "organisation").and_then(|s| s.id.clone())
}

async fn push_manifest(client: IdClient, source: pacod_id_authz::ManifestSource, mut stop: watch::Receiver<bool>) {
    let mut backoff = Duration::from_secs(1);
    loop {
        match client.push_catalog(&source, &HashMap::new()).await {
            Ok(pushed) => {
                tracing::info!(
                    version = pushed.version,
                    changed = pushed.changed,
                    "the broker's manifest is at pacod-id"
                );
                return;
            }
            Err(e) => tracing::warn!(error = %e, "pushing the broker's manifest to pacod-id failed; trying again"),
        }
        tokio::select! {
            _ = tokio::time::sleep(backoff) => {}
            _ = stop.wait_for(|s| *s) => return,
        }
        backoff = (backoff * 2).min(Duration::from_secs(30));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pacod_id_authz::Binding;

    #[test]
    fn the_manifest_reads_and_grants_what_it_says() {
        let catalog = Catalog::new().with(Manifest::from_yaml(MANIFEST).unwrap());
        let platform = [ScopeRef::platform()];
        let org = [ScopeRef::platform(), ScopeRef::organisation("o1")];
        // Any person may discover and rate (the implicit role), not announce.
        let person = Subject::user(vec![]);
        assert!(catalog.allowed(&person, "broker.resources.discover", &platform));
        assert!(catalog.allowed(&person, "broker.reports.feedback", &platform));
        assert!(!catalog.allowed(&person, "broker.resources.announce", &platform));
        // A service account with no role may do nothing.
        assert!(!catalog.allowed(&Subject::service(vec![]), "broker.resources.discover", &platform));
        // A provider in an organisation announces there.
        let provider = Subject::service(vec![Binding::new("broker.provider", org.to_vec())]);
        assert!(catalog.allowed(&provider, "broker.resources.announce", &org));
        assert!(!catalog.allowed(&provider, "broker.broker.manage", &platform));
        // The admin may do everything.
        let admin = Subject::user(vec![Binding::new("broker.admin", platform.to_vec())]);
        for p in PERMISSIONS {
            assert!(catalog.allowed(&admin, &format!("broker.{p}"), &platform), "{p}");
        }
    }
}
