#!/usr/bin/env python3
"""Generate the model-blind unknown-typo development/holdout receipt."""

from __future__ import annotations

import argparse
import json
import re
from collections.abc import Sequence
from dataclasses import dataclass
from pathlib import Path
from typing import cast

import evaluate_intent_model as evaluator
from keyswitch.language_model import LanguageModel
from train_intent_model import (
    PRESEALED_SPLITS,
    PROJECT_ROOT,
    SAFETY_COLLISION_MINIMUM_WORD_LENGTH,
    SEALED_TEST_SPLITS,
    SPLIT_NAMESPACE,
    TrainingConfig,
    HardNegativeDevelopmentCorpus,
    UNKNOWN_TYPO_DEVELOPMENT_CHOICE_NAMESPACE,
    UNKNOWN_TYPO_DEVELOPMENT_RANK_NAMESPACE,
    UNKNOWN_TYPO_HOLDOUT_CHOICE_NAMESPACE,
    UNKNOWN_TYPO_HOLDOUT_RANK_NAMESPACE,
    assert_no_split_leakage,
    build_dataset,
    load_onboard_unigrams,
    load_training_config,
    load_hard_negative_development_corpus,
    merge_sealed_test_dataset,
    prepare_lexicon,
)


@dataclass(frozen=True)
class ModelBlindExternalCorpora:
    """Unscored development/holdout corpora and their exclusion evidence."""

    config: TrainingConfig
    development: evaluator.LexicalDisjointCorpus
    holdout: evaluator.LexicalDisjointCorpus
    sealed_index: evaluator.SealedSignatureIndex
    development_signatures: frozenset[str]
    holdout_signatures: frozenset[str]
    combined_exclusions: frozenset[str]
    frozen_development: HardNegativeDevelopmentCorpus | None


