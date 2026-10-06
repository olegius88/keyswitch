"""The work of a context-v3 fit before its epochs, on the back end the caller chooses.

`train_context_action_model.fit` builds the same candidate on every back end, byte for byte:

- `cpu`: the curricula and the features of every frame by the trainer's own Python, in `jobs`
  worker processes forked from this one (with one job, in this process, one task after another);
- `gpu`: the curricula on the CPU workers as above, the evidence and features of the frames on
  CUDA (context_action_cuda.py), every frame CUDA cannot take by the same Python, and a sample of
  the frames recomputed on the CPU and compared, so that a kernel that drifted from the Python
  stops the fit instead of changing the candidate.

Every value that reaches the candidate comes from the trainer's functions or from operations
that keep it exact: frames, curricula and their reports are combined in the order the serial
fit used, feature masses are summed in that order (Kahan, in C), and the packed matrices hold
the same entries in the same order. NumPy only moves integers and copies floats here.
"""
from __future__ import annotations

import ctypes
import hashlib
import heapq
import inspect
import itertools
import json
import math
import os
import shutil
import subprocess
import sys
import tempfile
import time
from array import array
from collections.abc import Callable, Iterable, Iterator, Mapping, Sequence
from contextlib import contextmanager
from dataclasses import dataclass, field
from enum import IntEnum, auto
from functools import partial
from multiprocessing import get_context
from multiprocessing.pool import Pool
from pathlib import Path
from typing import Generic, Protocol, TypeVar, cast
from unittest.mock import patch

from keyswitch.constants.units import BYTES_PER_KIBIBYTE
from keyswitch.constants.training import (
    CONTEXT_ACTION_BACKEND_AUTO,
    CONTEXT_ACTION_BACKEND_CPU,
    CONTEXT_ACTION_BACKEND_GPU,
    CONTEXT_ACTION_BACKENDS,
    CONTEXT_ACTION_CUDA_MIRRORED_SOURCES,
    CONTEXT_ACTION_CUDA_MIRRORED_TRAINER_PARTS,
    CONTEXT_ACTION_CUDA_SPOT_CHECK_STRIDE,
    CONTEXT_ACTION_FRAMES_PER_TASK,
    CONTEXT_ACTION_MAIN_PROCESS_BYTES,
    CONTEXT_ACTION_MEMO_ENTRIES,
    CONTEXT_ACTION_PACKING_CHUNK_ENTRIES,
    CONTEXT_ACTION_REPORTED_DIFFERENCES,
    CONTEXT_ACTION_SCHEDULER_POLL_SECONDS,
    CONTEXT_OPTIMIZER_CACHE_DIGEST_CHARACTERS,
    CONTEXT_ACTION_TASKS_PER_WORKER,
    CONTEXT_ACTION_WORKER_BYTES,
    TRAINING_ACCELERATOR_SITE,
)

ROOT = Path(__file__).resolve().parents[1]
SITE = ROOT / TRAINING_ACCELERATOR_SITE
# tools/install-training-accelerators.sh puts NumPy (and, with --cuda, CuPy and the CUDA libraries)
# here; a package installed for the interpreter itself serves as well.
if SITE.is_dir() and str(SITE) not in sys.path:
    sys.path.insert(0, str(SITE))
    for toolkit in sorted((SITE / "nvidia").glob("cu*")):
        if (toolkit / "include").is_dir():
            os.environ.setdefault("CUDA_PATH", str(toolkit))

import numpy as np  # noqa: E402
import numpy.typing as npt  # noqa: E402

import train_context_action_model as trainer  # noqa: E402
from context_action_spans import SpanCurriculum, SpanFrame, build_span_curriculum  # noqa: E402
from context_optimizer import FLAGS, Packed  # noqa: E402
from train_context_model import CapturedSource  # noqa: E402
from keyswitch.constants.model_protocol import CALIBRATION, DEVELOPMENT, TRAIN  # noqa: E402
from keyswitch.context_action_features import extract_action_features  # noqa: E402
from keyswitch.context_model import ACTIONS  # noqa: E402
from keyswitch.language_model import LanguageModel  # noqa: E402
from keyswitch.ortho_model import OrthoModel  # noqa: E402


Frame = trainer.ActionRow | SpanFrame
Key = tuple[str, str]
FeatureRow = tuple[dict[str, float], int, float]
UInt32Array = npt.NDArray[np.uint32]
UInt8Array = npt.NDArray[np.uint8]
Int64Array = npt.NDArray[np.int64]
FloatArray = npt.NDArray[np.float64]
BoolArray = npt.NDArray[np.bool_]
A = TypeVar("A")
R = TypeVar("R")
R_co = TypeVar("R_co", covariant=True)


def log(message: str) -> None:
    print(message, flush=True)


# ---- choosing the back end ----------------------------------------------------------------------

def mirrored_hashes() -> dict[str, str]:
    """The hashes of the Python the CUDA kernels follow: whole files, and the trainer's per-frame functions."""
    result = {path: hashlib.sha256((ROOT / path).read_bytes()).hexdigest() for path in CONTEXT_ACTION_CUDA_MIRRORED_SOURCES}
    for name in CONTEXT_ACTION_CUDA_MIRRORED_TRAINER_PARTS:
        source = inspect.getsource(getattr(trainer, name))
        result["tools/train_context_action_model.py:" + name] = hashlib.sha256(source.encode()).hexdigest()
    return result


def parity_record(path: Path) -> dict[str, str] | None:
    try:
        payload = cast(dict[str, object], json.loads(path.read_text(encoding="utf-8")))
    except (OSError, ValueError):
        return None
    sources = payload.get("sources")
    return cast(dict[str, str], sources) if isinstance(sources, dict) else None


