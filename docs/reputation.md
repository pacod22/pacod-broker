# pacod-broker: reputation

| Field | Value |
|---|---|
| Status | Built in the first release; slot booking, canaries and attestation come later (section 8) |
| Code | `crates/pacod-broker-reputation` (engine), `crates/pacod-broker/src/tickets.rs`, `api/public.rs` (intake) |

---

## 1. Why

A catalogue says what providers claim. Consumers also need to know whether those claims hold and whether a provider is dependable. Providers, for their part, need to know whether a client behaves and pays. The broker sees both sides, so it can keep a record of each, as long as that record is hard to game and open about how it was reached.

## 2. Principles

1. **Evidence, not opinion alone.** Most signals are what the broker observes itself (leases, uptime against the declared schedule, changes) or what callers report about a real interaction. A rating counts only when the rater was shown the resource by the broker. Section 6 explains how that is proved.
2. **Transparent.** No score travels alone. Each one carries its components, the evidence it rests on, a confidence and when it was computed. Discovery returns them with every result.
3. **Robust:**
   - **Smoothing.** Bayesian smoothing pulls every score toward a neutral prior (0.5). A newcomer with three good reports does not outrank an established provider.
   - **Decay.** Evidence decays exponentially, with a half-life of 30 days by default. A provider that improves recovers.
   - **Rater weight.** A report counts with its author's own reputation as a client, from 0.5 to 1.5 times. A new rater counts once.
   - **Conflicts of interest.** Reports from the provider's own organisation are ignored.
   - **Rate limits.** A rater counts at most once per resource per hour for ratings, and 120 times per hour for usage reports.
4. **Fair across kinds.** Performance is a percentile **within the resource's kind**. Milliseconds are never compared across kinds: a speech model and a chat model are not rivals.
5. **Scoped.** Scores exist per resource and per provider; a provider's score gathers its resources' evidence. A client's score belongs to its pacod-id principal. Anonymous callers have no reputation.

## 3. Criteria

### 3.1 Providers and resources

Every component is a value in [0, 1]. Each has its own evidence count, and its default weight in the composite is given here.

| Component | Weight | What it measures | Evidence |
|---|---|---|---|
| `availability` | 0.20 | The share of the time the schedule promised that the resource was announced and usable. Maintenance announced at least `BROKER_MAINTENANCE_NOTICE` ahead (24 h by default) is not promised time; maintenance announced late is, and counts as down. | Broker: an uptime sample each minute, one piece of evidence per hour of promised time |
| `reliability` | 0.20 | The share of calls that succeeded. `ok` counts 1; `rate_limited` 0.5; `server_error`, `timeout` and `refused` (refused although capacity was declared) count 0. `client_error` does not count. | Usage reports |
| `performance` | 0.15 | The percentile, within the kind, of observed time to first token plus 200 tokens at the observed per-token latency. Lower is better. | Usage reports with `ok` |
| `accuracy` | 0.15 | Whether claims hold. Observed latency is compared with the declared latency, with 25 % slack. The price charged is compared with the price listed at the time for the rate named. | Usage reports |
| `lease_discipline` | 0.05 | Leases ended with a goodbye (`DELETE /v1/leases/{id}`) or by expiry; status flapping between usable and unusable. | Broker |
| `stability` | 0.05 | Tenure (full after 90 days) and churn: how often prices or interfaces change. | Broker |
| `quality` | 0.15 | Ratings from 1 to 5, each backed by a ticket. | Feedback |
| `trust` | 0.05 | Whether the provider is verified (by pacod-id or a broker admin), minus incidents an admin upheld. | Admin, pacod-id |

### 3.2 Clients

| Component | Weight | What it measures | Evidence |
|---|---|---|---|
| `identity` | 0.25 | How sure the broker is of who the client is. Anonymous scores 0, a person signed in 0.6, a service account 0.7, and a principal of a verified organisation 1. | pacod-id |
| `conduct` | 0.30 | Reports of abuse (weight 2) and policy violations (weight 1) count against it; good standing counts for it. | Provider reports |
| `settlement` | 0.20 | Whether the client pays as agreed: non-payment (weight 2) against, good standing for. | Provider reports |
| `commitment` | 0.10 | Whether the client keeps reservations: no-shows count 0, late cancellations 0.5, good standing counts for it. It becomes significant once slots can be booked. | Provider reports |
| `feedback_honesty` | 0.10 | How far the client's ratings agree with the consensus that stood before them. Only resources with confidence of at least 0.3 count. | Broker |
| `tenure` | 0.05 | The age of the client in the broker's records. It reaches its full value after 90 days. | Broker |

