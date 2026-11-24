"""Contrat documentaire F006 entre brouillon, rapport et publication."""
import copy
import hashlib
import json
import unittest
from pathlib import Path

from jsonschema import Draft202012Validator, FormatChecker
from referencing import Registry, Resource
from referencing.jsonschema import DRAFT202012

ROOT = Path(__file__).resolve().parents[2] / ".claude"
EXAMPLES = ROOT / "examples"


def read_json(path):
    return json.loads(path.read_text(encoding="utf-8"))


class WorkflowValidationContractTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.schemas = {
            path.name: read_json(path) for path in (ROOT / "schemas").glob("*.schema.json")
        }
        cls.registry = Registry().with_resources(
            (schema["$id"], Resource.from_contents(schema, default_specification=DRAFT202012))
            for schema in cls.schemas.values()
        )

    def validator(self, name, definition=None):
        schema = self.schemas[f"ipc-{name}.schema.json"]
        if definition:
            schema = {"$ref": f"{schema['$id']}#/$defs/{definition}"}
        return Draft202012Validator(schema, registry=self.registry, format_checker=FormatChecker())

    def setUp(self):
        self.command = read_json(EXAMPLES / "ipc-validate-workflow-draft.json")
        self.query = read_json(EXAMPLES / "ipc-get-workflow-validation-query.json")
        self.valid = read_json(EXAMPLES / "ipc-get-workflow-validation-valid-response.json")
        self.invalid = read_json(EXAMPLES / "ipc-get-workflow-validation-invalid-response.json")

    def test_catalogue_is_explicit_or_pinned_by_daemon(self):
        validator = self.validator("command")
        self.assertTrue(validator.is_valid(self.command))
        default = copy.deepcopy(self.command)
        default["payload"]["catalogue_snapshot_ref"] = None
        self.assertTrue(validator.is_valid(default))

        del default["payload"]["catalogue_snapshot_ref"]
        self.assertFalse(validator.is_valid(default))
        for invalid_ref in (
            {"schema_version": 1},
            {"schema_version": 2, "digest": "sha256:" + "a2" * 32},
            {"schema_version": 1, "digest": "sha256:abc"},
            {**self.command["payload"]["catalogue_snapshot_ref"], "entries": []},
        ):
            with self.subTest(ref=invalid_ref):
                default["payload"]["catalogue_snapshot_ref"] = invalid_ref
                self.assertFalse(validator.is_valid(default))

    def test_report_creation_and_query_coordinates(self):
        command_validator = self.validator("command")
        for changes in ({"project_id": None}, {"aggregate_id": "report"}, {"expected_revision": 1}):
            with self.subTest(changes=changes):
                self.assertFalse(command_validator.is_valid({**self.command, **changes}))
        for changes in ({"draft_revision": 0}, {"draft_id": "draft"}, {"extra": True}):
            with self.subTest(payload_changes=changes):
                payload = {**self.command["payload"], **changes}
                invalid = {**self.command, "payload": payload}
                self.assertFalse(command_validator.is_valid(invalid))

        query_validator = self.validator("query")
        self.assertTrue(query_validator.is_valid(self.query))
        for changes in ({"project_id": None}, {"payload": {}}, {"payload": {"validation_report_id": 3}}):
            with self.subTest(changes=changes):
                self.assertFalse(query_validator.is_valid({**self.query, **changes}))

    def test_report_disposition_requires_matching_fields(self):
        validator = self.validator("query-response", "workflow_validation_response")
        self.assertTrue(validator.is_valid(self.valid))
        self.assertTrue(validator.is_valid(self.invalid))

        for field in ("normalized_digest", "draft_digest", "bounds", "catalogue_snapshot_ref"):
            with self.subTest(field=field):
                missing = copy.deepcopy(self.valid)
                del missing["result"][field]
                self.assertFalse(validator.is_valid(missing))
        for field in ("workflow_profile", "required_capabilities", "bounds", "catalogue_snapshot_ref"):
            with self.subTest(null_field=field):
                null_field = copy.deepcopy(self.valid)
                null_field["result"][field] = None
                self.assertFalse(validator.is_valid(null_field))

        contradictory = copy.deepcopy(self.invalid)
        contradictory["result"]["normalized_digest"] = self.valid["result"]["normalized_digest"]
        self.assertFalse(validator.is_valid(contradictory))
        contradictory = copy.deepcopy(self.valid)
        contradictory["result"]["diagnostics"] = self.invalid["result"]["diagnostics"]
        self.assertFalse(validator.is_valid(contradictory))

    def test_invalid_report_needs_localized_error_and_watermark(self):
        validator = self.validator("query-response", "workflow_validation_response")
        error = self.invalid["result"]["diagnostics"][0]
        warning = {**error, "severity": "warning"}
        malformed_pointer = {**error, "pointer": "/incomplet~"}
        for diagnostics in ([], [warning], [malformed_pointer]):
            with self.subTest(diagnostics=diagnostics):
                result = {**self.invalid["result"], "diagnostics": diagnostics}
                self.assertFalse(validator.is_valid({**self.invalid, "result": result}))
        for changes in ({"watermark": None}, {"watermark": {"sequence": 7}}):
            with self.subTest(changes=changes):
                self.assertFalse(validator.is_valid({**self.invalid, **changes}))

    def test_v1_report_keeps_the_v1_concurrency_bounds(self):
        validator = self.validator("query-response", "workflow_validation_response")
        for field, value in (("max_agent_executions", 0), ("max_concurrent_action_executions", 1)):
            with self.subTest(field=field):
                report = copy.deepcopy(self.valid)
                report["result"]["bounds"][field] = value
                self.assertFalse(validator.is_valid(report))
                report["result"]["workflow_profile"] = "unleash.workflow.v1.2"
                self.assertTrue(validator.is_valid(report))

    def test_report_fixtures_link_draft_catalogue_and_publication(self):
        report = self.valid["result"]
        publication = read_json(EXAMPLES / "ipc-publish-workflow-version.json")
        self.assertEqual(report["project_id"], self.command["project_id"])
        self.assertEqual(report["project_id"], self.query["project_id"])
        self.assertEqual(report["project_id"], publication["project_id"])
        self.assertEqual(self.valid["request_id"], self.query["request_id"])
        self.assertEqual(report["validation_report_id"], self.command["aggregate_id"])
        self.assertEqual(report["validation_report_id"], self.query["payload"]["validation_report_id"])
        for field in ("draft_id", "draft_revision", "catalogue_snapshot_ref"):
            self.assertEqual(report[field], self.command["payload"][field])
        for field in ("draft_id", "draft_revision", "validation_report_id", "normalized_digest"):
            self.assertEqual(report[field], publication["payload"][field])
        fixture_bytes = (EXAMPLES / "merge-only-workflow.json").read_bytes()
        self.assertEqual(report["draft_digest"], "sha256:" + hashlib.sha256(fixture_bytes).hexdigest())

        manual = read_json(EXAMPLES / "ipc-get-workflow-text-draft-response.json")["result"]
        invalid_report = self.invalid["result"]
        self.assertEqual(invalid_report["project_id"], manual["project_id"])
        self.assertEqual(invalid_report["draft_id"], manual["draft_id"])
        self.assertEqual(invalid_report["draft_revision"], manual["revision"])
        self.assertEqual(invalid_report["draft_digest"], manual["text_hash"])

    def test_operation_descriptions_match_the_catalogue_source(self):
        source = "architecture/client-operation-catalog.md"
        groups = ("Validation workflow / F006", "Workflows 01–05 + AUTOMATION-01–04")
        rows = {}
        for line in (ROOT / source).read_text(encoding="utf-8").splitlines():
            if line.startswith("| "):
                cells = line.strip("|").split(" | ", 2)
                if len(cells) == 3 and cells[0].strip() in groups:
                    rows[cells[0].strip()] = cells[2].strip()
        self.assertEqual(set(rows), set(groups))
        catalogue = read_json(ROOT / "catalog" / "operations.json")
        seen = set()
        for operation in catalogue["operations"]:
            origin = operation["contract_source"]
            if origin["path"] == source and origin["subject"] in groups:
                with self.subTest(operation=operation["name"]):
                    self.assertEqual(operation["description"], rows[origin["subject"]])
                seen.add(operation["name"])
        self.assertTrue({"ValidateWorkflowDraft", "GetWorkflowValidation"} <= seen)