def write_parity_record(path: Path) -> None:
    path.write_text(json.dumps({"schema_version": 1, "sources": mirrored_hashes()}, indent=1, sort_keys=True) + "\n",
                    encoding="utf-8")


def cuda_status() -> tuple[bool, str]:
    """Whether the CUDA back end can run here, and why not."""
    parity = ROOT / "tools/context_action_kernels/parity.json"
    recorded = parity_record(parity)
    if recorded is None:
        return False, f"{parity.relative_to(ROOT)} is missing"
    changed = sorted(name for name, digest in mirrored_hashes().items() if recorded.get(name) != digest)
    if changed:
        return False, ("the kernels follow Python that changed since the last parity check (" + ", ".join(changed)
                       + "); run tools/context_action_cuda.py --refresh-parity on a CUDA machine after updating the kernels")
    try:
        import cupy
    except ImportError as error:
        return False, f"CuPy is not installed ({error}); see tools/install-training-accelerators.sh"
    try:
        devices = int(cupy.cuda.runtime.getDeviceCount())
    except cupy.cuda.runtime.CUDARuntimeError as error:
        return False, f"no CUDA device ({error})"
    return (True, "CUDA device found") if devices else (False, "no CUDA device")


def choose_backend(requested: str) -> str:
    if requested not in CONTEXT_ACTION_BACKENDS:
        raise ValueError(f"unknown back end {requested!r}")
    if requested == CONTEXT_ACTION_BACKEND_CPU:
        return CONTEXT_ACTION_BACKEND_CPU
    available, reason = cuda_status()
    if requested == CONTEXT_ACTION_BACKEND_GPU and not available:
        raise RuntimeError("the gpu back end cannot run: " + reason)
    if not available:
        log(f"back end: cpu ({reason})")
    return CONTEXT_ACTION_BACKEND_GPU if available else CONTEXT_ACTION_BACKEND_CPU


def available_memory() -> int | None:
    try:
        for line in Path("/proc/meminfo").read_text(encoding="ascii").splitlines():
            name, _, value = line.partition(":")
            if name == "MemAvailable":
                return int(value.split()[0]) * BYTES_PER_KIBIBYTE
    except (OSError, ValueError, IndexError):
        return None
    return None


