"""Python reference for the broker stream driver (`tests/test_rust_ai_broker.py`).

Reads a JSON scenario on stdin, drives `BrokerAIClient.stream_chat` through
its legs, and prints every chunk and each leg's outcome as one JSON line, in
the shape `crates/ai/examples/broker_stream.rs` prints for the Rust client.
The broker is found through the environment overrides, as in the product.
"""

from __future__ import annotations

import json
import sys
from unittest import mock


def main() -> int:
    from kassiber import egress_policy
    from kassiber.ai import broker_client
    from kassiber.ai.broker_client import BrokerAIClient
    from kassiber.ai.contracts import ResponsesRequestContext
    from kassiber.errors import AppError

    scenario = json.loads(sys.stdin.read())
    launches = 0
    start = broker_client._start_broker

    def counted(node, script):
        nonlocal launches
        launches += 1
        return start(node, script)

    offline = mock.patch.object(egress_policy, "offline_mode_enabled", return_value=bool(scenario.get("offline")))
    with mock.patch.object(broker_client, "_start_broker", counted), offline:
        client = BrokerAIClient(locator=scenario["locator"], timeout=scenario.get("timeout", 120))
        for leg in scenario["legs"]:
            context = leg.get("context")
            seen = 0
            error = None
            try:
                for chunk in client.stream_chat(
                    messages=leg.get("messages"),
                    model=leg["model"],
                    options=leg.get("options"),
                    tools=leg.get("tools"),
                    context=(
                        ResponsesRequestContext(
                            instructions=context.get("instructions"),
                            input_items=context.get("input_items") or [],
                        )
                        if context is not None
                        else None
                    ),
                ):
                    print(
                        json.dumps(
                            {
                                "delta": chunk.delta,
                                "finish_reason": chunk.finish_reason,
                                "raw": chunk.raw,
                                "response_output": chunk.response_output,
                                "status_phase": chunk.status_phase,
                            }
                        ),
                        flush=True,
                    )
                    seen += 1
                    if leg.get("cancel_after") == seen:
                        client.cancel()
            except AppError as exc:
                error = {"code": exc.code, "message": str(exc), "hint": exc.hint, "retryable": exc.retryable}
            print(
                json.dumps(
                    {
                        "end": {
                            "error": error,
                            "last_provider_session_id": client.last_provider_session_id,
                            "pending": sorted(client._pending_call_ids, key=json.dumps),
                            "alive": client._process is not None,
                        }
                    }
                ),
                flush=True,
            )
    print(json.dumps({"launches": launches}), flush=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
