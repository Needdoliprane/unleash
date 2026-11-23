import hashlib
import json
import unittest
from pathlib import Path

from jsonschema import Draft202012Validator, FormatChecker, ValidationError


CLAUDE = Path(__file__).resolve().parents[2] / ".claude"


class WorkflowPublicationContractTests(unittest.TestCase):
    def setUp(self):
        schema = json.loads((CLAUDE / "schemas/ipc-command.schema.json").read_text(encoding="utf-8"))
        self.validator = Draft202012Validator(schema, format_checker=FormatChecker())
        fixture = CLAUDE / "examples/ipc-publish-workflow-version.json"
        self.command = json.loads(fixture.read_text(encoding="utf-8"))

    def test_t_f006_009_requires_a_validation_report_to_publish(self):
        self.validator.validate(self.command)
        del self.command["payload"]["validation_report_id"]
        with self.assertRaises(ValidationError):
            self.validator.validate(self.command)

    def test_t_f006_009_requires_the_normalized_digest(self):
        del self.command["payload"]["normalized_digest"]
        with self.assertRaises(ValidationError):
            self.validator.validate(self.command)

    def test_t_f006_009_accepts_an_exact_previous_version(self):
        self.command["payload"]["expected_previous_version"] = 1
        self.validator.validate(self.command)

    def test_t_f006_009_fixture_targets_the_exact_definition_for_create_run(self):
        source = CLAUDE / "examples/merge-only-workflow.json"
        definition = json.loads(source.read_text(encoding="utf-8"))
        published = self.command["payload"]
        self.assertEqual(definition["workflow_id"], published["workflow_id"])
        self.assertEqual(definition["version"], published["expected_previous_version"] + 1)

        # The merge-only fixture uses ASCII keys and integers.
        normalized = json.dumps(
            definition, ensure_ascii=False, sort_keys=True, separators=(",", ":")
        ).encode("utf-8")
        digest = f"sha256:{hashlib.sha256(normalized).hexdigest()}"
        self.assertEqual(digest, published["normalized_digest"])

        self.validator.validate({
            **self.command,
            "request_id": "10000000-0000-4000-8000-000000000035",
            "command_id": "10000000-0000-4000-8000-000000000036",
            "aggregate_id": "10000000-0000-4000-8000-000000000037",
            "command_type": "CreateRun",
            "expected_revision": 0,
            "payload": {
                "mission_id": "10000000-0000-4000-8000-000000000038",
                "source": {
                    "kind": "workflow_version",
                    "id": published["workflow_id"],
                    "version": published["expected_previous_version"] + 1,
                    "digest": published["normalized_digest"],
                },
                "purpose": "operational",
                "selection_reason": "Exact published workflow from the validation report",
                "scope_ref": "sha256:" + "b" * 64,
            },
        })

    def test_t_f006_009_rejects_noncanonical_coordinates(self):
        payload = self.command["payload"]
        for field, value in (
            ("draft_id", "workflow.fixture"),
            ("draft_revision", 0),
            ("validation_report_id", "latest"),
            ("normalized_digest", "a" * 64),
            ("workflow_id", "workflow/fixture"),
            ("expected_previous_version", -1),
            ("expected_previous_version", "latest"),
            ("expected_previous_version", 4294967296),
        ):
            with self.subTest(field=field, value=value):
                mutated = {**self.command, "payload": {**payload, field: value}}
                with self.assertRaises(ValidationError):
                    self.validator.validate(mutated)

    def test_t_f006_009_requires_an_explicit_previous_version_without_writer_fields(self):
        payload = self.command["payload"]
        del payload["expected_previous_version"]
        with self.assertRaises(ValidationError):
            self.validator.validate(self.command)
        payload.update(expected_previous_version=0, version=2)
        with self.assertRaises(ValidationError):
            self.validator.validate(self.command)


if __name__ == "__main__":
    unittest.main()