def choose_jobs(requested: int | None) -> int:
    """Worker processes: as asked, or one per core as far as the available memory allows."""
    if requested is not None:
        if requested < 1:
            raise ValueError("jobs must be positive")
        return requested
    cores = len(os.sched_getaffinity(0)) if hasattr(os, "sched_getaffinity") else os.cpu_count() or 1
    memory = available_memory()
    if memory is None:
        return cores
    return max(1, min(cores, (memory - CONTEXT_ACTION_MAIN_PROCESS_BYTES) // CONTEXT_ACTION_WORKER_BYTES))


# ---- executors -----------------------------------------------------------------------------------

class Job(Protocol[R_co]):
    def ready(self) -> bool: ...

    def get(self) -> R_co: ...


class Done(Generic[R]):
    """A result computed at once, with the interface of a pool's AsyncResult."""

    def __init__(self, value: R) -> None:
        self.value = value

    def ready(self) -> bool:
        return True

    def get(self) -> R:
        return self.value


class Executor:
    """Worker processes forked from this one (they inherit `STATE`), or this process alone for one job."""

    def __init__(self, jobs: int) -> None:
        self.pool: Pool | None = get_context("fork").Pool(jobs) if jobs > 1 else None

    def submit(self, function: Callable[[A], R], argument: A) -> Job[R]:
        if self.pool is None:
            return Done(function(argument))
        return self.pool.apply_async(function, (argument,))

    def close(self, failed: bool = False) -> None:
        """Wait for the workers; after a failure, stop them at once."""
        if self.pool is not None:
            if failed:
                self.pool.terminate()
            else:
                self.pool.close()
            self.pool.join()


@dataclass(order=True)
class _Pending:
    priority: int
    sequence: int
    start: Callable[[], Callable[[], bool]] = field(compare=False)


class Scheduler:
    """Starts tasks in priority order (then in the order added), keeps at most `capacity` of them
    running and hands each result to its callback in this process; a callback may add tasks."""

    def __init__(self, executor: Executor, capacity: int) -> None:
        self.executor = executor
        self.capacity = capacity
        self.pending: list[_Pending] = []
        self.running: list[Callable[[], bool]] = []
        self.counter = itertools.count()

    def add(self, priority: int, function: Callable[[A], R], argument: A, done: Callable[[R], None]) -> None:
        def start() -> Callable[[], bool]:
            job = self.executor.submit(function, argument)

            def finish() -> bool:
                if not job.ready():
                    return False
                done(job.get())
                return True
            return finish
        heapq.heappush(self.pending, _Pending(priority, next(self.counter), start))

    def run(self) -> None:
        while self.pending or self.running:
            while self.pending and len(self.running) < self.capacity:
                self.running.append(heapq.heappop(self.pending).start())
            still = [finish for finish in self.running if not finish()]
            if len(still) == len(self.running):
                time.sleep(CONTEXT_ACTION_SCHEDULER_POLL_SECONDS)
            self.running = still


class Priority(IntEnum):
    """The TRAIN curricula are on the critical path, then the other curricula, the features the fit
    waits for, and last the features of frames computed ahead of their curricula."""
    CRITICAL = auto()
    CURRICULA = auto()
    NEEDED = auto()
    AHEAD = auto()


CRITICAL, CURRICULA, NEEDED, AHEAD = Priority.CRITICAL, Priority.CURRICULA, Priority.NEEDED, Priority.AHEAD


# ---- state of a fit and of its workers ------------------------------------------------------------

@dataclass
class State:
    """What every task reads; forked workers inherit it."""
    inputs: trainer.FitInputs
    keep_importance: float
    captured_sources: list[CapturedSource]


STATE: State | None = None


def state() -> State:
    if STATE is None:
        raise RuntimeError("no fit is running in this process")
    return STATE


def spelling_lexicons(inputs: trainer.FitInputs) -> dict[int, LanguageModel]:
    return inputs.lexicons[True]


def python_features(inputs: trainer.FitInputs, profile: str, split: str) -> Callable[[Frame], dict[str, float]]:
    detector = inputs.detectors[profile]

    def features(frame: Frame) -> dict[str, float]:
        return trainer.frame_features(frame, split, detector, inputs.ortho)
    return features


def frame_identifier(frame: Frame) -> str:
    return frame.identifier if isinstance(frame, trainer.ActionRow) else f"span:{frame.sequence_id}"


@contextmanager
def bound_process_state(inputs: trainer.FitInputs) -> Iterator[None]:
    """Bind, before any worker forks, the dictionaries a serial fit binds lazily, and remember pure results.

    The trainer's ortho model answers `known` from the engine's dictionaries, loaded at its first score.
    The engine's own ortho model (ContextPolicy._shared_ortho), which span frames replay through, binds
    its dictionaries once per process at its first score; in a serial fit that is inside the first span
    replay, where `LanguageModel.load` returns the first profile's lexicons, and every later replay, of
    either profile, keeps them. Both are bound here as a serial fit leaves them, once for all workers.

    Three pure functions remember their answers while the fit runs (exact: the same arguments give the
    same result): `one_typo_from_word` per model, `OrthoModel.score` without explicit dictionaries, and
    the intent model's FNV-1a hashes of n-gram names. Everything is restored on exit.
    """
    import keyswitch.context_policy as policy
    import keyswitch.intent_model as intent_module
    import keyswitch.ortho_model as ortho_module
    from functools import lru_cache
    from keyswitch.context_policy import ContextPolicy

    global STATE
    ortho = inputs.ortho
    saved_ortho = (ortho._known, ortho._identifier)
    saved_shared = ContextPolicy._shared_ortho
    # context_policy imports one_typo_from_word by name; that is the name its evidence calls.
    typo = cast(Callable[[str, LanguageModel], bool], getattr(policy, "one_typo_from_word"))
    score = OrthoModel.score
    fnv, signed_hash = intent_module.fnv1a64, intent_module.signed_feature_hash
    if ortho._known is None:
        ortho._known = ortho_module._engine_dictionaries()
    if ortho._identifier is None:
        ortho._identifier = ortho_module._packaged_identifier()
    shared = OrthoModel.try_load()
    first = inputs.lexical_models[inputs.profiles[0]]

    def first_profile(locale: str, extra_words: Iterable[str] = ()) -> LanguageModel:
        return first[0 if locale == "en_US" else 1]

    if shared[0] is not None:
        with patch("keyswitch.engine.LanguageModel.load", side_effect=first_profile):
            shared[0]._known = ortho_module._engine_dictionaries()
        shared[0]._identifier = ortho_module._packaged_identifier()
    ContextPolicy._shared_ortho = shared

    typo_answers: dict[tuple[str, int], tuple[LanguageModel, bool]] = {}

    def remembered_typo(text: str, model: LanguageModel) -> bool:
        found = typo_answers.get((text, id(model)))
        if found is None or found[0] is not model:
            if len(typo_answers) >= CONTEXT_ACTION_MEMO_ENTRIES:
                typo_answers.clear()
            found = typo_answers[text, id(model)] = (model, typo(text, model))
        return found[1]

    scores: dict[tuple[int, object], tuple[OrthoModel, object]] = {}

    def remembered_score(self: OrthoModel, evidence: ortho_module.OrthoEvidence, *, known: ortho_module.KnownWord | None = None,
                         ) -> ortho_module.OrthoScore:
        if known is not None:
            return score(self, evidence, known=known)
        found = scores.get((id(self), evidence))
        if found is None or found[0] is not self:
            if len(scores) >= CONTEXT_ACTION_MEMO_ENTRIES:
                scores.clear()
            found = scores[id(self), evidence] = (self, score(self, evidence))
        return cast(ortho_module.OrthoScore, found[1])

    setattr(policy, "one_typo_from_word", remembered_typo)
    OrthoModel.score = remembered_score  # type: ignore[method-assign]
    intent_module.fnv1a64 = lru_cache(maxsize=CONTEXT_ACTION_MEMO_ENTRIES)(fnv)
    intent_module.signed_feature_hash = lru_cache(maxsize=CONTEXT_ACTION_MEMO_ENTRIES)(signed_hash)
    from train_context_model import captured_sources
    STATE = State(inputs, float(cast(float, inputs.options["keep_importance"])),
                  captured_sources(ROOT / str(cast(dict[str, object], inputs.options["captured_curriculum"])["manifest"])))
    try:
        yield
    finally:
        STATE = None
        setattr(policy, "one_typo_from_word", typo)
        OrthoModel.score = score  # type: ignore[method-assign]
        intent_module.fnv1a64, intent_module.signed_feature_hash = fnv, signed_hash
        ContextPolicy._shared_ortho = saved_shared
        ortho._known, ortho._identifier = saved_ortho


# ---- tasks (run in the workers; module functions, so that a pool can name them) -------------------

def historical_task(_: None) -> list[trainer.ActionRow]:
    inputs = state().inputs
    return trainer.historical_curriculum(inputs.intent, inputs.lexicons[True])


def captured_task(index: int) -> tuple[list[trainer.ActionRow], dict[str, object]]:
    current = state()
    return trainer.captured_source_curriculum(current.captured_sources[index],
                                              cast(dict[str, object], current.inputs.options["captured_curriculum"]))


def capitals_task(_: None) -> tuple[list[trainer.ActionRow], dict[str, object]]:
    inputs = state().inputs
    return trainer.capital_citation_curriculum(inputs.source_rows[TRAIN], inputs.refused,
                                               cast(dict[str, object], inputs.options["capital_citation_curriculum"]),
                                               inputs.lexicons[False])


def chain_task(task: tuple[str, str, list[trainer.ActionRow] | None]) -> tuple[list[trainer.ActionRow], dict[str, object]]:
    """The curricula of one profile and split; TRAIN gets the frames the other curricula added."""
    profile, split, extra = task
    inputs = state().inputs
    rows = inputs.frames[split] if extra is None else trainer.training_order([*inputs.frames[split], *extra])
    return trainer.frame_chain(inputs, profile, split, rows)


def spans_task(key: Key) -> SpanCurriculum:
    profile, split = key
    inputs = state().inputs
    budgets = cast(dict[str, int], inputs.options["span_maximum_families"])
    return build_span_curriculum(inputs.source_rows[split], inputs.lexical_models[profile], profile=profile,
                                 maximum_families=budgets[split], expected_split=split)


@dataclass(frozen=True)
class FeatureTask:
    profile: str
    split: str
    source: str          # where the frames come from: "natural", "extra" or "fresh"
    start: int
    frames: list[Frame] | None   # None: the natural frames of the split this worker inherited


@dataclass(frozen=True)
class FeatureChunk:
    """The features of consecutive frames: names local to the chunk, their ids per entry, values and counts."""
    task: FeatureTask
    names: list[str]
    ids: array[int]
    values: array[float]
    counts: array[int]


def features_task(task: FeatureTask) -> FeatureChunk:
    current = state()
    inputs = current.inputs
    frames: Sequence[Frame] = (task.frames if task.frames is not None
                               else inputs.frames[task.split][task.start:task.start + CONTEXT_ACTION_FRAMES_PER_TASK])
    detector = inputs.detectors[task.profile]
    local: dict[str, int] = {}
    ids, values, counts = array("I"), array("d"), array("I")
    by_evidence: dict[str, dict[str, float]] = {}
    for frame in frames:
        if isinstance(frame, trainer.ActionRow):
            item = trainer.frame_evidence(frame, task.split, detector, inputs.ortho)
            # extract_action_features reads the evidence alone; its repr tells even 0.0 from -0.0.
            signature = repr(item)
            features = by_evidence.get(signature)
            if features is None:
                features = by_evidence[signature] = extract_action_features(item)
        else:
            features = extract_action_features(frame.evidence)
        for name, value in sorted(features.items()):
            identifier = local.get(name)
            if identifier is None:
                identifier = local[name] = len(local)
            ids.append(identifier)
            values.append(value)
        counts.append(len(features))
    return FeatureChunk(task, list(local), ids, values, counts)


# ---- featurised frames ------------------------------------------------------------------------------

def gather_index(starts: Int64Array, counts: Int64Array) -> Int64Array:
    """The entry positions of rows that start at `starts` and hold `counts` entries, row after row."""
    counts = counts.astype(np.int64)
    out_start = np.cumsum(counts) - counts
    return np.repeat(starts.astype(np.int64) - out_start, counts) + np.arange(int(counts.sum()), dtype=np.int64)


@dataclass
class Columns:
    """The featurised frames of one profile and split, in frame order: entries with global feature ids,
    then each frame's entry count, label, importance and sample weight."""
    ids: UInt32Array
    values: FloatArray
    counts: Int64Array
    labels: UInt8Array
    importance: FloatArray
    weights: FloatArray

    @classmethod
    def of(cls, ids: UInt32Array, values: FloatArray, counts: Int64Array, frames: Sequence[Frame],
           keep_importance: float) -> Columns:
        labels = np.fromiter((ACTIONS.index(frame.action) for frame in frames), dtype=np.uint8, count=len(frames))
        weights = np.fromiter((frame.sample_weight for frame in frames), dtype=np.float64, count=len(frames))
        # the serial fit: row.sample_weight * (keep_importance if label == 0 else 1.0)
        importance = weights * np.where(labels == 0, keep_importance, 1.0)
        return cls(ids.astype(np.uint32), values.astype(np.float64), counts.astype(np.int64), labels, importance, weights)

    def check(self, split: str, frames: Sequence[Frame]) -> None:
        """What the serial fit rejects: Packed.build's checks, and FeatureMass.add's on TRAIN sample weights."""
        if not (np.isfinite(self.importance).all() and (self.importance > 0).all()):
            raise ValueError("invalid training target or importance")
        if not np.isfinite(self.values).all():
            raise ValueError("non-finite feature")
        if split == TRAIN and any(type(frame.sample_weight) not in (int, float) or not math.isfinite(frame.sample_weight)
                                  or frame.sample_weight <= 0 for frame in frames):
            raise ValueError("feature sample weight must be positive and finite")

    def starts(self) -> Int64Array:
        return np.cumsum(self.counts) - self.counts

    def features(self, row: int, names: Sequence[str], start: int | None = None) -> dict[str, float]:
        """The features of one row; `start`, its first entry, saves summing the counts before it."""
        if start is None:
            start = int(self.counts[:row].sum())
        stop = start + int(self.counts[row])
        return {names[identifier]: value for identifier, value in
                zip(self.ids[start:stop].tolist(), self.values[start:stop].tolist())}

    def rows(self, names: Sequence[str]) -> Iterator[FeatureRow]:
        """The rows as feature_rows reads them back from a feature file."""
        ids, values = self.ids.tolist(), self.values.tolist()
        position = 0
        for count, label, importance in zip(self.counts.tolist(), self.labels.tolist(), self.importance.tolist()):
            yield ({names[ids[entry]]: values[entry] for entry in range(position, position + count)}, label, importance)
            position += count


def packed(parts: Sequence[Columns], column: Int64Array, sort: Callable[[Int64Array], Int64Array] | None = None) -> Packed:
    """Packed.build of the parts' rows for the vocabulary `column` (global id -> column, -1 if not selected).

    A row's entries must reach Packed in vocabulary order, the order of their names: so they arrive from
    the CPU workers. Entries in any other order are sorted per row, by `sort` (a stable argsort) if given.
    Rows are packed a few million entries at a time, so that the temporary arrays stay small.
    """
    result = Packed(array("Q", [0]), array("I"), array("d"), array("B"), array("d"))
    narrow = column.astype(np.int32)
    total = 0
    for part in parts:
        starts = part.starts()
        first = 0
        while first < len(part.counts):
            last = max(first + 1, int(np.searchsorted(starts, starts[first] + CONTEXT_ACTION_PACKING_CHUNK_ENTRIES, side="right")))
            low, high = int(starts[first]), int(starts[last - 1] + part.counts[last - 1])
            selected = narrow[part.ids[low:high]]
            kept = selected >= 0
            rows = np.repeat(np.arange(last - first, dtype=np.int32), part.counts[first:last])[kept]
            columns, values = selected[kept], part.values[low:high][kept]
            if not bool(((np.diff(columns) > 0) | (np.diff(rows) != 0)).all()):
                keys = rows.astype(np.int64) * (len(column) + 1) + columns
                order = sort(keys) if sort is not None else np.argsort(keys, kind="stable")
                columns, values = columns[order], values[order]
            result.offsets.frombytes((total + np.cumsum(np.bincount(rows, minlength=last - first))).astype(np.uint64).tobytes())
            total += len(columns)
            result.indices.frombytes(columns.astype(np.uint32).tobytes())
            result.values.frombytes(values.astype(np.float64).tobytes())
            first = last
        result.labels.frombytes(part.labels.astype(np.uint8).tobytes())
        result.importance.frombytes(part.importance.astype(np.float64).tobytes())
    return result


class MassKernel:
    """tools/context_action_mass.c, built with the optimizer's flags and kept in build/ by its digest."""

    def __init__(self) -> None:
        source = ROOT / "tools/context_action_mass.c"
        digest = hashlib.sha256(source.read_bytes() + repr(FLAGS).encode()).hexdigest()[:CONTEXT_OPTIMIZER_CACHE_DIGEST_CHARACTERS]
        library = ROOT / "build/context-action-mass" / digest / "mass.so"
        if not library.exists():
            compiler = shutil.which("gcc") or shutil.which("cc")
            if compiler is None:
                raise RuntimeError("the context-v3 trainer needs a C compiler")
            library.parent.mkdir(parents=True, exist_ok=True)
            with tempfile.TemporaryDirectory(dir=library.parent) as temporary:
                output = Path(temporary) / library.name
                subprocess.run([compiler, *FLAGS, "-o", str(output), str(source), "-lm"], check=True, capture_output=True)
                output.replace(library)
        self.library = ctypes.CDLL(str(library))
        pointer = ctypes.c_void_p
        self.library.context_action_feature_mass.restype = ctypes.c_int
        # ids, offsets, sample weights; rows; mass, compensation (both per feature id)
        self.library.context_action_feature_mass.argtypes = [pointer, pointer, pointer, ctypes.c_uint64, pointer, pointer]

    def add(self, ids: int, offsets: int, weights: int, rows: int, values: array[float], errors: array[float]) -> None:
        """FeatureMass.add for `rows` packed rows (addresses of uint32 ids, uint64 offsets, double weights),
        continuing the sums in `values` and `errors`, indexed by feature id."""
        if self.library.context_action_feature_mass(ids, offsets, weights, rows, values.buffer_info()[0], errors.buffer_info()[0]):
            raise ValueError("feature mass overflow")


@dataclass
class FeatureSet:
    """Every featurised frame of a fit, and what the seal reports about the curricula."""
    names: list[str]
    columns: dict[Key, Columns]
    profiles: list[str]
    chains: dict[Key, dict[str, object]]
    spans: dict[Key, dict[str, int]]
    captured: dict[str, object]
    capitals: dict[str, object]
    sort: Callable[[Int64Array], Int64Array] | None = None

    def masses(self) -> dict[str, float]:
        """FeatureMass over the TRAIN frames of every profile, in the order the serial fit adds them."""
        values = array("d", [0.0]) * len(self.names)
        errors = array("d", [0.0]) * len(self.names)
        present = np.zeros(len(self.names), dtype=bool)
        kernel = MassKernel()
        for profile in self.profiles:
            part = self.columns[profile, TRAIN]
            ids = np.ascontiguousarray(part.ids, dtype=np.uint32)
            offsets = np.zeros(len(part.counts) + 1, dtype=np.uint64)
            np.cumsum(part.counts, out=offsets[1:])
            weights = np.ascontiguousarray(part.weights, dtype=np.float64)
            kernel.add(ids.ctypes.data, offsets.ctypes.data, weights.ctypes.data, len(part.counts), values, errors)
            present[ids] = True
        return {self.names[identifier]: values[identifier] for identifier in np.flatnonzero(present).tolist()}

    def packed(self, names: Sequence[str]) -> tuple[Packed, dict[str, Packed], dict[str, Packed]]:
        column = np.full(len(self.names), -1, dtype=np.int64)
        index = {name: identifier for identifier, name in enumerate(self.names)}
        column[np.array([index[name] for name in names], dtype=np.int64)] = np.arange(len(names), dtype=np.int64)
        train = packed([self.columns[profile, TRAIN] for profile in self.profiles], column, self.sort)
        development = {profile: packed([self.columns[profile, DEVELOPMENT]], column, self.sort) for profile in self.profiles}
        calibration = {profile: packed([self.columns[profile, CALIBRATION]], column, self.sort) for profile in self.profiles}
        return train, development, calibration

    def rows(self, profile: str, split: str) -> Iterator[FeatureRow]:
        return self.columns[profile, split].rows(self.names)


class Vocabulary:
    """Global feature ids, in the order the names arrive."""

    def __init__(self) -> None:
        self.names: list[str] = []
        self.index: dict[str, int] = {}

    def ids(self, names: Sequence[str]) -> UInt32Array:
        result = np.empty(len(names), dtype=np.uint32)
        for position, name in enumerate(names):
            identifier = self.index.get(name)
            if identifier is None:
                identifier = self.index[name] = len(self.names)
                self.names.append(name)
            result[position] = identifier
        return result


class ProfileStore:
    """The featurised chunks of one profile, as they arrive; any frame is found by its source and number."""

    def __init__(self) -> None:
        self.first_row: dict[tuple[str, str, int], int] = {}
        self.ids: list[UInt32Array] = []
        self.values: list[FloatArray] = []
        self.counts: list[Int64Array] = []
        self.rows = 0

    def add(self, chunk: FeatureChunk, vocabulary: Vocabulary) -> None:
        mapping = vocabulary.ids(chunk.names)
        ids = mapping[np.frombuffer(chunk.ids, dtype=np.uint32)] if len(chunk.ids) else np.zeros(0, np.uint32)
        counts = np.frombuffer(chunk.counts, dtype=np.uint32).astype(np.int64)
        self.first_row[chunk.task.source, chunk.task.split, chunk.task.start] = self.rows
        self.ids.append(ids)
        self.values.append(np.frombuffer(chunk.values, dtype=np.float64).copy())
        self.counts.append(counts)
        self.rows += len(counts)

    def row(self, source: str, split: str, number: int) -> int:
        offset = number % CONTEXT_ACTION_FRAMES_PER_TASK
        return self.first_row[source, split, number - offset] + offset

    def gather(self, rows: Int64Array) -> tuple[UInt32Array, FloatArray, Int64Array]:
        ids = np.concatenate([np.zeros(0, np.uint32), *self.ids])
        values = np.concatenate([np.zeros(0, np.float64), *self.values])
        counts = np.concatenate([np.zeros(0, np.int64), *self.counts])
        starts = np.cumsum(counts) - counts
        index = gather_index(starts[rows], counts[rows])
        return ids[index], values[index], counts[rows]


# ---- the fit's work ---------------------------------------------------------------------------------

class Build:
    """Curricula and features of one fit on the chosen back end; `run` returns the FeatureSet."""

    def __init__(self, inputs: trainer.FitInputs, backend: str, jobs: int) -> None:
        self.inputs = inputs
        self.backend = backend
        self.jobs = jobs
        self.profiles = inputs.profiles
        self.keys = [(profile, split) for profile in self.profiles for split in inputs.frames]
        self.keep_importance = float(cast(float, inputs.options["keep_importance"]))
        self.started = time.monotonic()
        self.vocabulary = Vocabulary()
        self.stores = {profile: ProfileStore() for profile in self.profiles}
        self.historical: list[trainer.ActionRow] | None = None
        self.captured: dict[int, tuple[list[trainer.ActionRow], dict[str, object]]] = {}
        self.capitals: tuple[list[trainer.ActionRow], dict[str, object]] | None = None
        self.extra: list[trainer.ActionRow] = []
        self.chains: dict[Key, tuple[list[trainer.ActionRow], dict[str, object]]] = {}
        self.spans: dict[Key, SpanCurriculum] = {}
        self.entries: dict[Key, list[Frame]] = {}
        self.plans: dict[Key, list[tuple[str, int]]] = {}
        self.fresh: dict[Key, list[Frame]] = {}
        self.columns: dict[Key, Columns] = {}
        self.missing: dict[Key, set[tuple[str, str, int]]] = {}
        self.arrived: dict[str, set[tuple[str, str, int]]] = {profile: set() for profile in self.profiles}
        self.spot_checks: list[tuple[Key, list[int], FeatureChunk]] = []
        self.featurizer: object = None
        self.executor = Executor(jobs)
        self.scheduler = Scheduler(self.executor, jobs * CONTEXT_ACTION_TASKS_PER_WORKER)

    def note(self, message: str) -> None:
        log(f"{time.monotonic() - self.started:8.1f} {message}")

    def run(self) -> FeatureSet:
        sources = state().captured_sources
        scheduler = self.scheduler
        try:
            scheduler.add(CRITICAL, historical_task, None, self.historical_done)
            for index in range(len(sources)):
                scheduler.add(CRITICAL, captured_task, index, partial(self.captured_done, index))
            scheduler.add(CRITICAL, capitals_task, None, self.capitals_done)
            for key in self.keys:
                if key[1] != TRAIN:
                    scheduler.add(CURRICULA, chain_task, (key[0], key[1], None), partial(self.chain_done, key))
                scheduler.add(CURRICULA, spans_task, key, partial(self.spans_done, key))
            if self.backend == CONTEXT_ACTION_BACKEND_GPU:
                self.start_gpu()
            else:
                for profile in self.profiles:
                    for split, frames in self.inputs.frames.items():
                        for start in range(0, len(frames), CONTEXT_ACTION_FRAMES_PER_TASK):
                            scheduler.add(AHEAD, features_task, FeatureTask(profile, split, "natural", start, None), self.chunk_done)
            scheduler.run()
        except BaseException:
            self.executor.close(failed=True)
            raise
        self.executor.close()
        missing = [key for key in self.keys if key not in self.columns]
        if missing:
            raise RuntimeError(f"frames of {missing} were never featurised")
        names = self.vocabulary.names
        sort: Callable[[Int64Array], Int64Array] | None = None
        if self.featurizer is not None:
            import context_action_cuda

            featurizer = cast(context_action_cuda.Featurizer, self.featurizer)
            self.check_spots(featurizer.dictionary.names)
            featurizer.save_cache()
            names = featurizer.dictionary.names
            self.note(f"features ready on the GPU; Hunspell answered {featurizer.hunspell_asked} words, "
                      f"the CPU path took {featurizer.cpu_frames} frames")
            self.featurizer = None
            context_action_cuda.release_device_memory()
            sort = context_action_cuda.device_sort
        else:
            self.note(f"features ready on {self.jobs} processes")
        captured_report: dict[str, object] = {}
        for index in range(len(sources)):
            captured_report[sources[index].path.name] = self.captured[index][1]
        return FeatureSet(names, self.columns, self.profiles, {key: report for key, (_rows, report) in self.chains.items()},
                          {key: curriculum.counts for key, curriculum in self.spans.items()}, captured_report,
                          cast(tuple[list[trainer.ActionRow], dict[str, object]], self.capitals)[1], sort)

    # -- curricula --

    def historical_done(self, rows: list[trainer.ActionRow]) -> None:
        self.historical = rows
        self.extra_ready()

    def captured_done(self, index: int, result: tuple[list[trainer.ActionRow], dict[str, object]]) -> None:
        self.captured[index] = result
        self.extra_ready()

    def capitals_done(self, result: tuple[list[trainer.ActionRow], dict[str, object]]) -> None:
        self.capitals = result
        self.extra_ready()

    def extra_ready(self) -> None:
        """Once the historical, captured and capital curricula are in, the TRAIN chains can start."""
        sources = state().captured_sources
        if self.historical is None or self.capitals is None or len(self.captured) < len(sources):
            return
        captured = [row for index in range(len(sources)) for row in self.captured[index][0]]
        # The serial fit: training_order([*natural TRAIN, *historical, *captured, *capitals])
        self.extra = [*self.historical, *captured, *self.capitals[0]]
        self.note(f"historical, captured and capital curricula ready: {len(self.extra)} frames")
        for profile in self.profiles:
            key = (profile, TRAIN)
            self.scheduler.add(CRITICAL, chain_task, (profile, TRAIN, self.extra), partial(self.chain_done, key))
            if self.backend == CONTEXT_ACTION_BACKEND_CPU:
                for start in range(0, len(self.extra), CONTEXT_ACTION_FRAMES_PER_TASK):
                    task = FeatureTask(profile, TRAIN, "extra", start, list(self.extra[start:start + CONTEXT_ACTION_FRAMES_PER_TASK]))
                    self.scheduler.add(AHEAD, features_task, task, self.chunk_done)

    def chain_done(self, key: Key, result: tuple[list[trainer.ActionRow], dict[str, object]]) -> None:
        self.chains[key] = result
        self.frames_ready(key)

    def spans_done(self, key: Key, curriculum: SpanCurriculum) -> None:
        self.spans[key] = curriculum
        self.frames_ready(key)

    def frames_ready(self, key: Key) -> None:
        if key not in self.chains or key not in self.spans:
            return
        rows, report = self.chains[key]
        self.chains[key] = ([], report)
        entries = trainer.prepared_frames(rows, self.spans[key], key[1])
        self.entries[key] = [frame for _identifier, frame in entries]
        self.note(f"{key[0]}/{key[1]}: {len(entries)} frames, span frames={len(self.spans[key].frames)}")
        if self.backend == CONTEXT_ACTION_BACKEND_GPU:
            self.gpu_split_ready(key[1])
            return
        # Each final frame: one featurised ahead (the same frame, compared field by field), or a fresh one.
        natural = self.inputs.frames[key[1]]
        natural_at = {row.identifier: number for number, row in enumerate(natural)}
        extra_at = {row.identifier: number for number, row in enumerate(self.extra)} if key[1] == TRAIN else {}
        plan: list[tuple[str, int]] = []
        fresh: list[Frame] = []
        for frame in self.entries[key]:
            if isinstance(frame, trainer.ActionRow):
                number = natural_at.get(frame.identifier)
                if number is not None and natural[number] == frame:
                    plan.append(("natural", number))
                    continue
                number = extra_at.get(frame.identifier)
                if number is not None and self.extra[number] == frame:
                    plan.append(("extra", number))
                    continue
            plan.append(("fresh", len(fresh)))
            fresh.append(frame)
        self.plans[key], self.fresh[key] = plan, fresh
        needed = {(source, TRAIN if source == "extra" else key[1], number - number % CONTEXT_ACTION_FRAMES_PER_TASK)
                  for source, number in plan}
        self.missing[key] = needed - self.arrived[key[0]]
        self.note(f"{key[0]}/{key[1]}: {len(fresh)} frames featurised after the curricula")
        for start in range(0, len(fresh), CONTEXT_ACTION_FRAMES_PER_TASK):
            task = FeatureTask(key[0], key[1], "fresh", start, fresh[start:start + CONTEXT_ACTION_FRAMES_PER_TASK])
            self.scheduler.add(NEEDED, features_task, task, self.chunk_done)
        if not self.missing[key]:
            self.assemble(key)

    # -- features on the CPU --

    def chunk_done(self, chunk: FeatureChunk) -> None:
        task = chunk.task
        self.stores[task.profile].add(chunk, self.vocabulary)
        arrived = (task.source, task.split, task.start)
        self.arrived[task.profile].add(arrived)
        for key, missing in list(self.missing.items()):
            if key[0] == task.profile and arrived in missing:
                missing.discard(arrived)
                if not missing:
                    self.assemble(key)

    def assemble(self, key: Key) -> None:
        """Put the frames of a planned profile/split, whose features all arrived, into their final order."""
        del self.missing[key]
        store = self.stores[key[0]]
        rows = np.fromiter((store.row(source, TRAIN if source == "extra" else key[1], number) for source, number in self.plans[key]),
                           dtype=np.int64, count=len(self.plans[key]))
        ids, values, counts = store.gather(rows)
        frames = self.entries[key]
        columns = Columns.of(ids, values, counts, frames, self.keep_importance)
        columns.check(key[1], frames)
        self.columns[key] = columns
        del self.entries[key], self.plans[key], self.fresh[key]
        if all((key[0], split) in self.columns for split in self.inputs.frames):
            self.stores[key[0]] = ProfileStore()

    # -- features on the GPU --

    def start_gpu(self) -> None:
        """CUDA starts in this process once every worker has forked (the pool forked them all at its start)."""
        import context_action_cuda

        inputs = self.inputs
        first = inputs.lexical_models[self.profiles[0]]
        for profile in self.profiles:
            for group in first:
                if inputs.lexical_models[profile][group].frequencies != first[group].frequencies:
                    raise RuntimeError("the gpu back end shares one frequency table per locale between the profiles")
        check_engine_dictionaries(spelling_lexicons(inputs))
        featurizer = context_action_cuda.Featurizer(first, spelling_lexicons(inputs), inputs.intent, inputs.ortho, self.keep_importance)
        self.featurizer = featurizer
        self.note(f"CUDA featurizer ready ({featurizer.setup_seconds:.1f} s)")

    def gpu_split_ready(self, split: str) -> None:
        import context_action_cuda

        if any((profile, split) not in self.entries for profile in self.profiles):
            return
        featurizer = cast(context_action_cuda.Featurizer, self.featurizer)
        inputs = self.inputs
        result = featurizer.featurize_split({profile: self.entries[profile, split] for profile in self.profiles}, split,
                                            {profile: inputs.lexical_models[profile][0].speller.available for profile in self.profiles},
                                            {profile: python_features(inputs, profile, split) for profile in self.profiles})
        for profile, columns in result.items():
            frames = self.entries[profile, split]
            columns.check(split, frames)
            self.columns[profile, split] = columns
            # Every CONTEXT_ACTION_CUDA_SPOT_CHECK_STRIDE-th frame again on the CPU, compared once all are in.
            sample = list(range(0, len(frames), CONTEXT_ACTION_CUDA_SPOT_CHECK_STRIDE))
            for start in range(0, len(sample), CONTEXT_ACTION_FRAMES_PER_TASK):
                numbers = sample[start:start + CONTEXT_ACTION_FRAMES_PER_TASK]
                task = FeatureTask(profile, split, "check", start, [frames[number] for number in numbers])
                self.scheduler.add(AHEAD, features_task, task, partial(self.spot_checked, (profile, split), numbers))
        self.note(f"{split}: features on the GPU")

    def spot_checked(self, key: Key, numbers: list[int], chunk: FeatureChunk) -> None:
        self.spot_checks.append((key, numbers, chunk))

    def check_spots(self, names: Sequence[str]) -> None:
        checked = 0
        starts = {key: columns.starts() for key, columns in self.columns.items()}
        for key, numbers, chunk in self.spot_checks:
            columns = self.columns[key]
            ids, values, counts = chunk.ids.tolist(), chunk.values.tolist(), chunk.counts.tolist()
            position = 0
            for number, count in zip(numbers, counts):
                expected = {chunk.names[ids[entry]]: values[entry] for entry in range(position, position + count)}
                position += count
                actual = columns.features(number, names, int(starts[key][number]))
                if actual != expected:
                    frame = self.entries[key][number]
                    differing = sorted(set(actual.items()) ^ set(expected.items()))[:CONTEXT_ACTION_REPORTED_DIFFERENCES]
                    raise RuntimeError(f"the CUDA features of {key[0]}/{key[1]} frame {frame_identifier(frame)} differ from "
                                       f"the Python ones: {differing}; train with --backend cpu and update the kernels")
                checked += 1
        self.note(f"spot check: {checked} frames equal on the GPU and the CPU")


def check_engine_dictionaries(spelling: Mapping[int, LanguageModel]) -> None:
    """The kernels answer the ortho model's `known` from the reference lexicons with morphology; the trainer's
    ortho model asks the engine's dictionaries. They must hold the same tables and the same Hunspell dictionary."""
    from keyswitch.lexicon_supplement import supplement_words

    for group, locale in enumerate(("en_US", "ru_RU")):
        engine = LanguageModel.load(locale, supplement_words(locale))
        reference = spelling[group]
        if (engine.frequencies != reference.frequencies or engine.speller.source != reference.speller.source
                or engine._gram_counts != reference._gram_counts or engine.maximum != reference.maximum):
            raise RuntimeError("the engine's dictionaries differ from the reference lexicons; train with --backend cpu")


def build_features(inputs: trainer.FitInputs, backend: str, jobs: int) -> FeatureSet:
    with bound_process_state(inputs):
        return Build(inputs, backend, jobs).run()


# ---- frames for the parity check ---------------------------------------------------------------------

def parity_frames(inputs: trainer.FitInputs, limit: int | None) -> dict[str, list[Frame]]:
    """Frames of every kind a fit featurises, from the corpus's TRAIN and DEVELOPMENT splits: natural frames,
    the curricula of the first profile, the historical, captured and capital curricula, and span frames."""
    profile = inputs.profiles[0]
    current = state()
    extra = [*historical_task(None),
             *(row for index in range(len(current.captured_sources)) for row in captured_task(index)[0]),
             *capitals_task(None)[0]]
    result: dict[str, list[Frame]] = {}
    for split in (TRAIN, DEVELOPMENT):
        natural = inputs.frames[split][:limit] if limit is not None else inputs.frames[split]
        rows, _report = trainer.frame_chain(inputs, profile, split, [*natural, *(extra if split == TRAIN else [])])
        spans = spans_task((profile, split))
        frames: list[Frame] = [frame for _identifier, frame in trainer.prepared_frames(rows, spans, split)]
        result[split] = frames[:limit] if limit is not None else frames
    return result
