"""Python client of pacod-broker.

Consumers::

    from pacod_broker import BrokerClient, UsageReport

    broker = BrokerClient("http://127.0.0.1:8060", client_name="my-agent")
    found = broker.discover(kind="model", capabilities=["tools"], currency="EUR", sort="price:input")
    hit = found["hits"][0]
    # ... call hit["resource"]["interfaces"][0]["url"] ...
    broker.report_usage(UsageReport(hit["ticket"], "ok", ttft_ms=310))
    broker.rate(hit["ticket"], 5)

Providers::

    from pacod_broker import BrokerClient, Provider, load_announcement

    with Provider(BrokerClient(), load_announcement("provider.yaml")) as p:
        p.set_in_use(3)
        serve_forever()
"""

from .client import AsyncBrokerClient, BrokerClient, BrokerError, PendingApproval, UsageReport
from .provider import AsyncProvider, Provider, load_announcement

__all__ = [
    "AsyncBrokerClient",
    "AsyncProvider",
    "BrokerClient",
    "BrokerError",
    "PendingApproval",
    "Provider",
    "UsageReport",
    "load_announcement",
]
__version__ = "0.1.0"
