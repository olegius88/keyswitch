"""Typed CSR adapter and Python parity reference for the training-only kernel."""
from __future__ import annotations

import ctypes
import hashlib
import math
import shutil
import subprocess
import tempfile
from array import array
from dataclasses import dataclass
from pathlib import Path
from collections.abc import Iterable, Mapping

from keyswitch.context_model import softmax

ROOT = Path(__file__).resolve().parents[1]
SOURCE = Path(__file__).with_suffix(".c")
FLAGS = ("-O2", "-std=c99", "-shared", "-fPIC", "-ffp-contract=off", "-fno-fast-math")


@dataclass
class Packed:
    offsets: array[int]
    indices: array[int]
    values: array[float]
    labels: array[int]
    importance: array[float]

    @classmethod
    def build(cls, rows: Iterable[tuple[Mapping[str, float], int, float]], names: list[str]) -> Packed:
        vocabulary = {name: index for index, name in enumerate(names)}
        result = cls(array("Q", [0]), array("I"), array("d"), array("B"), array("d"))
        for features, label, importance in rows:
            if not 0 <= label < 4 or not math.isfinite(importance) or importance <= 0:
                raise ValueError("invalid training target or importance")
            for name, value in features.items():
                if not math.isfinite(value):
                    raise ValueError("non-finite feature")
                if name in vocabulary:
                    result.indices.append(vocabulary[name])
                    result.values.append(value)
            result.offsets.append(len(result.indices))
            result.labels.append(label)
            result.importance.append(importance)
        return result


def python_epoch(data: Packed, weights: array[float], accumulators: array[float], rate: float) -> None:
    for row, label in enumerate(data.labels):
        scores = [0.0] * 4
        for position in range(data.offsets[row], data.offsets[row + 1]):
            for action in range(4):
                scores[action] += weights[data.indices[position] * 4 + action] * data.values[position]
        probabilities = softmax(scores)
        for position in range(data.offsets[row], data.offsets[row + 1]):
            for action in range(4):
                index = data.indices[position] * 4 + action
                gradient = data.importance[row] * (probabilities[action] - float(action == label)) * data.values[position]
                accumulators[index] += gradient * gradient
                weights[index] -= rate * gradient / math.sqrt(accumulators[index])


class Kernel:
    def __init__(self, path: Path) -> None:
        self.library = ctypes.CDLL(str(path))
        self.library.context_epoch.restype = None
        self.library.context_epoch.argtypes = [ctypes.c_void_p] * 5 + [ctypes.c_uint64, ctypes.c_void_p, ctypes.c_void_p, ctypes.c_double]
        self.library.context_predict.restype = None
        self.library.context_predict.argtypes = [ctypes.c_void_p] * 3 + [ctypes.c_uint64, ctypes.c_void_p, ctypes.c_void_p]

    @classmethod
    def load(cls) -> Kernel:
        compiler = shutil.which("gcc") or shutil.which("cc")
        if compiler is None:
            raise RuntimeError("large-corpus training needs a C compiler; runtime and artifact validation do not")
        digest = hashlib.sha256(SOURCE.read_bytes() + repr(FLAGS).encode()).hexdigest()[:16]
        directory = ROOT / "build/context-optimizer" / digest
        directory.mkdir(parents=True, exist_ok=True)
        library = directory / "context.so"
        if not library.exists():
            with tempfile.TemporaryDirectory(dir=directory) as temporary:
                output = Path(temporary) / "context.so"
                subprocess.run([compiler, *FLAGS, "-o", str(output), str(SOURCE), "-lm"], check=True, capture_output=True)
                output.replace(library)
        return cls(library)

    def epoch(self, data: Packed, weights: array[float], accumulators: array[float], rate: float) -> None:
        self.library.context_epoch(data.offsets.buffer_info()[0], data.indices.buffer_info()[0], data.values.buffer_info()[0], data.labels.buffer_info()[0], data.importance.buffer_info()[0], len(data.labels), weights.buffer_info()[0], accumulators.buffer_info()[0], rate)

    def predict(self, data: Packed, weights: array[float]) -> array[float]:
        result = array("d", [0.0]) * (len(data.labels) * 4)
        self.library.context_predict(data.offsets.buffer_info()[0], data.indices.buffer_info()[0], data.values.buffer_info()[0], len(data.labels), weights.buffer_info()[0], result.buffer_info()[0])
        return result
