//! Interaction tickets (docs/reputation.md §6).
//!
//! Every discovery result for an identified client carries a ticket: proof, signed by the
//! broker, that this client was shown this resource at this time. A usage report or a rating
//! counts only with a valid ticket for the same client and resource, so nobody can rate a
//! resource they were never offered, and a provider reporting about a client quotes the ticket
//! the client presented. Tickets are stateless: an HMAC over their content, checked without a
//! lookup.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use jiff::{SignedDuration, Timestamp};
use ring::hmac;
use serde::{Deserialize, Serialize};

const PREFIX: &str = "tkt_";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Ticket {
    /// The client.
    pub c: String,
    /// The resource.
    pub r: String,
    /// Issued, milliseconds since the epoch.
    pub t: i64,
    /// The client's organisation when it was issued, for the same-organisation rule when a
    /// witness reports on the ticket. Tickets from before it existed have none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub o: Option<String>,
}

pub struct Tickets {
    key: hmac::Key,
    ttl: SignedDuration,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum TicketError {
    #[error("not a ticket")]
    Malformed,
    #[error("the ticket's signature does not match")]
    Signature,
    #[error("the ticket has expired")]
    Expired,
    #[error("the ticket was issued to another client")]
    WrongClient,
    #[error("the ticket is for another resource")]
    WrongResource,
}

impl Tickets {
    pub fn new(secret: &[u8], ttl: std::time::Duration) -> Self {
        Self {
            key: hmac::Key::new(hmac::HMAC_SHA256, secret),
            ttl: SignedDuration::try_from(ttl).unwrap_or(SignedDuration::from_hours(24)),
        }
    }

    pub fn issue(&self, client: &str, organisation: Option<&str>, resource: &str, now: Timestamp) -> String {
        let t =
            Ticket { c: client.into(), r: resource.into(), t: now.as_millisecond(), o: organisation.map(String::from) };
        let body = serde_json::to_vec(&t).unwrap_or_default();
        let tag = hmac::sign(&self.key, &body);
        format!("{PREFIX}{}.{}", B64.encode(&body), B64.encode(tag.as_ref()))
    }

    /// Checks a ticket's signature and age, and returns what it says.
    pub fn open(&self, ticket: &str, now: Timestamp) -> Result<Ticket, TicketError> {
        let rest = ticket.strip_prefix(PREFIX).ok_or(TicketError::Malformed)?;
        let (body, tag) = rest.split_once('.').ok_or(TicketError::Malformed)?;
        let body = B64.decode(body).map_err(|_| TicketError::Malformed)?;
        let tag = B64.decode(tag).map_err(|_| TicketError::Malformed)?;
        hmac::verify(&self.key, &body, &tag).map_err(|_| TicketError::Signature)?;
        let t: Ticket = serde_json::from_slice(&body).map_err(|_| TicketError::Malformed)?;
        let issued = Timestamp::from_millisecond(t.t).map_err(|_| TicketError::Malformed)?;
        if issued > now.checked_add(SignedDuration::from_mins(5)).unwrap_or(now)
            || now.duration_since(issued) > self.ttl
        {
            return Err(TicketError::Expired);
        }
        Ok(t)
    }

    /// Checks that a ticket was issued to `client` for `resource`.
    pub fn check(
        &self,
        ticket: &str,
        client: &str,
        resource: Option<&str>,
        now: Timestamp,
    ) -> Result<Ticket, TicketError> {
        let t = self.open(ticket, now)?;
        if t.c != client {
            return Err(TicketError::WrongClient);
        }
        if resource.is_some_and(|r| r != t.r) {
            return Err(TicketError::WrongResource);
        }
        Ok(t)
    }
}

/// A new random key, for a broker without a configured or kept one.
pub fn random_key() -> Vec<u8> {
    let mut k = vec![0u8; 32];
    ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut k).expect("the system's random source");
    k
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tickets_round_trip_and_refuse_tampering() {
        let t = Tickets::new(b"0123456789abcdef0123456789abcdef", std::time::Duration::from_secs(3600));
        let now = Timestamp::now();
        let tk = t.issue("c1", None, "res_1", now);
        assert_eq!(t.check(&tk, "c1", Some("res_1"), now).unwrap().r, "res_1");
        assert_eq!(t.check(&tk, "c2", Some("res_1"), now), Err(TicketError::WrongClient));
        assert_eq!(t.check(&tk, "c1", Some("res_2"), now), Err(TicketError::WrongResource));
        let later = now.checked_add(SignedDuration::from_hours(2)).unwrap();
        assert_eq!(t.open(&tk, later), Err(TicketError::Expired));
        let other = Tickets::new(b"another key, also thirty-two bytes", std::time::Duration::from_secs(3600));
        assert_eq!(other.open(&tk, now), Err(TicketError::Signature));
        assert_eq!(t.open("tkt_nonsense", now), Err(TicketError::Malformed));
    }
}
