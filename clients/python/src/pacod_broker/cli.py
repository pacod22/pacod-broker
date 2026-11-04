"""pacod-broker: the broker from the command line (the same commands as broker-cli).

    pacod-broker discover --kind model --capability tools --currency EUR --sort price:input
    pacod-broker announce provider.yaml        # keeps the lease alive until Ctrl-C
    pacod-broker report-usage --ticket tkt_… --outcome ok --ttft-ms 310
    pacod-broker rate --ticket tkt_… --rating 5
    pacod-broker watch

The broker comes from --url or BROKER_URL; a pacod-id token from --token or BROKER_TOKEN; in open
mode a client name from --client or BROKER_CLIENT.
"""

from __future__ import annotations

import argparse
import json
import os
import signal
import sys
import threading
from typing import Any

from .client import BrokerClient, BrokerError, UsageReport
from .provider import Provider, load_announcement


def _print(v: Any) -> None:
    print(json.dumps(v, indent=2, ensure_ascii=False))


def _price(h: dict[str, Any]) -> str:
    p = h.get("price")
    if p:
        if p.get("free"):
            return "free"
        rates = p.get("rates") or {}
        if rates:
            k, v = next(iter(rates.items()))
            return f"{k} {v} {p.get('currency', '')}"
    s = h.get("price_summary")
    if s and s.get("rates"):
        k, r = next(iter(s["rates"].items()))
        return f"{k} avg {r['avg']:.4f} {s.get('currency', '')}"
    return "—"


def _cut(s: str, n: int) -> str:
    return s if len(s) <= n else s[: n - 1] + "…"


def _table(r: dict[str, Any]) -> None:
    print(f"{r['total']} found (generation {r['generation']})")
    print(f"{'RESOURCE':<34} {'KIND':<12} {'PROVIDER':<14} {'PRICE':<26} {'STATUS':<11} {'REP':>6}  ID")
    for h in r["hits"]:
        status = h["live"]["status"] if h["available"] else "unavailable"
        rep = f"{h['reputation']['score'] * 100:.0f}%" if h.get("reputation") else "new"
        print(
            f"{_cut(h['resource']['name'], 34):<34} {_cut(h['resource']['kind'], 12):<12} "
            f"{_cut(h['provider']['name'], 14):<14} {_cut(_price(h), 26):<26} {status:<11} {rep:>6}  {h['id']}"
        )
    if r.get("next_cursor"):
        print(f'more: --query with "cursor": "{r["next_cursor"]}"')


def _query(a: argparse.Namespace) -> dict[str, Any]:
    q: dict[str, Any] = {}
    if a.query:
        with open(a.query, encoding="utf-8") as f:
            q = json.load(f)
    for key, value in (("kind", a.kind), ("q", a.q), ("currency", a.currency), ("at", a.at),
                       ("sort", a.sort), ("limit", a.limit), ("min_reputation", a.min_reputation)):
        if value is not None:
            q[key] = value
    for key, values in (("tags", a.tag), ("capabilities", a.capability), ("protocols", a.protocol), ("regions", a.region)):
        if values:
            q[key] = q.get(key, []) + values
    for m in a.max_price or []:
        rate, _, v = m.partition("=")
        q.setdefault("max_price", {})[rate] = float(v)
    if a.during:
        start, _, end = a.during.partition("..")
        q["during"] = {"from": start, "to": end}
    if a.gpu:
        q.setdefault("hardware", {})["gpu_model"] = a.gpu
    for s in a.software or []:
        name, _, version = s.partition("@")
        q.setdefault("software", []).append({"name": name, **({"version": version} if version else {})})
    return q


