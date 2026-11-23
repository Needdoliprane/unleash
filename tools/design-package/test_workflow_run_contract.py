import copy
import json
import unittest
from pathlib import Path

from jsonschema import Draft202012Validator, FormatChecker, ValidationError
from referencing import Registry, Resource


CLAUDE = Path(__file__).resolve().parents[2] / ".claude"


def read_document(path):
    return json.loads((CLAUDE / path).read_text(encoding="utf-8"))


class ListRunsContractTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        query_schema = read_document("schemas/ipc-query.schema.json")
        response_schema = read_document("schemas/ipc-query-response.schema.json")
        registry = Registry().with_resources(
            (schema["$id"], Resource.from_contents(schema))
            for schema in (query_schema, response_schema)
        )
        cls.query_validator = Draft202012Validator(query_schema, format_checker=FormatChecker())
        cls.response_validator = Draft202012Validator(
            {"$ref": f"{response_schema['$id']}#/$defs/workflow_run_list_response"},
            registry=registry,
            format_checker=FormatChecker(),
        )

    def setUp(self):
        self.query = read_document("examples/ipc-list-runs-query.json")
        self.response = read_document("examples/ipc-list-runs-response.json")

    def test_list_runs_requires_a_project_and_closed_bounded_arguments(self):
        self.query_validator.validate(self.query)
        for overrides in (
            {"project_id": None},
            {"project_id": "00000000-0000-0000-0000-000000000000"},
            {"project_id": "Atlas"},
            {"payload": {"project_id": self.query["project_id"]}},
            {"payload": {"page_size": 0}},
            {"payload": {"page_size": 101}},
            {"payload": {"opaque_cursor": ""}},
            {"payload": {"opaque_cursor": "A" * 513}},
        ):
            with self.subTest(overrides=overrides), self.assertRaises(ValidationError):
                self.query_validator.validate(self.query | overrides)

    def test_first_and_continuation_requests_use_the_same_envelope(self):
        for payload in ({}, {"opaque_cursor": None}, {"opaque_cursor": "fixture-next-page"}):
            with self.subTest(payload=payload):
                self.query_validator.validate(self.query | {"payload": payload})

    def test_page_keeps_the_pinned_run_coordinates_and_transactional_watermark(self):
        self.response_validator.validate(self.response)
        for field, patch in (
            ("watermark", lambda page: page.pop("watermark")),
            ("project_id", lambda page: page["result"].pop("project_id")),
            ("workflow_version_id", lambda page: page["result"]["items"][0].pop("workflow_version_id")),
            ("state", lambda page: page["result"]["items"][0].update(state="done")),
            ("plan", lambda page: page["result"]["items"][0].update(plan={})),
            ("next_cursor", lambda page: page["result"].update(next_cursor="")),
            ("coverage", lambda page: page["result"].update(coverage={"status": "unknown"})),
        ):
            with self.subTest(field=field):
                invalid = copy.deepcopy(self.response)
                patch(invalid)
                with self.assertRaises(ValidationError):
                    self.response_validator.validate(invalid)

    def test_empty_page_is_terminal(self):
        self.response["result"].update(items=[], next_cursor=None)
        self.response_validator.validate(self.response)
        self.response["result"]["next_cursor"] = "fixture-next-page"
        with self.assertRaises(ValidationError):
            self.response_validator.validate(self.response)
