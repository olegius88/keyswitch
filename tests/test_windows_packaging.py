"""Static contracts for the native Windows build and its CI entry points."""

from __future__ import annotations

import hashlib
import json
import re
import struct
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


PROJECT_ROOT = Path(__file__).resolve().parents[1]


class WindowsPackagingContractTests(unittest.TestCase):
    script: str
    deb_script: str
    deb_verifier: str
    git_attributes: str
    man_page: str

    @classmethod
    def setUpClass(cls) -> None:
        cls.script = (PROJECT_ROOT / "packaging/build-windows.ps1").read_text(
            encoding="utf-8"
        )
        cls.deb_script = (PROJECT_ROOT / "packaging/build-deb.sh").read_text(
            encoding="utf-8"
        )
        cls.deb_verifier = (
            PROJECT_ROOT / "tools/verify-native-deb.sh"
        ).read_text(encoding="utf-8")
        cls.git_attributes = (PROJECT_ROOT / ".gitattributes").read_text(
            encoding="utf-8"
        )
        cls.man_page = (PROJECT_ROOT / "packaging/keyswitch.1").read_text(
            encoding="utf-8"
        )

    def test_fresh_clone_defaults_to_the_frozen_repository_sources(self) -> None:
        self.assertIn(
            '$FrozenModelDirectory = Join-Path $ProjectDirectory "model\\intent_v1\\sources"',
            self.script,
        )
        self.assertIn("$ModelDirectory = $FrozenModelDirectory", self.script)
        self.assertIn(
            '$ModelLicense = Join-Path $FrozenModelDirectory "COPYRIGHT.onboard-data"',
            self.script,
        )
        default_section = self.script[
            self.script.index("$ProjectDirectory =") : self.script.index("$PyProject =")
        ]
        self.assertNotIn("build\\windows-models", default_section)

    def test_native_help_describes_the_v14_model_first_contract(self) -> None:
        for contract in (
            "keyswitch:intent-v14:physical-signature",
            "Training config schema 13",
            "sole statistical",
            "coverage and language scores are diagnostic only",
            "not handed to the fallback",
            "KEYSWITCH_MODEL_PATH",
            "bundled frozen EN/RU models",
        ):
            self.assertIn(contract, self.man_page)
        self.assertNotIn(
            "can veto its doubtful decisions",
            self.man_page,
        )

    def test_embedded_model_contract_validator_is_valid_and_fail_closed(self) -> None:
        match = re.search(
            r"\$ModelContractValidator = @'\n(.*?)\n'@",
            self.script,
            re.DOTALL,
        )
        self.assertIsNotNone(match)
        if match is None:
            raise AssertionError("embedded Python validator is missing")
        validator = match.group(1)
        compile(validator, "build-windows-model-contract", "exec")

        required_contracts = (
            "KSLM_MAXIMUM_CONTAINER_BYTES = 14 * 1024 * 1024",
            "KSLM_MAXIMUM_MANIFEST_BYTES = 1024 * 1024",
            "KSLM_MAXIMUM_PAYLOAD_BYTES = 12 * 1024 * 1024",
            "KSLM_MAXIMUM_FINGERPRINTS = 1 << 20",
            "KSLM_SCHEMA = 4",
            "schema != KSLM_SCHEMA",
            "flags != 0",
            "runtime_limits != packaging_limits",
            "artifact_bytes = verify_kslm_packaging_bounds(artifact_path)",
            'strict_json(manifest_path, 1024 * 1024, "intent manifest")',
            "config.schema_version != 13",
            'manifest.get("config_sha256") != config_digest',
            'manifest.get("gate_policy") != gate_policy_payload(config)',
            "SELECTION_FALSE_POSITIVE_COMPARISONS = 12",
            "SELECTION_WILSON_Z_SCORE = 2.8652602385321333",
            '"multiplicity_correction"',
            '"false_positive_bound"',
            "wilson_upper_bound(",
            'exact_true(manifest.get("quality_gates_passed")',
            '"false_positive_budget"',
            "selection_maximum_false_positives_per_trigger",
            "threshold_logit_margin_cap",
            '"global_logit_margin"',
            "selected_margins",
            "threshold_evidence(",
            "quality_evidence(",
            'model.metadata != signed_manifest',
            'model.threshold_logits[trigger][direction]',
            'thresholds[trigger]["logits"][direction]',
            'model.veto_threshold != raw_veto["selection"]["raw_logit"]',
        )
        for contract in required_contracts:
            self.assertIn(contract, validator)

        self.assertIn(
            "$IntentConfigObject.schema_version -ne 13",
            self.script,
        )
        self.assertIn(
            "Windows packaging requires intent-model training config schema 13",
            self.script,
        )

        validation_call = self.script.index(
            'Intent-model schema, provenance or quality evidence validation failed'
        )
        compilation = self.script.index('$NuitkaArguments = @(')
        self.assertLess(validation_call, compilation)

    def test_windows_preflight_is_bounded_and_replays_sealed_provenance(self) -> None:
        preflight = self.script[
            self.script.index("$ProjectDirectory =") : self.script.index(
                "$BuildDirectory ="
            )
        ]
        self.assertIn("function Read-BoundedFileBytes", self.script)
        self.assertIn("function Read-BoundedJsonObject", self.script)
        self.assertNotIn("ReadAllBytes", self.script)
        self.assertNotIn("Get-Content", self.script)
        self.assertNotIn("config_path.read_bytes()", self.script)
        self.assertNotIn("artifact_path.read_bytes()", self.script)
        self.assertIn(
            "config, config_digest = load_training_config_snapshot(config_path)",
            self.script,
        )
        self.assertIn(
            "artifact_digest = hashlib.sha256(artifact_bytes).hexdigest()",
            self.script,
        )
        self.assertNotIn("Get-Content $IntentManifest", preflight)
        self.assertNotIn("Get-Content $IntentConfig", preflight)
        self.assertIn('-MaximumBytes $IntentModelMaximumBytes', preflight)
        self.assertIn('-MaximumBytes 1MB', preflight)
        self.assertIn('-MaximumBytes 64KB', preflight)

        provenance = preflight.index("--provenance-only")
        embedded_contract = preflight.index("$ModelContractValidator")
        self.assertLess(provenance, embedded_contract)
        for argument in (
            '"--config", $IntentConfig',
            '"--en-model", $EnglishModel',
            '"--ru-model", $RussianModel',
            '"--artifact", $IntentModel',
            '"--manifest", $IntentManifest',
        ):
            self.assertIn(argument, preflight)

        for contract in (
            'model/intent_v1/sources/en_US.lm',
            'model/intent_v1/sources/ru_RU.lm',
            'model/intent_v1/sources/COPYRIGHT.onboard-data',
            'Get-VerifiedFrozenFileHash',
            'Frozen-source SHA256SUMS differs from config.json',
            'ExpectedBytes ([long]$EnglishSourcePolicy.bytes)',
            'ExpectedSha256 ([string]$EnglishSourcePolicy.sha256)',
        ):
            self.assertIn(contract, preflight)

    def test_every_raw_hashed_repository_file_forces_lf(self) -> None:
        required_lf_paths = (
            "model/intent_v1/config.json",
            "model/intent_v1/manifest.json",
            "model/intent_v1/test-report.json",
            "model/intent_v1/seal-registry-v2.json",
            "model/intent_v1/seal-registry-v3.json",
            "model/intent_v1/seal-registry-v4.json",
            "model/intent_v1/seal-registry-v5.json",
            "model/intent_v1/seal-registry-v6.json",
            "model/intent_v1/seal-registry-v7.json",
            "model/intent_v1/seal-registry-v8.json",
            "model/intent_v1/seal-registry-v9.json",
            "model/intent_v1/seal-registry-v10.json",
            "model/intent_v1/seal-registry-v11.json",
            "model/intent_v1/seal-registry-v12.json",
            "model/intent_v1/seal-registry-v13.json",
            "model/intent_v1/seal-registry-v14.json",
            "model/intent_v1/holdout-v6-preseal.json",
            "model/intent_v1/holdout-v7-preseal.json",
            "model/intent_v1/holdout-v8-preseal.json",
            "model/intent_v1/holdout-v9-preseal.json",
            "model/intent_v1/holdout-v10-preseal.json",
            "model/intent_v1/holdout-v11-preseal.json",
            "model/intent_v1/holdout-v12-preseal.json",
            "model/intent_v1/holdout-v13-preseal.json",
            "model/intent_v1/holdout-v14-preseal.json",
            "model/intent_v1/unknown-typo-development-v11.json",
            "model/intent_v1/unknown-typo-development-v12.json",
            "model/intent_v1/unknown-typo-development-v13.json",
            "model/intent_v1/unknown-typo-development-v14.json",
            "model/intent_v1/rejection-v12.json",
            "model/intent_v1/rejection-v13.json",
            "model/intent_v1/rejection-v11.json",
            "model/intent_v1/rejection-v10.json",
            "model/intent_v1/rejection-v9.json",
            "model/intent_v1/rejection-v8.json",
            "model/intent_v1/rejection-v7.json",
            "model/intent_v1/rejection-v6.json",
            "model/intent_v1/sources/SHA256SUMS",
            "tools/evaluate_intent_model.py",
            "tools/preseal_intent_holdout.py",
            "tools/freeze_intent_development_corpus.py",
            "tools/train_intent_model.py",
            "src/keyswitch/intent_model.py",
            "src/keyswitch/detector.py",
            "src/keyswitch/layouts.py",
            "src/keyswitch/language_model.py",
            "src/keyswitch/resources/protected_tokens.txt",
        )
        for path in required_lf_paths:
            self.assertIn(f"{path} text eol=lf", self.git_attributes)
        self.assertIn("model/intent_v1/sources/*.lm -text", self.git_attributes)
        self.assertIn(
            "model/intent_v1/sources/COPYRIGHT.onboard-data -text",
            self.git_attributes,
        )
        self.assertIn(
            "src/keyswitch/resources/models/*.ksm binary",
            self.git_attributes,
        )

    def test_every_native_package_enforces_the_same_bounded_kslm_envelope(
        self,
    ) -> None:
        self.assertIn("$IntentModelMaximumBytes = 12MB", self.script)
        self.assertIn(
            'src\\keyswitch\\resources\\models\\layout_intent_v1.ksm',
            self.script,
        )
        self.assertIn(
            'keyswitch\\resources\\models\\layout_intent_v1.ksm',
            self.script,
        )
        for shell_script in (self.deb_script, self.deb_verifier):
            self.assertIn(
                "intent_model_max_bytes=$((14 * 1024 * 1024))",
                shell_script,
            )
            self.assertIn(
                "intent_manifest_max_bytes=$((1024 * 1024))",
                shell_script,
            )
            self.assertIn(
                "intent_payload_max_bytes=$((12 * 1024 * 1024))",
                shell_script,
            )
            self.assertIn(
                "intent_fingerprint_max_count=$((1 << 20))",
                shell_script,
            )
            self.assertIn("verify_kslm_packaging_bounds", shell_script)
            self.assertIn(
                "supported_fingerprint_count", shell_script
            )
            self.assertIn(
                "KSLM payload shape does not match its embedded manifest",
                shell_script,
            )
            self.assertIn("if schema != 4:", shell_script)
            self.assertIn("if flags != 0:", shell_script)

        self.assertIn(
            'src/keyswitch/resources/models/layout_intent_v1.ksm',
            self.deb_script,
        )
        for variable, locale in (("english", "en_US"), ("russian", "ru_RU")):
            self.assertIn(
                f'--include-data-files="$frozen_{variable}_model='
                f'keyswitch/resources/models/{locale}.lm"',
                self.deb_script,
            )
            self.assertIn(
                f'usr/lib/keyswitch/keyswitch/resources/models/{locale}.lm',
                self.deb_verifier,
            )
        self.assertIn(
            'cmp -s "$expected_english_model" "$english_model"',
            self.deb_verifier,
        )
        self.assertIn(
            'cmp -s "$expected_russian_model" "$russian_model"',
            self.deb_verifier,
        )
        self.assertIn(
            'usr/lib/keyswitch/keyswitch/resources/models/layout_intent_v1.ksm',
            self.deb_verifier,
        )

    def test_deb_build_preserves_strict_model_evaluation_diagnostics(self) -> None:
        self.assertIn(
            'mkdir -p -- "$project_dir/build" "$output_dir"',
            self.deb_script,
        )
        self.assertIn(
            'intent_quality_report="$output_dir/keyswitch-intent-evaluation.json"',
            self.deb_script,
        )
        self.assertIn('--strict >"$intent_quality_report"', self.deb_script)
        self.assertNotIn("--strict >/dev/null", self.deb_script)
        self.assertIn("Failed strict intent-model gates:", self.deb_script)
        self.assertIn(
            "report exceeds the 8 MiB diagnostic bound",
            self.deb_script,
        )

    def test_shell_kslm_envelope_validators_accept_only_bounded_shapes(
        self,
    ) -> None:
        header = struct.Struct("<4sHHIII32s")
        limits = (14 * 1024 * 1024, 1024 * 1024, 12 * 1024 * 1024, 1 << 20)

        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            for index, shell_script in enumerate(
                (self.deb_script, self.deb_verifier)
            ):
                match = re.search(
                    r"python3 -c '\n(.*?)\n' \"\$model_path\"",
                    shell_script,
                    re.DOTALL,
                )
                self.assertIsNotNone(match)
                if match is None:
                    raise AssertionError("shell KSLM validator is missing")
                validator = match.group(1)
                compile(validator, f"shell-kslm-validator-{index}", "exec")

                valid_manifest = json.dumps(
                    {"dimension": 1, "supported_fingerprint_count": 0},
                    separators=(",", ":"),
                    sort_keys=True,
                ).encode("utf-8")
                payload = b"\0\0"
                valid_path = root / f"valid-{index}.ksm"
                valid_path.write_bytes(
                    header.pack(
                        b"KSLM",
                        4,
                        0,
                        len(valid_manifest),
                        len(payload),
                        0,
                        hashlib.sha256(valid_manifest).digest(),
                    )
                    + valid_manifest
                    + payload
                )
                accepted = subprocess.run(
                    [sys.executable, "-c", validator, str(valid_path), *map(str, limits)],
                    check=False,
                    capture_output=True,
                    text=True,
                )
                self.assertEqual(accepted.returncode, 0, accepted.stderr)

                for label, schema, flags, expected_error in (
                    ("schema", 3, 0, "schema is unsupported"),
                    ("flags", 4, 1, "header flags are unsupported"),
                ):
                    header_invalid_path = root / f"{label}-{index}.ksm"
                    header_invalid_path.write_bytes(
                        header.pack(
                            b"KSLM",
                            schema,
                            flags,
                            len(valid_manifest),
                            len(payload),
                            0,
                            hashlib.sha256(valid_manifest).digest(),
                        )
                        + valid_manifest
                        + payload
                    )
                    header_rejected = subprocess.run(
                        [
                            sys.executable,
                            "-c",
                            validator,
                            str(header_invalid_path),
                            *map(str, limits),
                        ],
                        check=False,
                        capture_output=True,
                        text=True,
                    )
                    self.assertNotEqual(header_rejected.returncode, 0)
                    self.assertIn(expected_error, header_rejected.stderr)

                oversized_fingerprint_manifest = json.dumps(
                    {
                        "dimension": 1,
                        "supported_fingerprint_count": (1 << 20) + 1,
                    },
                    separators=(",", ":"),
                    sort_keys=True,
                ).encode("utf-8")
                invalid_path = root / f"invalid-{index}.ksm"
                invalid_path.write_bytes(
                    header.pack(
                        b"KSLM",
                        4,
                        0,
                        len(oversized_fingerprint_manifest),
                        len(payload),
                        0,
                        hashlib.sha256(oversized_fingerprint_manifest).digest(),
                    )
                    + oversized_fingerprint_manifest
                    + payload
                )
                rejected = subprocess.run(
                    [
                        sys.executable,
                        "-c",
                        validator,
                        str(invalid_path),
                        *map(str, limits),
                    ],
                    check=False,
                    capture_output=True,
                    text=True,
                )
                self.assertNotEqual(rejected.returncode, 0)
                self.assertIn("2^20", rejected.stderr)

    def test_built_executable_must_load_the_in_distribution_intent_model(self) -> None:
        executable_check = self.script.index(
            'if (-not (Test-Path $Executable -PathType Leaf))'
        )
        diagnostic_launch = self.script.index('-ArgumentList "--diagnose"')
        archive = self.script.index("Compress-Archive")
        self.assertLess(executable_check, diagnostic_launch)
        self.assertLess(diagnostic_launch, archive)
        self.assertIn(
            "$PostBuildDiagnostics.intent_model.available -isnot [bool]",
            self.script,
        )
        self.assertIn(
            "$PostBuildDiagnostics.intent_model.checksum -ne $IntentHash",
            self.script,
        )
        self.assertIn(
            "$PostBuildDiagnostics.intent_model.version -ne $IntentManifestObject.artifact_model_version",
            self.script,
        )
        self.assertIn(
            "did not load the exact in-distribution intent model",
            self.script,
        )
        self.assertIn("WaitForExit(30000)", self.script)
        self.assertIn(
            'Read-BoundedJsonObject `\n    -Path $PostBuildDiagnosticsPath',
            self.script,
        )
        self.assertIn(
            '-Label "native-distribution English language model"',
            self.script,
        )
        self.assertIn(
            '-Label "native-distribution Russian language model"',
            self.script,
        )
        self.assertIn(
            '-Label "native-distribution language-model license"',
            self.script,
        )

    def test_linux_native_diagnostics_bind_model_version_and_checksum(self) -> None:
        for shell_script in (self.deb_script, self.deb_verifier):
            self.assertNotIn("payload_path.read_text", shell_script)
            self.assertNotIn("expected_path.read_bytes", shell_script)
            self.assertIn(
                'def bounded_read(path: Path, maximum_bytes: int, label: str)',
                shell_script,
            )
            self.assertIn(
                'expected_bytes = bounded_read(expected_path, maximum_model_bytes, "KSLM model")',
                shell_script,
            )
            self.assertIn('expected_version = embedded.get("model_version")', shell_script)
            self.assertIn('intent.get("version") == expected_version', shell_script)
            self.assertIn(
                'intent.get("checksum")\n    == hashlib.sha256(expected_bytes).hexdigest()',
                shell_script,
            )

    def test_linux_native_diagnostic_validators_accept_only_exact_model(
        self,
    ) -> None:
        header = struct.Struct("<4sHHIII32s")
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            manifest = json.dumps(
                {"model_version": "intent-v1-test"},
                separators=(",", ":"),
                sort_keys=True,
            ).encode("utf-8")
            payload = b"\0\0"
            model = root / "layout_intent_v1.ksm"
            model.write_bytes(
                header.pack(
                    b"KSLM",
                    4,
                    0,
                    len(manifest),
                    len(payload),
                    0,
                    hashlib.sha256(manifest).digest(),
                )
                + manifest
                + payload
            )
            diagnostic = root / "diagnostic.json"
            exact_status = {
                "intent_model": {
                    "available": True,
                    "path": str(model.resolve()),
                    "version": "intent-v1-test",
                    "checksum": hashlib.sha256(model.read_bytes()).hexdigest(),
                }
            }

            for index, shell_script in enumerate(
                (self.deb_script, self.deb_verifier)
            ):
                match = re.search(
                    r"if ! python3 -c '\n(.*?)\n' \"\$diagnostic_json\" "
                    r'"\$(?:expected_model|intent_model)"',
                    shell_script,
                    re.DOTALL,
                )
                self.assertIsNotNone(match)
                if match is None:
                    raise AssertionError("native diagnostic validator is missing")
                validator = match.group(1)
                compile(validator, f"native-diagnostic-validator-{index}", "exec")

                diagnostic.write_text(json.dumps(exact_status), encoding="utf-8")
                accepted = subprocess.run(
                    [
                        sys.executable,
                        "-c",
                        validator,
                        str(diagnostic),
                        str(model),
                        str(14 * 1024 * 1024),
                        str(1024 * 1024),
                    ],
                    check=False,
                    capture_output=True,
                    text=True,
                )
                self.assertEqual(accepted.returncode, 0, accepted.stderr)

                wrong_status = json.loads(json.dumps(exact_status))
                wrong_status["intent_model"]["version"] = "intent-v1-wrong"
                diagnostic.write_text(json.dumps(wrong_status), encoding="utf-8")
                rejected = subprocess.run(
                    [
                        sys.executable,
                        "-c",
                        validator,
                        str(diagnostic),
                        str(model),
                        str(14 * 1024 * 1024),
                        str(1024 * 1024),
                    ],
                    check=False,
                    capture_output=True,
                    text=True,
                )
                self.assertNotEqual(rejected.returncode, 0)

    def test_windows_workflows_exercise_fresh_clone_defaults(self) -> None:
        for relative in (
            ".github/workflows/tests.yml",
            ".github/workflows/release.yml",
        ):
            workflow = (PROJECT_ROOT / relative).read_text(encoding="utf-8")
            self.assertNotIn("keyswitch-windows-models", workflow)
            self.assertNotIn("build/windows-models", workflow)
            self.assertIn(
                "KEYSWITCH_MODEL_PATH: model/intent_v1/sources",
                workflow,
            )
            self.assertIn(
                "run: ./packaging/build-windows.ps1",
                workflow,
            )
            self.assertIn(
                "Build and self-diagnose native ZIP and Inno Setup installer",
                workflow,
            )
            installer = workflow.index(
                '$install = Start-Process -FilePath $installer'
            )
            installed_diagnostics = workflow.index(
                '$diagnosticProcess = Start-Process -FilePath $installedExecutable'
            )
            ui_smoke = workflow.index(
                '$smoke = Start-Process -FilePath (Join-Path $installRoot "KeySwitch.exe")'
            )
            self.assertLess(installer, installed_diagnostics)
            self.assertLess(installed_diagnostics, ui_smoke)
            for contract in (
                '$diagnosticProcess.WaitForExit(30000)',
                'Installed executable diagnostics exceed 1 MiB',
                '$diagnostics.intent_model.checksum -cne',
                '$diagnostics.intent_model.version -cne $manifest.artifact_model_version',
                'exact in-installation intent model',
            ):
                self.assertIn(contract, workflow)


if __name__ == "__main__":
    unittest.main()