def _parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(prog="pacod-broker", description="pacod-broker from the command line")
    p.add_argument("--url", default=os.environ.get("BROKER_URL", "http://127.0.0.1:8060"))
    p.add_argument("--token", default=os.environ.get("BROKER_TOKEN"), help="a pacod-id token (pidt_… for services)")
    p.add_argument("--client", default=os.environ.get("BROKER_CLIENT"), help="open mode: this client's name")
    p.add_argument("--json", action="store_true", help="print the broker's JSON instead of a table")
    sub = p.add_subparsers(dest="command", required=True)

    d = sub.add_parser("discover", help="find resources")
    d.add_argument("--query", help="a whole query as a JSON file; the flags are added to it")
    d.add_argument("--kind")
    d.add_argument("-q", "--q", help="words to search for")
    for name in ("tag", "capability", "protocol", "region", "software"):
        d.add_argument(f"--{name}", action="append")
    d.add_argument("--max-price", action="append", help="rate=value, such as input=0.5")
    d.add_argument("--currency")
    d.add_argument("--at", help="an instant, RFC 3339")
    d.add_argument("--during", help="FROM..TO, RFC 3339")
    d.add_argument("--gpu")
    d.add_argument("--min-reputation", type=float)
    d.add_argument("--sort")
    d.add_argument("--limit", type=int)

    sub.add_parser("resource", help="one resource").add_argument("id")
    sub.add_parser("provider", help="one provider").add_argument("id")

    an = sub.add_parser("announce", help="announce from a YAML or JSON file and keep the lease alive")
    an.add_argument("file")
    an.add_argument("--ttl", type=int)
    an.add_argument("--once", action="store_true", help="announce once and leave")

    u = sub.add_parser("report-usage", help="report how a call went")
    u.add_argument("--ticket", required=True)
    u.add_argument("--outcome", required=True,
                   choices=["ok", "client_error", "server_error", "timeout", "rate_limited", "refused"])
    u.add_argument("--ttft-ms", type=float)
    u.add_argument("--ms-per-token", type=float)
    u.add_argument("--price-charged", type=float)
    u.add_argument("--rate")

    r = sub.add_parser("rate", help="rate a resource, 1 to 5")
    r.add_argument("--ticket", required=True)
    r.add_argument("--rating", type=int, required=True, choices=range(1, 6))
    r.add_argument("--comment")

    c = sub.add_parser("report-client", help="providers: report about a client")
    c.add_argument("--ticket", required=True)
    c.add_argument("--kind", required=True,
                   choices=["abuse", "policy_violation", "non_payment", "no_show", "late_cancel", "good_standing"])
    c.add_argument("--details")

    sub.add_parser("verify-ticket", help="providers: who presented a ticket").add_argument("ticket")
    sub.add_parser("watch", help="follow changes").add_argument("--after", type=int)
    sub.add_parser("me", help="your own reputation")
    sub.add_parser("schema", help="the JSON Schemas of what the API takes")
    return p


def main(argv: list[str] | None = None) -> int:
    a = _parser().parse_args(argv)
    broker = BrokerClient(a.url, token=a.token or None, client_name=None if a.token else a.client)
    try:
        if a.command == "discover":
            r = broker.discover(_query(a))
            _print(r) if a.json else _table(r)
        elif a.command == "resource":
            _print(broker.resource(a.id))
        elif a.command == "provider":
            _print(broker.provider(a.id))
        elif a.command == "announce":
            ann = load_announcement(a.file)
            if a.ttl:
                ann["ttl_s"] = a.ttl
            if a.once:
                _print(broker.announce(ann))
                return 0
            done = threading.Event()

            def on_event(kind: str, value: Any) -> None:
                if kind == "announced":
                    print(f"announced {len(value['resources'])} resources under {value['lease_id']}; "
                          f"heartbeat every {value['heartbeat_every_s']} s (Ctrl-C ends the lease)", file=sys.stderr)
                    for w in value.get("warnings", []):
                        print(f"warning: {w}", file=sys.stderr)
                elif kind == "reannounced":
                    print(f"the broker had forgotten the lease; announced again under {value['lease_id']}", file=sys.stderr)
                elif kind == "retrying":
                    print(f"retrying: {value}", file=sys.stderr)
                elif kind == "pending":
                    print("waiting for an admin's approval", file=sys.stderr)

            provider = Provider(broker, ann, on_event)
            provider.start()
            signal.signal(signal.SIGTERM, lambda *_: done.set())
            try:
                done.wait()
            except KeyboardInterrupt:
                pass
            provider.stop()
            print("lease ended", file=sys.stderr)
        elif a.command == "report-usage":
            _print(broker.report_usage(UsageReport(a.ticket, a.outcome, a.ttft_ms, a.ms_per_token, a.price_charged, a.rate)))
        elif a.command == "rate":
            _print(broker.rate(a.ticket, a.rating, a.comment))
        elif a.command == "report-client":
            _print(broker.report_client(a.ticket, a.kind, a.details))
        elif a.command == "verify-ticket":
            _print(broker.verify_ticket(a.ticket))
        elif a.command == "watch":
            for e in broker.watch(a.after):
                print(json.dumps(e), flush=True)
        elif a.command == "me":
            _print(broker.me())
        elif a.command == "schema":
            _print(broker.schema())
    except BrokerError as e:
        print(f"the broker refused: {e}", file=sys.stderr)
        return 1
    except KeyboardInterrupt:
        return 130
    finally:
        broker.close()
    return 0


if __name__ == "__main__":
    sys.exit(main())
