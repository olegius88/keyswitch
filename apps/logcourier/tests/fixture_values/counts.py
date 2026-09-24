"""Counts, sizes and lengths the tests expect or feed."""

from __future__ import annotations

from typing import Final

# Generous default so FakeTelegram.download() never trips over test payloads.
FAKE_DOWNLOAD_LIMIT_BYTES: Final = 10 * 1024 * 1024
# Filler length for a path that must exceed autostart.WINDOWS_MAX_PATH_CHARACTERS.
OVER_LONG_PATH_FILLER_CHARACTERS: Final = 270
# Fixture fragment counts: the default scanned-and-compacted batch, and a smaller one for the tests
# that look at partial batching.
FRAGMENT_COUNT: Final = 32
SMALL_FRAGMENT_COUNT: Final = 5
# One data package upload plus one catalog upload, not one document per fragment.
UPLOADS_PER_DELIVERY: Final = 2
COMPACTION_SIZE_LIMIT_BYTES: Final = 6000
# The size-limited compaction above must still split the fragments into at least this many batches.
MIN_COMPACTED_BATCHES: Final = 2
# Two archived files delivered across two scan-and-deliver cycles.
EXPECTED_ENTRIES: Final = 2
# Deliberately smaller than CHUNK_BYTES so a short payload still spans several scans.
SMALL_CHUNK_SIZE_BYTES: Final = 11
# Comfortably above what a single test payload can queue, to sidestep QueueFull.
RELAXED_QUEUE_CAPACITY_BYTES: Final = 10000
TAB_COUNT: Final = 4
FAKE_BOT_TOKEN_SECRET_CHARACTERS: Final = 30
# Hex-length fixtures for a fabricated catalog entry.
FAKE_HEX_ID_LENGTH: Final = 32
DOWNLOAD_FIXTURE_BYTES: Final = 3
SMALL_LIMIT_BYTES: Final = 4
FOUND_REDIRECT_STATUS: Final = 302
# Generous loop bounds for collect_all/deliver_all: high enough that the real stopping condition
# always fires first, low enough to fail fast if it never does.
SAFETY_LOOP_LIMIT: Final = 1000
# Parametrized read chunk sizes: pathologically small, small, and effectively unbounded.
CHUNK_SIZES: Final = (7, 71, 100000)
# A chunk size small enough that Collector.scan(max_chunks=1) stops after one chunk, leaving further
# sources/rotations unread until the next scan.
PARTIAL_SCAN_CHUNK_SIZE: Final = 80
# Arbitrary backlog size for an old rotated file that must not be read first.
BACKLOG_LINE_COUNT: Final = 20
# 0.16.1 -> 0.16.2 -> 0.16.2 -> 0.16.1 is three transitions; the repeated 0.16.2 in the middle is
# not marked again.
RESTART_MARKER_COUNT: Final = 3
# A queue capacity comfortably larger than anything this test writes.
SUFFICIENT_CAPACITY_BYTES: Final = 100000
# Splits a generated log line mid-header, so a restart is seen to preserve the wait.
HEADER_SPLIT_OFFSET: Final = 33
# A chunk size smaller than one log line, forcing multi-chunk partial reads.
TINY_CHUNK_SIZE: Final = 9
# How many times a fixture log line is duplicated, to produce that many separate fragments/entries.
TWO_COPIES: Final = 2
# A chunk size chosen to split several duplicated source lines across chunks while exercising
# compaction.
COMPACTION_TEST_CHUNK_SIZE: Final = 70
# marker + data + catalog uploads for one delivery that includes a fresh version marker.
UPLOADS_WITH_VERSION_MARKER: Final = 3
# All catalog entries survive dropping a source from config; only "current" narrows.
ALL_ENTRIES_AFTER_SOURCE_REMOVED: Final = 2
ALL_ENTRIES_AFTER_THREE_DELIVERIES: Final = 3
TWO_DISTINCT_SELECTIONS: Final = 2
# A deliberately wrong (non-string) "version" field value.
INVALID_VERSION_TYPE_VALUE: Final = 3
# Unfinished header lengths that must not let the old version leak through.
UNFINISHED_HEADER_LENGTHS: Final = (1, 4, 8, 23, 33)
# Past PROBE_BYTES by this margin, to be sure the probe window is exceeded.
PAST_PROBE_MARGIN_BYTES: Final = 10
FILL_BUDGET_LINE_COUNT: Final = 100
TWO_FRAGMENT_ENTRIES: Final = 2
TWO_DOWNLOADED_ZIPS: Final = 2
FIXTURE_FILE_SIZE: Final = 10
# Counts whose last digits pick each Russian noun form, teens included, for the russian_text tests.
RUSSIAN_QUANTITY_SAMPLE_COUNTS: Final = (1, 2, 5, 11, 12, 21, 22, 25, 111)
# A whole float is written without a fraction part; a fraction is written with a decimal comma.
RUSSIAN_QUANTITY_SAMPLE_WHOLE_FLOAT: Final = 3.0
RUSSIAN_QUANTITY_SAMPLE_FRACTION: Final = 2.5