Both sets of weights are configurable: `BROKER_REPUTATION_WEIGHTS_PROVIDER` and `BROKER_REPUTATION_WEIGHTS_CLIENT`, as `component=weight,…`.

## 4. Computation

For each component, the evidence is decayed to the present (half-life `BROKER_REPUTATION_HALF_LIFE_DAYS`) and smoothed toward the prior:

```text
score_c    = (Σ w·x + k·0.5) / (Σ w + k)        k = BROKER_REPUTATION_PRIOR_WEIGHT (10)
composite  = Σ weight_c · score_c / Σ weight_c
confidence = 1 − e^(−n / 20)                    n = evidence counted, after decay
label      = new (confidence < 0.3), else low < 0.4 ≤ fair < 0.6 ≤ good < 0.8 ≤ excellent
```

- **Performance** is ranked within each kind at computation time. A resource with one observation in its kind sits at 0.5. A provider's performance is the evidence-weighted mean of its resources'.
- **Recomputation.** Scores are recomputed every `BROKER_REPUTATION_EVERY` (60 s) and swapped in atomically. Each report updates its aggregate in O(1); discovery only reads.
- **Frozen scores** keep their last value until an admin unfreezes them. A **reset** removes the subject's evidence.

## 5. How reputation is used

| Where | How |
|---|---|
| Discovery results | `reputation` and `provider_reputation` on every hit, with components and confidence. |
| Discovery filters | `min_reputation` (and `min_confidence`, 0.3 by default): resources with less evidence are left out unless `min_confidence` is 0. |
| Discovery sort | `sort: reputation`, within one kind. |
| Provider gating | A resource's `access.min_client_reputation` hides it from clients below the threshold. `allow_new_clients` (true by default) lets clients without enough evidence through. |
| Ticket checks | `POST /v1/tickets/verify` tells a provider who presented a ticket and that client's reputation. |
| The console | Scores by side, lowest first; components, evidence; freeze, unfreeze, reset; client reports that can be taken back; notes. |

## 6. Evidence intake: interaction tickets

Every discovery result for an identified client carries a **ticket**: `tkt_<payload>.<signature>`. The payload is (client, resource, issued at), and the signature is HMAC-SHA256 with the broker's key. A ticket is valid for `BROKER_TICKET_TTL` (24 h by default).

- **Usage reports and ratings** quote a ticket issued to the same client. Without one they do not count, so nobody can rate what the broker never showed them.
- **A provider reporting about a client** quotes the ticket the client presented to it. The broker checks that the ticket's resource is the provider's own.
- **Tickets are stateless.** They are checked without a lookup. The key comes from `BROKER_TICKET_KEY`, or is kept in `BROKER_STATE_DIR/ticket.key`, or is made at start; in the last case tickets do not survive a restart.

| Endpoint | Who | Body |
|---|---|---|
| `POST /v1/reports/usage` | Consumers and gateways | Up to 1000 `{ticket, outcome, ttft_ms?, ms_per_token?, price_charged?, rate?, at?}` |
| `POST /v1/reports/feedback` | Consumers | `{ticket, rating 1–5, comment?}`. The comment is for admins only |
| `POST /v1/reports/client` | Providers | `{ticket, kind: abuse \| policy_violation \| non_payment \| no_show \| late_cancel \| good_standing, details?}` |
| `GET /v1/reputation/me` | Anyone identified | The caller's score as a client and as a provider |

## 7. What is kept

Every piece of evidence that counted is one JSON line in `BROKER_STATE_DIR/reputation.log`, with a sequence number.

- **Snapshot.** Daily and at shutdown, the aggregates are written to `reputation.snap` and the log is started afresh.
- **Restart.** At start the broker reads the snapshot and replays the log lines after its sequence number.
- **Without a state directory**, reputation lives in memory only.

## 8. Not yet

| What | Why it waits |
|---|---|
| Slot booking | Needed to observe no-shows directly. The slot model exists (`availability.schedule.slots`). |
| Synthetic canaries and task-specific evaluations | Quality from probes costs money: a budget per resource is needed first. |
| Attested inventory and certifications | Everything is marked `declared` today. `attested` is reserved. |
| Exchange of evidence between brokers | Comes with federation. The evidence is self-contained so that it can travel. |