def _parse_arguments(argv: Sequence[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--config",
        type=Path,
        default=Path("model/intent_v1/config.json"),
    )
    parser.add_argument(
        "--en-model",
        type=Path,
        default=Path("model/intent_v1/sources/en_US.lm"),
    )
    parser.add_argument(
        "--ru-model",
        type=Path,
        default=Path("model/intent_v1/sources/ru_RU.lm"),
    )
    return parser.parse_args(argv)


def _release_name() -> str:
    match = re.fullmatch(
        r"keyswitch:intent-(v[1-9][0-9]*):physical-signature",
        SPLIT_NAMESPACE,
    )
    if match is None:
        raise ValueError("split namespace does not contain a release version")
    return match.group(1)


def build_model_blind_external_corpora(
    config_path: Path,
    english_path: Path,
    russian_path: Path,
    *,
    verify_frozen_source: bool = True,
) -> ModelBlindExternalCorpora:
    """Build both corpora without loading, receiving, or scoring a model."""

    config = load_training_config(config_path)
    english, _english_source = load_onboard_unigrams(
        english_path,
        "en_US",
        0,
        config,
        license_declaration=config.sources.license_declaration,
        license_evidence=config.sources.license_evidence.path,
        logical_path=config.sources.english.path,
        minimum_word_length=SAFETY_COLLISION_MINIMUM_WORD_LENGTH,
    )
    russian, _russian_source = load_onboard_unigrams(
        russian_path,
        "ru_RU",
        1,
        config,
        license_declaration=config.sources.license_declaration,
        license_evidence=config.sources.license_evidence.path,
        logical_path=config.sources.russian.path,
        minimum_word_length=SAFETY_COLLISION_MINIMUM_WORD_LENGTH,
    )
    prepared = prepare_lexicon(
        (*english, *russian),
        minimum_training_signature_length=config.minimum_word_length,
    )
    onboard_words = {
        0: {
            word.word
            for word in english
            if len(word.physical_signature) >= config.minimum_word_length
        },
        1: {
            word.word
            for word in russian
            if len(word.physical_signature) >= config.minimum_word_length
        },
    }
    candidate = build_dataset(
        prepared, config, included_splits=PRESEALED_SPLITS
    )
    assert_no_split_leakage(candidate)
    sealed = build_dataset(
        prepared, config, included_splits=SEALED_TEST_SPLITS
    )
    assert_no_split_leakage(
        sealed,
        variant_quarantine_splits=SEALED_TEST_SPLITS,
    )
    dataset = merge_sealed_test_dataset(candidate, sealed)
    assert_no_split_leakage(dataset)
    sealed_index = evaluator.build_sealed_signature_index(dataset)

    language_models = {
        0: LanguageModel.load("en_US"),
        1: LanguageModel.load("ru_RU"),
    }
    snapshots = {
        group: evaluator._hunspell_dictionary_snapshot(
            language_models[group], locale
        )
        for group, locale in ((0, "en_US"), (1, "ru_RU"))
    }
    development = evaluator.build_unknown_typo_disjoint_corpus(
        onboard_words,
        sealed_physical_signatures=sealed_index.signatures,
        minimum_words_per_group=(
            config.external_evaluation.minimum_words_per_group
        ),
        hunspell_snapshots=snapshots,
        language_models=language_models,
        rank_namespace=UNKNOWN_TYPO_DEVELOPMENT_RANK_NAMESPACE,
        choice_namespace=UNKNOWN_TYPO_DEVELOPMENT_CHOICE_NAMESPACE,
    )
    development_signatures = evaluator.unknown_typo_physical_signatures(
        development
    )
    combined_exclusions = (
        sealed_index.signatures | development_signatures
    )
    holdout = evaluator.build_unknown_typo_disjoint_corpus(
        onboard_words,
        sealed_physical_signatures=combined_exclusions,
        minimum_words_per_group=(
            config.external_evaluation.minimum_words_per_group
        ),
        hunspell_snapshots=snapshots,
        language_models=language_models,
        rank_namespace=UNKNOWN_TYPO_HOLDOUT_RANK_NAMESPACE,
        choice_namespace=UNKNOWN_TYPO_HOLDOUT_CHOICE_NAMESPACE,
    )
    holdout_signatures = evaluator.unknown_typo_physical_signatures(holdout)
    development_holdout_overlap = len(
        development_signatures & holdout_signatures
    )
    sealed_holdout_overlap = len(
        sealed_index.signatures & holdout_signatures
    )
    if development_holdout_overlap or sealed_holdout_overlap:
        raise RuntimeError("preseal holdout overlaps a previously exposed domain")
    frozen_development = (
        load_hard_negative_development_corpus(
            PROJECT_ROOT / config.hard_negative_development.source.path,
            config,
        )
        if verify_frozen_source
        else None
    )
    if frozen_development is not None and (
        frozen_development.expanded_corpus_sha256
        != development.corpus_sha256
        or frozen_development.physical_signatures_sha256
        != evaluator.physical_signature_set_sha256(
            development_signatures
        )
        or frozen_development.signature_count
        != len(development_signatures)
        or frozen_development.words_by_group != development.words_by_group
    ):
        raise RuntimeError(
            "frozen hard-negative source differs from model-blind development"
        )

    return ModelBlindExternalCorpora(
        config=config,
        development=development,
        holdout=holdout,
        sealed_index=sealed_index,
        development_signatures=development_signatures,
        holdout_signatures=holdout_signatures,
        combined_exclusions=frozenset(combined_exclusions),
        frozen_development=frozen_development,
    )


def build_preseal_receipt(
    config_path: Path,
    english_path: Path,
    russian_path: Path,
) -> dict[str, object]:
    """Build an auditable receipt without model loading or metric evaluation."""

    corpora = build_model_blind_external_corpora(
        config_path,
        english_path,
        russian_path,
    )
    config = corpora.config
    development = corpora.development
    holdout = corpora.holdout
    sealed_index = corpora.sealed_index
    development_signatures = corpora.development_signatures
    holdout_signatures = corpora.holdout_signatures
    combined_exclusions = corpora.combined_exclusions
    frozen_development = corpora.frozen_development
    if frozen_development is None:
        raise AssertionError("preseal receipt requires the frozen development source")
    release = _release_name()
    return {
        "schema_version": 1,
        "policy": f"keyswitch-intent-{release}-preseal-holdout",
        "model_loaded": False,
        "metrics_evaluated": False,
        "development": {
            "role": "hard-negative-presealed-roles",
            "rank_namespace": development.rank_namespace,
            "choice_namespace": development.choice_namespace,
            "corpus_sha256": development.corpus_sha256,
            "signature_count": len(development_signatures),
            "words_by_group": development.words_by_group,
            "frozen_source": {
                "path": config.hard_negative_development.source.path,
                "sha256": config.hard_negative_development.source.sha256,
                "bytes": config.hard_negative_development.source.bytes,
            },
            "role_namespace": (
                config.hard_negative_development.role_namespace
            ),
            "role_words_by_group": {
                split: {
                    str(group): count
                    for group, count in sorted(group_counts.items())
                }
                for split, group_counts in (
                    frozen_development.role_words_by_group.items()
                )
                if split in PRESEALED_SPLITS
            },
            "examples_by_role": {
                split: len(frozen_development.by_split[split])
                for split in PRESEALED_SPLITS
            },
            "training_example_weight": (
                config.hard_negative_development.training_example_weight
            ),
        },
        "holdout": {
            "role": "independent-release-holdout",
            "rank_namespace": holdout.rank_namespace,
            "choice_namespace": holdout.choice_namespace,
            "corpus_sha256": holdout.corpus_sha256,
            "signature_count": len(holdout_signatures),
            "words_by_group": holdout.words_by_group,
        },
        "sealed_dataset_exclusions": {
            "signature_count": sealed_index.signature_count,
            "sha256": sealed_index.sha256,
        },
        "combined_holdout_exclusions": {
            "signature_count": len(combined_exclusions),
            "sha256": evaluator.physical_signature_set_sha256(
                combined_exclusions
            ),
        },
        "overlap_counts": {
            "development_holdout": len(
                development_signatures & holdout_signatures
            ),
            "sealed_holdout": len(
                sealed_index.signatures & holdout_signatures
            ),
        },
    }


def main(argv: Sequence[str] | None = None) -> int:
    arguments = _parse_arguments(argv)
    report = build_preseal_receipt(
        cast(Path, arguments.config),
        cast(Path, arguments.en_model),
        cast(Path, arguments.ru_model),
    )
    print(
        json.dumps(
            report,
            ensure_ascii=False,
            indent=2,
            sort_keys=False,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
