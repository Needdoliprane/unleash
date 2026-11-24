"""F006: schéma de la tranche manuelle, sans prétendre valider le daemon."""
import copy
import hashlib
import json
import unittest
from pathlib import Path

from jsonschema import Draft202012Validator, FormatChecker
from referencing import Registry, Resource

ROOT = Path(__file__).resolve().parents[2]
SCHEMAS = ROOT / ".claude" / "schemas"
EXAMPLES = ROOT / ".claude" / "examples"


def load(path):
    return json.loads(path.read_text(encoding="utf-8"))


class WorkflowTextDraftContractTests(unittest.TestCase):
    def setUp(self):
        self.command = load(EXAMPLES / "ipc-create-workflow-text-draft.json")
        self.query = load(EXAMPLES / "ipc-get-workflow-draft-query.json")
        self.response = load(EXAMPLES / "ipc-get-workflow-text-draft-response.json")
        self.command_validator = Draft202012Validator(
            load(SCHEMAS / "ipc-command.schema.json"), format_checker=FormatChecker()
        )
        self.query_validator = Draft202012Validator(
            load(SCHEMAS / "ipc-query.schema.json"), format_checker=FormatChecker()
        )
        response_schema = load(SCHEMAS / "ipc-query-response.schema.json")
        registry = Registry().with_resource(
            response_schema["$id"], Resource.from_contents(response_schema)
        )
        self.response_validator = Draft202012Validator(
            {"$ref": f"{response_schema['$id']}#/$defs/workflow_text_draft_response"},
            registry=registry,
            format_checker=FormatChecker(),
        )

    def test_manual_fixture_accepts_empty_invalid_and_duplicate_key_text(self):
        for text in ("", "{", "{\"x\":1,\"x\":2}"):
            with self.subTest(text=text):
                command = copy.deepcopy(self.command)
                command["payload"]["source"]["text"] = text
                self.assertTrue(self.command_validator.is_valid(command))

    def test_manual_coordinates_and_closed_source(self):
        for mutate in ("project", "revision", "extra", "missing", "oversize"):
            with self.subTest(mutate=mutate):
                command = copy.deepcopy(self.command)
                if mutate == "project":
                    command["project_id"] = None
                elif mutate == "revision":
                    command["expected_revision"] = 1
                elif mutate == "extra":
                    command["payload"]["source"]["extra"] = True
                elif mutate == "missing":
                    del command["payload"]["source"]["text"]
                else:
                    command["payload"]["source"]["text"] = "x" * 1_048_577
                self.assertFalse(self.command_validator.is_valid(command))

    def test_query_requires_project_and_exact_draft_id(self):
        self.assertTrue(self.query_validator.is_valid(self.query))
        for field in ("project_id", "draft_id", "extra"):
            query = copy.deepcopy(self.query)
            if field == "project_id":
                query[field] = None
            elif field == "draft_id":
                del query["payload"][field]
            else:
                query["payload"][field] = True
            self.assertFalse(self.query_validator.is_valid(query))

    def test_response_returns_exact_text_digest_and_watermark(self):
        self.assertTrue(self.response_validator.is_valid(self.response))
        for field in ("draft_id", "project_id", "revision", "text", "text_hash"):
            response = copy.deepcopy(self.response)
            del response["result"][field]
            self.assertFalse(self.response_validator.is_valid(response))
        for field in ("request_id", "result", "watermark"):
            response = copy.deepcopy(self.response)
            response.pop(field)
            self.assertFalse(self.response_validator.is_valid(response))
        for field in ("stream_id", "sequence"):
            response = copy.deepcopy(self.response)
            del response["watermark"][field]
            self.assertFalse(self.response_validator.is_valid(response))

    def test_fixtures_keep_exact_text_and_digest(self):
        text = self.command["payload"]["source"]["text"]
        self.assertEqual(text, self.response["result"]["text"])
        self.assertEqual(self.command["aggregate_id"], self.query["payload"]["draft_id"])
        self.assertEqual(self.query["payload"]["draft_id"], self.response["result"]["draft_id"])
        self.assertEqual(self.command["project_id"], self.response["result"]["project_id"])
        self.assertEqual(
            self.response["result"]["text_hash"],
            "sha256:" + hashlib.sha256(text.encode("utf-8")).hexdigest(),
        )

    def test_exact_published_version_is_a_distinct_source(self):
        command = load(EXAMPLES / "ipc-create-workflow-version-draft.json")
        self.assertTrue(self.command_validator.is_valid(command))
        self.assertEqual(command["payload"]["source"]["id"], "workflow.merge-only")
        self.assertEqual(command["payload"]["source"]["version"], 1)
        self.assertEqual(
            command["payload"]["source"]["digest"],
            "sha256:5e0309449c265a65e65653567f52a97dc32ed9851dc79eb85a0ed7ebcc03ae79",
        )
        for field in ("id", "version", "digest"):
            invalid = copy.deepcopy(command)
            del invalid["payload"]["source"][field]
            self.assertFalse(self.command_validator.is_valid(invalid))
        command["payload"]["source"]["kind"] = "unknown"
        self.assertFalse(self.command_validator.is_valid(command))

    def test_source_without_discriminator_is_refused(self):
        command = copy.deepcopy(self.command)
        del command["payload"]["source"]["kind"]
        self.assertFalse(self.command_validator.is_valid(command))

    def test_schema_character_limit_is_not_utf8_byte_limit(self):
        command = copy.deepcopy(self.command)
        command["payload"]["source"]["text"] = "é" * 524_289
        self.assertTrue(self.command_validator.is_valid(command))
        self.assertGreater(len(command["payload"]["source"]["text"].encode("utf-8")), 1_048_576)


if __name__ == "__main__":
    unittest.main()
