"""Release fixtures of the tests: fake versions, sizes, timeouts and pipeline numbers."""

from __future__ import annotations

from datetime import datetime, timezone
from typing import Final

from keyswitch.updates import SemanticVersion

# An arbitrary build timestamp used to check the .deb package's Date/Release stanza formatting.
APT_REPOSITORY_BUILD_TIMESTAMP: Final = datetime(2026, 9, 22, 10, 0, 0, tzinfo=timezone.utc)
# Shape of the fake `dpkg-deb --field <deb> <field>` command: its fixed head and where the field
# name sits.
DPKG_DEB_FIELD_COMMAND_PREFIX_ARGUMENTS: Final = 2
DPKG_DEB_STANZA_COMMAND_ARGUMENTS: Final = 3
DPKG_DEB_FIELD_NAME_ARGUMENT_INDEX: Final = 3
PIPELINE_SAMPLE_REPLAYS: Final = 2
PIPELINE_SAMPLE_JOBS: Final = 2
PIPELINE_SAMPLE_MEMORY_RESERVE_MIB: Final = 1024
PIPELINE_SAMPLE_INT_VALUE: Final = 3
PIPELINE_SAMPLE_OPTIONAL_NUMBER_INPUT: Final = 2
PIPELINE_SAMPLE_OPTIONAL_NUMBER_EXPECTED: Final = 2.0
PIPELINE_TYPECHECK_DURATION_SECONDS: Final = 12.5
PIPELINE_TYPECHECK_PEAK_RSS_MIB: Final = 300
PIPELINE_COVERAGE_DURATION_SECONDS: Final = 3661
PIPELINE_COVERAGE_PEAK_RSS_MIB: Final = 400
PIPELINE_COVERAGE_TEST_COUNT: Final = 346
PIPELINE_SECOND_JSON_VALUE: Final = 2
RELEASE_SCRIPT_PIPELINE_FAILURE_RETURN_CODE: Final = 3
RELEASE_SCRIPT_TARGET_RUN_ID: Final = 2
SAMPLE_RELEASE_ASSET_SIZE_BYTES: Final = 3
WRONG_RELEASE_ASSET_SIZE: Final = 2
NON_STRING_RELEASE_BODY: Final = 123
FAKE_DOWNLOAD_FIRST_PROGRESS_DONE: Final = 5
FAKE_DOWNLOAD_SECOND_PROGRESS_DONE: Final = 20
FAKE_DOWNLOAD_SECOND_PROGRESS_TOTAL: Final = 10
# A semantic version whose three fields differ, so a components mix-up in formatting or comparison
# would show up.
UPDATE_SAMPLE_VERSION: Final = SemanticVersion(2, 3, 4)
# The KSLM schema of the shipped intent model: the release pipeline must read this one back.
EXPECTED_KSLM_SCHEMA_VERSION: Final = 4
