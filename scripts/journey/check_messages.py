#!/usr/bin/env python3
"""Consumer-side checks for the end-to-end journey (scripts/test-journey.sh).

Reads a capture written by ``mosquitto_sub -F '%t<TAB>%r<TAB>%p'`` and checks
what the MQTT Output Adapter v1 contract promises to an independent consumer:
topic grammar, the exact key order and types of every payload, continuous
sequences within a series, and the values the deterministic ``trial-sample``
waveform must produce. Each subcommand prints ``ok``/``FAIL`` lines and exits
non-zero on any failure. No dependency beyond the standard library.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from collections import OrderedDict
from dataclasses import dataclass

TOPIC_RE = re.compile(
    r"^iotkit/v1/edge-node/(?P<node>[a-z0-9](?:[a-z0-9-]*[a-z0-9])?)/"
    r"(?:observation/(?P<pipeline>[a-z0-9](?:[a-z0-9-]*[a-z0-9])?)/"
    r"(?P<kind>measurement|accumulated-count|state)|(?P<status>status))$"
)
OBSERVATION_KEYS = ["series_id", "sequence", "uptime_ms", "unix_epoch_ms", "value"]
STATUS_KEYS = ["uptime_ms", "unix_epoch_ms", "value", "faults"]
WILL_KEYS = ["uptime_ms", "unix_epoch_ms", "value"]
FAULT_KEYS = {
    "storage-write-failed": ["kind", "since_uptime_ms", "since_unix_epoch_ms", "count"],
    "interface-open-failed": [
        "kind",
        "since_uptime_ms",
        "since_unix_epoch_ms",
        "adapter",
        "reason",
    ],
}
# trial-sample illuminance: a triangle wave 120..200 in steps of 8, so two
# consecutive inputs always differ by exactly 8.
MEASUREMENT_STEP = 8.0
MEASUREMENT_RANGE = (120.0, 200.0)


@dataclass
class Message:
    line: int
    topic: str
    retained: bool
    raw: str
    payload: object  # parsed JSON (OrderedDict) or None for a zero-length payload
    node: str
    pipeline: str | None
    kind: str | None  # observation kind or "status"


failures = 0


def ok(name: str) -> None:
    print(f"ok   {name}")


def fail(name: str, detail: str = "") -> None:
    global failures
    failures += 1
    print(f"FAIL {name}", file=sys.stderr)
    if detail:
        print(f"     {detail}", file=sys.stderr)


def check(name: str, condition: bool, detail: str = "") -> bool:
    if condition:
        ok(name)
    else:
        fail(name, detail)
    return condition


def load(path: str, node: str) -> list[Message]:
    messages: list[Message] = []
    with open(path, encoding="utf-8") as handle:
        for number, line in enumerate(handle, start=1):
            line = line.rstrip("\n")
            if not line:
                continue
            parts = line.split("\t", 2)
            if len(parts) != 3:
                fail(f"line {number}: capture format", repr(line))
                continue
            topic, retained, raw = parts
            if not topic.startswith("iotkit/"):
                continue  # the script's own subscription probe
            match = TOPIC_RE.match(topic)
            if not match or match.group("node") != node:
                fail(f"line {number}: topic grammar", topic)
                continue
            payload = None
            if raw != "":
                try:
                    payload = json.loads(raw, object_pairs_hook=OrderedDict)
                except json.JSONDecodeError as error:
                    fail(f"line {number}: payload is JSON", f"{topic}: {error}")
                    continue
            messages.append(
                Message(
                    line=number,
                    topic=topic,
                    retained=retained == "1",
                    raw=raw,
                    payload=payload,
                    node=match.group("node"),
                    pipeline=match.group("pipeline"),
                    kind=match.group("kind") or "status",
                )
            )
    return messages


def is_int(value: object) -> bool:
    return isinstance(value, int) and not isinstance(value, bool)


def check_observation_shape(message: Message) -> bool:
    name = f"line {message.line}: {message.topic}"
    payload = message.payload
    if payload is None:
        return True  # deletion; checked by the caller
    if list(payload.keys()) != OBSERVATION_KEYS:
        fail(f"{name}: key order", f"{list(payload.keys())} != {OBSERVATION_KEYS}")
        return False
    if message.raw != json.dumps(payload, separators=(",", ":"), ensure_ascii=False):
        fail(f"{name}: canonical form (no whitespace)", message.raw)
        return False
    series_id = payload["series_id"]
    if not isinstance(series_id, str) or not 1 <= len(series_id.encode("utf-8")) <= 64:
        fail(f"{name}: series_id", repr(series_id))
        return False
    if not is_int(payload["sequence"]) or not 1 <= payload["sequence"] <= 2**53 - 1:
        fail(f"{name}: sequence", repr(payload["sequence"]))
        return False
    if not is_int(payload["uptime_ms"]) or payload["uptime_ms"] < 0:
        fail(f"{name}: uptime_ms", repr(payload["uptime_ms"]))
        return False
    if payload["unix_epoch_ms"] is not None and not is_int(payload["unix_epoch_ms"]):
        fail(f"{name}: unix_epoch_ms", repr(payload["unix_epoch_ms"]))
        return False
    value = payload["value"]
    if message.kind == "measurement" and (
        isinstance(value, bool) or not isinstance(value, (int, float))
    ):
        fail(f"{name}: measurement value is a JSON number", repr(value))
        return False
    if message.kind == "state" and not isinstance(value, bool):
        fail(f"{name}: state value is a boolean", repr(value))
        return False
    if message.kind == "accumulated-count" and (not is_int(value) or value < 0):
        fail(f"{name}: accumulated-count value is a non-negative integer", repr(value))
        return False
    return True


def check_status_shape(message: Message) -> bool:
    name = f"line {message.line}: status"
    payload = message.payload
    if payload is None:
        fail(f"{name}: payload present")
        return False
    keys = list(payload.keys())
    if keys == WILL_KEYS:
        if payload["uptime_ms"] is not None or payload["unix_epoch_ms"] is not None:
            fail(f"{name}: Will has null times", message.raw)
            return False
        if payload["value"] != "offline":
            fail(f"{name}: Will is offline", message.raw)
            return False
        return True
    if keys != STATUS_KEYS:
        fail(f"{name}: key order", f"{keys} != {STATUS_KEYS}")
        return False
    if message.raw != json.dumps(payload, separators=(",", ":"), ensure_ascii=False):
        fail(f"{name}: canonical form (no whitespace)", message.raw)
        return False
    if not is_int(payload["uptime_ms"]) or payload["uptime_ms"] < 0:
        fail(f"{name}: uptime_ms", repr(payload["uptime_ms"]))
        return False
    if payload["unix_epoch_ms"] is not None and not is_int(payload["unix_epoch_ms"]):
        fail(f"{name}: unix_epoch_ms", repr(payload["unix_epoch_ms"]))
        return False
    if payload["value"] not in ("online", "degraded", "offline"):
        fail(f"{name}: value", repr(payload["value"]))
        return False
    for fault in payload["faults"]:
        kind = fault.get("kind")
        expected = FAULT_KEYS.get(kind)
        if expected is None:
            fail(f"{name}: known fault kind", repr(kind))
            return False
        keys = [key for key in fault.keys() if key != "detail"]
        if keys != expected:
            fail(f"{name}: fault key order", f"{keys} != {expected}")
            return False
    if payload["value"] == "degraded" and not any(
        fault["kind"] == "storage-write-failed" for fault in payload["faults"]
    ):
        fail(f"{name}: degraded carries storage-write-failed", message.raw)
        return False
    return True


def observations(messages: list[Message], pipeline: str) -> list[Message]:
    return [m for m in messages if m.pipeline == pipeline and m.payload is not None]


def check_series_continuity(name: str, items: list[Message]) -> bool:
    """One series with sequences 1, 2, 3, ... in receipt order."""
    if not items:
        fail(name, "no observations")
        return False
    series = {m.payload["series_id"] for m in items}
    if len(series) != 1:
        fail(f"{name}: one series", f"{series}")
        return False
    sequences = [m.payload["sequence"] for m in items]
    if sequences != list(range(1, len(sequences) + 1)):
        fail(f"{name}: sequences are 1..n without gaps", f"{sequences}")
        return False
    uptimes = [m.payload["uptime_ms"] for m in items]
    if uptimes != sorted(uptimes):
        fail(f"{name}: uptime_ms never decreases within one boot", f"{uptimes}")
        return False
    ok(f"{name}: one series, sequences 1..{len(sequences)}, uptime monotonic")
    return True


def cmd_l1(args: argparse.Namespace) -> None:
    messages = load(args.capture, args.node)
    shapes = [
        check_status_shape(m) if m.kind == "status" else check_observation_shape(m)
        for m in messages
    ]
    check("every payload matches the contract's key order and types", all(shapes))

    heartbeats = [m for m in messages if m.kind == "status" and m.payload is not None]
    check(
        "heartbeat online with faults [] arrives",
        any(m.payload.get("value") == "online" and m.payload.get("faults") == [] for m in heartbeats),
    )

    measurement = observations(messages, args.measurement)
    state = observations(messages, args.state)
    count = observations(messages, args.count)
    check_series_continuity(f"{args.measurement} (measurement)", measurement)
    check_series_continuity(f"{args.state} (state)", state)
    check_series_continuity(f"{args.count} (accumulated-count)", count)

    values = [float(m.payload["value"]) for m in measurement]
    check(
        "measurement values follow the trial-sample triangle wave (120..200, step 8)",
        len(values) >= 8
        and all(MEASUREMENT_RANGE[0] <= v <= MEASUREMENT_RANGE[1] for v in values)
        and all(abs(b - a) == MEASUREMENT_STEP for a, b in zip(values, values[1:])),
        f"{values}",
    )
    check(
        "measurement publishes every input (consecutive sequences, never skipped for equal values)",
        len(measurement) >= len(state),
    )

    if count:
        first = count[0].payload
        check(
            "new accumulated-count series starts with sequence 1, value 0",
            first["sequence"] == 1 and first["value"] == 0,
            count[0].raw,
        )
        counts = [m.payload["value"] for m in count]
        check(
            "accumulated-count increases by exactly 1 per publication",
            counts == list(range(len(counts))),
            f"{counts}",
        )
        rising = sum(1 for m in state[1:] if m.payload["value"] is True)
        check(
            "accumulated-count equals the rising edges of the state pipeline",
            counts[-1] == rising,
            f"count {counts[-1]} vs {rising} rising edges after the initial state",
        )
        check(
            "accumulated-count reached the requested minimum",
            counts[-1] >= args.min_count,
            f"{counts[-1]} < {args.min_count}",
        )
    if state:
        transitions = [m.payload["value"] for m in state]
        check(
            "state publishes only changes after its initial value",
            all(a != b for a, b in zip(transitions, transitions[1:])),
            f"{transitions}",
        )


def cmd_shape(args: argparse.Namespace) -> None:
    """Shape of every message in a capture; used for the later L2 captures."""
    messages = load(args.capture, args.node)
    shapes = [
        check_status_shape(m) if m.kind == "status" else check_observation_shape(m)
        for m in messages
    ]
    check(f"{args.capture}: every payload matches the contract", all(shapes) and bool(messages))


def last_by_topic(messages: list[Message]) -> dict[str, Message]:
    last: dict[str, Message] = {}
    for m in messages:
        if m.kind != "status" and m.payload is not None:
            last[m.topic] = m
    return last


def cmd_continues(args: argparse.Namespace) -> None:
    """The `after` capture continues each series of the `before` capture.

    The first message per topic may repeat the last sequence seen before (the
    retransmission of the one publication that was in flight); after that the
    sequence increases by one per message. The series never changes.
    """
    before = last_by_topic(load(args.before, args.node))
    after = load(args.after, args.node)
    for topic, previous in sorted(before.items()):
        following = [m for m in after if m.topic == topic and m.payload is not None]
        if not following:
            fail(f"{topic}: observations after the event", "none")
            continue
        series_ok = all(m.payload["series_id"] == previous.payload["series_id"] for m in following)
        sequences = [m.payload["sequence"] for m in following]
        start = previous.payload["sequence"]
        allowed_start = {start, start + 1} if args.allow_duplicate else {start + 1}
        contiguous = sequences[0] in allowed_start and all(
            b == a + 1 for a, b in zip(sequences, sequences[1:])
        )
        check(
            f"{topic}: same series and continuous sequence across the event",
            series_ok and contiguous,
            f"before {start}, after {sequences[:6]}...",
        )


def cmd_retained(args: argparse.Namespace) -> None:
    """A late subscriber gets exactly one retained latest value per topic."""
    messages = load(args.capture, args.node)
    retained = [m for m in messages if m.retained]
    topics = {m.topic for m in retained}
    check(
        "late subscriber receives one retained value per published topic",
        len(retained) == len(topics) and topics >= set(args.expect_topic),
        f"retained topics {sorted(topics)}",
    )
    for spec in args.expect_value or []:
        topic, expected = spec.split("=", 1)
        found = [m for m in retained if m.topic == topic]
        check(
            f"{topic}: retained value is {expected}",
            bool(found) and str(found[0].payload["value"]) == expected,
            found[0].raw if found else "no retained value",
        )
    for spec in args.expect_sequence or []:
        topic, expected = spec.split("=", 1)
        found = [m for m in retained if m.topic == topic]
        check(
            f"{topic}: retained sequence is {expected}",
            bool(found) and str(found[0].payload["sequence"]) == expected,
            found[0].raw if found else "no retained value",
        )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--node", required=True, help="edge-node-id under test")
    sub = parser.add_subparsers(dest="command", required=True)

    l1 = sub.add_parser("l1", help="minimal loop checks on the first capture")
    l1.add_argument("capture")
    l1.add_argument("--measurement", required=True)
    l1.add_argument("--state", required=True)
    l1.add_argument("--count", required=True)
    l1.add_argument("--min-count", type=int, default=3)
    l1.set_defaults(func=cmd_l1)

    shape = sub.add_parser("shape", help="every payload in a capture matches the contract")
    shape.add_argument("capture")
    shape.set_defaults(func=cmd_shape)

    continues = sub.add_parser("continues", help="series continue across an event")
    continues.add_argument("before")
    continues.add_argument("after")
    continues.add_argument("--allow-duplicate", action="store_true")
    continues.set_defaults(func=cmd_continues)

    retained = sub.add_parser("retained", help="retained values seen by a late subscriber")
    retained.add_argument("capture")
    retained.add_argument("--expect-topic", action="append", default=[])
    retained.add_argument("--expect-value", action="append")
    retained.add_argument("--expect-sequence", action="append")
    retained.set_defaults(func=cmd_retained)

    args = parser.parse_args()
    args.func(args)
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
