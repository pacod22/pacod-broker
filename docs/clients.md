# pacod-broker: clients

Three clients, one design. Each is a library and a command line.

| Language | Where | Library | Command line |
|---|---|---|---|
| Rust | [crates/pacod-broker-client](../crates/pacod-broker-client) | `pacod_broker_client` | `broker-cli` |
| Python 3.10+ | [clients/python](../clients/python) | `pacod_broker`: blocking and asyncio | `pacod-broker` |
| TypeScript (Node 20+) | [clients/typescript](../clients/typescript) | `@pacod/broker-client` | `pacod-broker` |

**Where the types come from:**
- **Rust** uses the broker's own model types (`ProviderProfile`, `ResourceSpec`, `Query`, prices, availability). `Announcement::validate` runs the broker's own checks before anything is sent.
- **TypeScript** takes its types from [openapi.json](openapi.json), which the broker generates from its code (`make openapi`).
- **Python** works with the broker's JSON as dicts, documented in [api.md](api.md).

## 1. Connecting

| Setting | Variable | Default | What |
|---|---|---|---|
| URL | `BROKER_URL` | `http://127.0.0.1:8060` | The public API |
| Token | `BROKER_TOKEN` | none | A pacod-id access token, or a service account's `pidt_…` token (`BROKER_AUTH=oidc`) |
| Client name | `BROKER_CLIENT` | none | Open mode only: the name the client gives itself (`X-Broker-Client`), so that its tickets and reports are its own |

Every client and command line reads these variables, and each takes them as arguments too.

## 2. Providers

A provider announces its profile and resources, then keeps the lease alive:

1. **Heartbeats.** It heartbeats every `heartbeat_every_s` seconds, as the broker asked. Each heartbeat carries the live state the code sets: status (`active`, `degraded`, `draining`, `maintenance`), capacity in use, observed time to first token, overall or per resource.
2. **Re-announcing.** When the broker answers 404 to a heartbeat, it no longer knows the lease: it restarted, or the lease ran out while the network was down. The provider then announces again by itself. Resource ids do not change, because the broker derives them from the provider and the key.
3. **Retries.** When the broker cannot be reached, the provider tries again with back-off (1, 2, 4… seconds, never more than the heartbeat interval).
4. **Goodbye.** `stop` ends the lease cleanly. Reputation counts this in the provider's favour; a lease that just runs out counts against it.

Announcements are written as YAML or JSON, like [examples/announcement.example.yaml](../examples/announcement.example.yaml) ([concept.md](concept.md) section 3 explains the fields).

```rust
let broker = BrokerClient::from_env()?;
let provider = Provider::new(broker, Announcement::parse(&std::fs::read_to_string("announcement.yaml")?)?);
let live = provider.live();                 // keep it: set_status, set_in_use, set_ttft_ms, set_resource
let running = provider.start().await?;      // or start_with(|event| …) to hear re-announcements
live.set_in_use(3);
running.stop().await?;
```

```python
from pacod_broker import BrokerClient, Provider, load_announcement

with Provider(BrokerClient(), load_announcement("announcement.yaml")) as p:
    p.set_in_use(3)
    serve_forever()
# asyncio: async with AsyncProvider(AsyncBrokerClient(), announcement) as p: …
```

```ts
import { BrokerClient, Provider } from '@pacod/broker-client';

const provider = new Provider(new BrokerClient(), announcement, (e) => console.log(e.kind));
await provider.start();
provider.setInUse(3);
await provider.stop();
```

## 3. Consumers

| What | Rust | Python | TypeScript |
|---|---|---|---|
| Discover (docs/api.md §3.1) | `discover(&Query)`, `discover_all` | `discover(query, **filters)`, `discover_all` | `discover(query)`, `discoverAll` |
| Simple form | `resources(&[("kind", "model")])` | `resources(kind="model")` | `resources({ kind: 'model' })` |
| One resource, one provider | `resource(id)`, `provider(id)` | same | same |
| Watch changes | `watch(after)` (a `Stream`) | `watch(after)` (an iterator; `async for` with the async client) | `watch({ after, signal })` (`for await`) |
| Report a call | `report_usage(&[UsageReport::new(t, Outcome::Ok).latency(…)])` | `report_usage(UsageReport(t, "ok", ttft_ms=…))` | `reportUsage({ ticket, outcome: 'ok', ttft_ms })` |
| Rate | `rate(ticket, 5, comment)` | same | same |
| Own reputation | `me()` | same | same |

**Tickets.** Every discovery result for an identified client carries a `ticket`. Quote it when reporting how the call went and when rating. Without a ticket, or with another client's, a report does not count ([reputation.md](reputation.md) section 6).

```python
broker = BrokerClient(client_name="my-agent")
hit = broker.discover(kind="model", capabilities=["tools"], currency="EUR", sort="price:input")["hits"][0]
url = hit["resource"]["interfaces"][0]["url"]
# … call the model …
broker.report_usage(UsageReport(hit["ticket"], "ok", ttft_ms=310, ms_per_token=18))
broker.rate(hit["ticket"], 5)
```

**Provider-side ticket calls.** A provider that received a ticket from a client can ask who it is with `verify_ticket` / `verifyTicket`, and report about it with `report_client` / `reportClient`.

## 4. Errors

| Error | When |
|---|---|
| `Error::Problem`, `BrokerError` | The broker refused. Each carries the HTTP status, the stable `type` (`invalid`, `not_found`, `forbidden`, `identity_unavailable`, …) and, for invalid input, every problem with its path. |
| `Error::Pending`, `PendingApproval` | The announcement waits for an admin's approval (`BROKER_APPROVAL=new_providers`). |
| Before sending | A rating outside 1 to 5 is refused by the client itself. |

## 5. Command lines

The three command lines have the same commands and flags:

```bash
broker-cli discover --kind model --capability tools --currency EUR --max-price input=0.5 --sort price:input
```

```bash
broker-cli discover --gpu H100-SXM --software 'vllm@>=0.7' --during 2026-11-10T20:00:00Z..2026-11-11T06:00:00Z
```

```bash
broker-cli announce examples/announcement.example.yaml
```

```bash
broker-cli report-usage --ticket tkt_… --outcome ok --ttft-ms 310
```

```bash
broker-cli rate --ticket tkt_… --rating 5 --comment "fast"
```

```bash
broker-cli watch
```

| Command | What |
|---|---|
| `discover` | Find resources. `--json` prints the broker's answer; otherwise a table. |
| `resource ID`, `provider ID` | One resource or provider |
| `announce FILE` | Announce and keep the lease alive until Ctrl-C. `--once` announces once; `--ttl` sets the lease. `broker-cli` also has `--check`, which runs the broker's own checks on the file without sending it. |
| `report-usage`, `rate` | Report a call; rate a resource |
| `report-client`, `verify-ticket` | Providers: report about a client; see who presented a ticket |
| `watch` | Follow changes |
| `me` | Your own reputation |
| `schema` | The JSON Schemas of what the API takes |

Python's and TypeScript's command lines are both called `pacod-broker`. Install them with `make clients`, or with `uv pip install ./clients/python` and `npm install ./clients/typescript`.

## 6. Tests

Each client is tested against a real broker on a local port. The test covers:
- announcing;
- discovery with a ticket;
- live state carried by heartbeats;
- usage and rating reports, and their refusal for another client's ticket;
- the provider-side ticket calls;
- re-announcing when the broker forgets the lease;
- the goodbye and the removal it causes;
- the watch stream;
- the command line.

| Client | Command |
|---|---|
| Rust | `cargo test -p pacod-broker --test client` |
| Python, TypeScript | `make clients-test` |
