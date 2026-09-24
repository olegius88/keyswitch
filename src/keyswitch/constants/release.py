"""Release pipeline, packaging, APT and verification limits and timeouts."""

from __future__ import annotations

from typing import Final

from .units import BYTES_PER_MEBIBYTE, SECONDS_PER_HOUR, SECONDS_PER_MINUTE

APT_GZIP_COMPRESSION_LEVEL: Final = 9
# the plain index and its .gz
APT_INDEX_FILE_VARIANTS: Final = 2
# Geometry and colours of the Windows application icon (.ico).
APP_ICON_SIZE_PIXELS: Final = 256
APP_ICON_BACKGROUND_BOX_PIXELS: Final = (8, 8, 247, 247)
APP_ICON_BACKGROUND_RADIUS_PIXELS: Final = 56
APP_ICON_BACKGROUND_RGBA: Final = (35, 92, 190, 255)
APP_ICON_BORDER_BOX_PIXELS: Final = (31, 31, 224, 224)
APP_ICON_BORDER_RADIUS_PIXELS: Final = 40
APP_ICON_BORDER_RGBA: Final = (255, 255, 255, 70)
APP_ICON_BORDER_WIDTH_PIXELS: Final = 5
APP_ICON_LABEL_FONT_SIZE_POINTS: Final = 86
APP_ICON_EXPORT_SIZES_PIXELS: Final = ((16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256))
# The release workflow run must appear on GitHub within this long.
CI_APPEARANCE_TIMEOUT_SECONDS: Final[float] = 180.0
CI_POLL_SECONDS: Final[float] = 5.0
# Characters of a git commit SHA shown in release messages.
COMMIT_SHA_PREVIEW_CHARACTERS: Final[int] = 12
DEFAULT_CI_TIMEOUT_SECONDS: Final[float] = 2400.0
# The shell convention for "killed by signal N" is 128 + N; SIGINT is 2.
SIGINT_EXIT_CODE: Final[int] = 130
# Numbers of the steps release() announces.
STEP_CHECK_TREE: Final[int] = 1
STEP_APPLY_VERSION: Final[int] = 2
STEP_CLOSE_CHANGELOG: Final[int] = 3
STEP_CHECK_RELEASE_NOTES: Final[int] = 4
STEP_RUN_CONTOUR: Final[int] = 5
STEP_COMMIT_TAG_PUSH: Final[int] = 6
STEP_WAIT_FOR_WORKFLOW: Final[int] = 7
STEP_VERIFY_RELEASE: Final[int] = 8
# How many steps release() announces in total: a dry run stops after the release notes, --skip-ci
# after the push, a full run after verification.
STEP_COUNT_DRY_RUN: Final[int] = STEP_CHECK_RELEASE_NOTES
STEP_COUNT_SKIP_CI: Final[int] = STEP_COMMIT_TAG_PUSH
STEP_COUNT_FULL: Final[int] = STEP_VERIFY_RELEASE
PIPELINE_STATE_SCHEMA_VERSION: Final[int] = 1
# Largest JSON file the release pipeline reads.
PIPELINE_JSON_READ_LIMIT_BYTES: Final[int] = 64 * BYTES_PER_MEBIBYTE
# One trainer replay holds ~8 GiB in the parent and forks one worker per CPU in its feature/scoring
# phases; two replays in parallel plus a desktop exhausted a 32 GiB host on 2026-09-02, so replays
# run one at a time under this budget.
REPLAY_MEMORY_MIB: Final[int] = 16000
REPLAY_POLL_SECONDS: Final[int] = 30
SCHEDULER_POLL_SECONDS: Final[float] = 5.0
TERMINATE_GRACE_SECONDS: Final[int] = 30
DEFAULT_MEMORY_RESERVE_MIB: Final[int] = 2048
# Memory page size assumed where os.sysconf is unavailable.
DEFAULT_PAGE_SIZE_BYTES: Final[int] = 4096
# Indices into `stat.rpartition(")")[2].split()`: the /proc/pid/stat fields that follow the
# parenthesised command name (see proc(5)), 0-based.
PROC_STAT_SESSION_INDEX: Final[int] = 3
PROC_STAT_RSS_PAGES_INDEX: Final[int] = 21
PROC_STAT_MIN_FIELDS_AFTER_COMM: Final[int] = PROC_STAT_RSS_PAGES_INDEX + 1
DEFAULT_LOG_TAIL_LINES: Final[int] = 40
COMMAND_PREVIEW_ARGUMENT_COUNT: Final[int] = 2
# Per-phase subprocess timeouts.
DEVELOPMENT_REPLAY_TIMEOUT_SECONDS: Final[int] = 3 * SECONDS_PER_HOUR
PRESEAL_REPLAY_TIMEOUT_SECONDS: Final[int] = 3 * SECONDS_PER_HOUR
STRICT_EVALUATION_TIMEOUT_SECONDS: Final[int] = 4 * SECONDS_PER_HOUR
MODEL_REPLAYS_DEADLINE_SECONDS: Final[int] = 8 * SECONDS_PER_HOUR
COVERAGE_TIMEOUT_SECONDS: Final[int] = 2 * SECONDS_PER_HOUR
DETECTOR_GATES_TIMEOUT_SECONDS: Final[int] = 2 * SECONDS_PER_HOUR
BUILD_DEB_TIMEOUT_SECONDS: Final[int] = 6 * SECONDS_PER_HOUR
DEFAULT_PHASE_TIMEOUT_SECONDS: Final[int] = 30 * SECONDS_PER_MINUTE
QUICK_VERIFICATION_TIMEOUT_SECONDS: Final[int] = 10 * SECONDS_PER_MINUTE
REQUIRED_COVERAGE_PERCENT: Final[int] = 100
QUICK_CHECK_TIMEOUT_SECONDS: Final[int] = 5 * SECONDS_PER_MINUTE
MODEL_REPLAYS_BASE_MEMORY_MIB: Final[int] = 300
SAVE_RETRY_ATTEMPTS: Final[int] = 3
SAVE_RETRY_BACKOFF_SECONDS: Final[float] = 0.05
PHASE_DURATION_DECIMALS: Final[int] = 3
MAX_MEMORY_PRESSURE_EVENTS: Final[int] = 50
ALREADY_RUNNING_EXIT_CODE: Final[int] = 2
STILL_RUNNING_EXIT_CODE: Final[int] = 3
LOG_LINE_PREVIEW_CHARACTERS: Final[int] = 160
MAX_DEFAULT_JOBS: Final[int] = 4
CPUS_PER_DEFAULT_JOB: Final[int] = 3
MAX_REPLAYS: Final[int] = 2
MINIMUM_WAIT_POLL_SECONDS: Final[int] = 5
TYPECHECK_TIMEOUT_SECONDS: Final[int] = SECONDS_PER_HOUR
DEFAULT_WAIT_POLL_SECONDS: Final[int] = SECONDS_PER_MINUTE
# The PHASES table of tools/release_pipeline.py lists, per phase, a timeout, the duration the
# scheduler expects (it starts the longest ready phase first) and a memory estimate. A phase whose
# runner enforces a single command timeout is listed with that timeout's constant above; only the
# timeouts that are not a command timeout are named here.
# model-replays runs its replays one after another, each under its own deadline.
PHASE_MODEL_REPLAYS_TIMEOUT_SECONDS: Final[int] = MAX_REPLAYS * MODEL_REPLAYS_DEADLINE_SECONDS
# release-metadata is listed with this budget; its only command (git diff --check) has the
# QUICK_CHECK_TIMEOUT_SECONDS timeout.
PHASE_RELEASE_METADATA_TIMEOUT_SECONDS: Final[int] = 10 * SECONDS_PER_MINUTE
PHASE_ENVIRONMENT_EXPECTED_SECONDS: Final[int] = 60
PHASE_MODEL_INPUTS_EXPECTED_SECONDS: Final[int] = 20
PHASE_MODEL_DEVELOPMENT_REPLAY_EXPECTED_SECONDS: Final[int] = 400
PHASE_MODEL_PRESEAL_REPLAY_EXPECTED_SECONDS: Final[int] = 400
PHASE_MODEL_STRICT_EXPECTED_SECONDS: Final[int] = 900
PHASE_MODEL_REPLAYS_EXPECTED_SECONDS: Final[int] = 4 * SECONDS_PER_HOUR
PHASE_MODEL_REPLAY_STRICT_EXPECTED_SECONDS: Final[int] = 900
PHASE_TYPECHECK_EXPECTED_SECONDS: Final[int] = 120
PHASE_COVERAGE_EXPECTED_SECONDS: Final[int] = 180
PHASE_DETECTOR_GATES_EXPECTED_SECONDS: Final[int] = 600
PHASE_E2E_X11_EXPECTED_SECONDS: Final[int] = 120
PHASE_E2E_TRAY_EXPECTED_SECONDS: Final[int] = 60
PHASE_BUILD_DEB_EXPECTED_SECONDS: Final[int] = 600
PHASE_VERIFY_DEB_EXPECTED_SECONDS: Final[int] = 120
PHASE_E2E_NATIVE_EXPECTED_SECONDS: Final[int] = 120
PHASE_RELEASE_METADATA_EXPECTED_SECONDS: Final[int] = 10
# Memory estimates are conservative peaks: observed PSS on the reference host on 2026-09-02 plus
# headroom. Every run records observed_peak_rss_mib per phase so they can be recalibrated.
# model-replays is estimated with REPLAY_MEMORY_MIB.
PHASE_ENVIRONMENT_MEMORY_MIB: Final[int] = 300
PHASE_MODEL_INPUTS_MEMORY_MIB: Final[int] = 300
PHASE_MODEL_DEVELOPMENT_REPLAY_MEMORY_MIB: Final[int] = 1200
PHASE_MODEL_PRESEAL_REPLAY_MEMORY_MIB: Final[int] = 1200
PHASE_MODEL_STRICT_MEMORY_MIB: Final[int] = 9500
PHASE_MODEL_REPLAY_STRICT_MEMORY_MIB: Final[int] = 4000
PHASE_TYPECHECK_MEMORY_MIB: Final[int] = 600
PHASE_COVERAGE_MEMORY_MIB: Final[int] = 800
PHASE_DETECTOR_GATES_MEMORY_MIB: Final[int] = 400
PHASE_E2E_X11_MEMORY_MIB: Final[int] = 500
PHASE_E2E_TRAY_MEMORY_MIB: Final[int] = 200
PHASE_BUILD_DEB_MEMORY_MIB: Final[int] = 4000
PHASE_VERIFY_DEB_MEMORY_MIB: Final[int] = 300
PHASE_E2E_NATIVE_MEMORY_MIB: Final[int] = 600
PHASE_RELEASE_METADATA_MEMORY_MIB: Final[int] = 200
