import copy
import json
import threading

import pytest

from pacod_broker import AsyncBrokerClient, AsyncProvider, BrokerClient, BrokerError, Provider, UsageReport
from pacod_broker.cli import main
from pacod_broker.sse import Parser, to_event

from .conftest import ANNOUNCEMENT, eventually


def test_sse_parser_handles_split_chunks():
    p = Parser()
    assert p.push(b"event: hello\nda") == []
    out = p.push(b'ta: {"generation": 3}\n\n: keep-alive\n\nevent: removed\ndata: {"resource_id": "r"}\n\n')
    assert [to_event(*e) for e in out] == [{"generation": 3, "type": "hello"}, {"resource_id": "r", "type": "removed"}]
    assert to_event("future-kind", "{}") is None


def test_provider_and_consumer(broker_url):
    consumer = BrokerClient(broker_url, client_name="py-consumer")
    assert consumer.ready()

    events = []
    watched = []
    stop_watch = threading.Event()

    def follow():
        for e in consumer.watch():
            watched.append(e)
            if stop_watch.is_set() and e["type"] == "removed":
                return

    threading.Thread(target=follow, daemon=True).start()

    provider = Provider(BrokerClient(broker_url), copy.deepcopy(ANNOUNCEMENT), lambda k, v: events.append((k, v)))
    first = provider.start()
    rid = first["resources"]["qwen"]
    provider.set_in_use(2)

    found = eventually(lambda: consumer.discover(kind="model", q="qwen")["hits"], "discovery")
    hit = found[0]
    assert hit["id"] == rid and hit["ticket"].startswith("tkt_")
    assert consumer.resources(kind="model", software="ollama") is not None
    assert consumer.discover(software=[{"name": "ollama", "version": ">=0.5"}])["total"] == 1

    eventually(lambda: consumer.resource(rid)["live"].get("in_use") == 2, "live state")
    assert consumer.provider(first["provider_id"])["profile"]["name"] == "python-client-test"

    assert consumer.report_usage(UsageReport(hit["ticket"], "ok", ttft_ms=120))["accepted"] == 1
    assert consumer.rate(hit["ticket"], 4, "fine")["accepted"] == 1
    with pytest.raises(ValueError):
        consumer.rate(hit["ticket"], 0)
    with pytest.raises(BrokerError) as e:
        BrokerClient(broker_url, client_name="someone-else").rate(hit["ticket"], 1)
    assert e.value.status == 422

    as_provider = BrokerClient(broker_url)
    assert as_provider.verify_ticket(hit["ticket"])["client"] == "anon:py-consumer"
    assert as_provider.report_client(hit["ticket"], "good_standing")["counted"]
    assert consumer.me()["client"] == "anon:py-consumer"

    # The broker forgets the lease (a goodbye sent behind the provider's back); the provider
    # announces again by itself.
    as_provider.deregister(first["lease_id"], "python-client-test")
    eventually(lambda: any(k == "reannounced" for k, _ in events), "re-announcement")
    assert provider.announced["resources"]["qwen"] == rid

    # The price feed, and an update under the same lease.
    feed = consumer.price_feed(provider="python-client-test")
    assert isinstance(feed["models"], dict)
    changed = copy.deepcopy(ANNOUNCEMENT)
    second = copy.deepcopy(changed["resources"][0])
    second["key"], second["name"] = "qwen-b", "qwen-b"
    changed["resources"].append(second)
    lease = provider.announced["lease_id"]
    updated = provider.update(changed)
    assert updated["lease_id"] == lease and updated["resources"]["qwen"] == rid and "qwen-b" in updated["resources"]

    stop_watch.set()
    provider.stop()
    eventually(lambda: consumer.discover(q="qwen")["total"] == 0, "removal")
    kinds = {e["type"] for e in watched}
    assert {"hello", "upserted", "removed"} <= kinds, kinds


async def test_async_provider_and_consumer(broker_url):
    ann = copy.deepcopy(ANNOUNCEMENT)
    ann["provider"]["name"] = "python-async-test"
    async with AsyncBrokerClient(broker_url, client_name="py-async") as consumer:
        async with AsyncProvider(AsyncBrokerClient(broker_url), ann) as provider:
            rid = provider.announced["resources"]["qwen"]
            import asyncio

            for _ in range(100):
                r = await consumer.discover(provider=provider.announced["provider_id"])
                if r["total"]:
                    break
                await asyncio.sleep(0.05)
            assert r["hits"][0]["id"] == rid
            assert (await consumer.report_usage(UsageReport(r["hits"][0]["ticket"], "timeout")))["accepted"] == 1
        for _ in range(100):
            if (await consumer.discover(q="qwen", provider=provider.announced["provider_id"]))["total"] == 0:
                break
            await asyncio.sleep(0.05)
        else:
            raise AssertionError("not removed")


def test_problems_are_typed(broker_url):
    c = BrokerClient(broker_url)
    with pytest.raises(BrokerError) as e:
        c.discover(sort="reputation")
    assert e.value.status == 422 and e.value.kind == "invalid"
    bad = copy.deepcopy(ANNOUNCEMENT)
    bad["resources"][0]["kind"] = "nonsense"
    with pytest.raises(BrokerError) as e:
        c.announce(bad)
    assert any("resources[0].kind" in x for x in e.value.errors)
    with pytest.raises(BrokerError) as e:
        c.resource("res_nope")
    assert e.value.not_found


def test_cli(broker_url, capsys, tmp_path):
    f = tmp_path / "a.json"
    ann = copy.deepcopy(ANNOUNCEMENT)
    ann["provider"]["name"] = "python-cli-test"
    f.write_text(json.dumps(ann))
    assert main(["--url", broker_url, "announce", str(f), "--once"]) == 0
    lease = json.loads(capsys.readouterr().out)["lease_id"]
    eventually(lambda: BrokerClient(broker_url).discover(q="qwen")["total"] >= 1, "discovery")
    assert main(["--url", broker_url, "--client", "cli", "discover", "-q", "qwen", "--capability", "tools"]) == 0
    assert "qwen-test" in capsys.readouterr().out
    assert main(["--url", broker_url, "--json", "discover", "--sort", "reputation"]) == 1
    BrokerClient(broker_url).deregister(lease, "python-cli-test")
