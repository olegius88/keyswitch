"""KeySwitch provenance from log headers, never from installed binaries or message text."""

from __future__ import annotations

import hashlib
import json
import re
import uuid
from datetime import datetime, timezone

VERSION = re.compile(r"[0-9]{1,6}\.[0-9]{1,6}\.[0-9]{1,6}(?:[-+][a-zA-Z0-9.-]{1,40})?")
HEADER_BYTES = 256
PROBE_BYTES = 1024 * 1024
RECORD_START = re.compile(rb"^\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2},\d{3} ", re.MULTILINE)
HEADER = re.compile(
    RECORD_START.pattern + rb"(?P<version>[0-9A-Za-z.+-]+) "
    rb"(?:DEBUG|INFO|WARNING|ERROR|CRITICAL) keyswitch(?:\.[A-Za-z0-9_]+)*: "
)
MARKER_KIND = "keyswitch.version"
TIMESTAMP_SHAPE = b"0000-00-00 00:00:00,000 "


def header_version(data: bytes) -> str | None:
    match = HEADER.match(data)
    if match:
        version = match["version"].decode("ascii")
        if VERSION.fullmatch(version):
            return version
    return None


def latest_version(stream) -> str | None:
    """Bounded tail probe; an unrecognized last record is not assigned an old version."""
    size = stream.seek(0, 2)
    start = max(0, size - PROBE_BYTES)
    stream.seek(start)
    data = stream.read(PROBE_BYTES)
    if start:
        # The first bytes may be the middle of an arbitrarily long log message.
        newline = data.find(b"\n")
        data = data[newline + 1 :] if newline >= 0 else b""
    matches = list(RECORD_START.finditer(data))
    return header_version(data[matches[-1].start() :][:HEADER_BYTES]) if matches else None


def take_fragment(data: bytes, previous: str | None, line_start: bool, lookahead: bytes):
    """Return a single-version prefix, preserving exact bytes including partial UTF-8.

    Lookahead only identifies the bounded header; it is not consumed. Cursor
    context survives a chunk/restart in the middle of a record or traceback.
    An unfinished header waits for more bytes instead of inheriting an old version.
    """
    combined = data + lookahead
    version = previous
    offset = 0
    while offset < len(data):
        if line_start:
            prefix = combined[offset : offset + HEADER_BYTES]
            found = header_version(prefix)
            if found is None and b"\n" not in prefix and len(prefix) < HEADER_BYTES:
                # Even a single digit can be the start of the next version's
                # timestamp. Do not consume it before the writer finishes the header.
                if all(
                    48 <= char <= 57 if expected == 48 else char == expected
                    for char, expected in zip(prefix, TIMESTAMP_SHAPE)
                ):
                    return data[:offset], version
            candidate = found if found is not None or RECORD_START.match(prefix) else version
            if offset and candidate != version:
                return data[:offset], version
            version = candidate
        newline = data.find(b"\n", offset)
        if newline < 0:
            break
        offset, line_start = newline + 1, True
    return data, version


def current_versions(store, config) -> dict:
    values = store.get("keyswitch_versions:" + config.destination, {})
    # Removed sources and a previous collector's sources are not current inputs.
    return {source.id: values[source.id] for source in config.sources if source.id in values}


def observe_version(store, config, source, version: str) -> None:
    key = "keyswitch_versions:" + config.destination
    values = current_versions(store, config)
    previous = values.get(source.id, {}).get("version")
    if previous == version:
        return
    identifier = uuid.uuid4().hex
    created = datetime.now(timezone.utc).isoformat()
    state = {
        "version": version,
        "previous_version": previous,
        "marker_id": identifier,
        "detected_at": created,
        "source_label": source.label,
    }
    metadata = {
        "schema": 1,
        "kind": MARKER_KIND,
        "bundle_id": identifier,
        "device_id": config.device_id,
        "device_name": config.device_name,
        "source_id": source.id,
        "source_label": source.label,
        "created_at": created,
        "keyswitch_version": version,
        "previous_version": previous,
    }
    payload = json.dumps(metadata, ensure_ascii=False, indent=2).encode()
    metadata.update(size=len(payload), sha256=hashlib.sha256(payload).hexdigest())
    values[source.id] = state
    # Both the marker and current pointer commit together, even when the queue fills.
    store.enqueue_event(
        config.destination, metadata, payload, f"lc-version-{identifier}.json", {key: values}
    )


def caption(metadata: dict) -> str:
    version = metadata.get("keyswitch_version")
    if metadata.get("kind") == MARKER_KIND:
        previous = metadata.get("previous_version")
        title = (
            f"KeySwitch: смена версии {previous} → {version}"
            if previous
            else f"KeySwitch: определена версия {version}"
        )
        return (
            f"МАРКЕР ВЕРСИИ · {title}\n"
            f"{metadata['device_name']} · {metadata['source_label']}\n"
            "Для анализа — только архивы с этой версией. Старая очередь может прийти позже; "
            "используйте фильтр версии, а не положение сообщения в чате.\n"
            f"{metadata['created_at']}\nID: {metadata['bundle_id']}"
        )
    label = f"KeySwitch {version}" if version else "версия KeySwitch не определена"
    return (
        f"LogCourier · {metadata['device_name']} · {metadata['source_label']}\n"
        f"{label}\n{metadata['created_at']}\nID: {metadata['bundle_id']}"
    )
