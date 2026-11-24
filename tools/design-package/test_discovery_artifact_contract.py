import base64
from copy import deepcopy
import hashlib
import json
import unittest
from pathlib import Path

from jsonschema import Draft202012Validator, FormatChecker, ValidationError
from referencing import Registry, Resource


CLAUDE = Path(__file__).resolve().parents[2] / ".claude"
QUERY = "urn:ovpt:unleash:ipc-query:1"
RESPONSE = "urn:ovpt:unleash:ipc-query-response:1"
RECEIPT = "urn:ovpt:unleash:discovery-receipt:1"


def fixture(name):
    return json.loads((CLAUDE / "examples" / f"ipc-{name}.json").read_text(encoding="utf-8"))


class DiscoveryArtifactContractTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        schemas = [
            json.loads(path.read_text(encoding="utf-8"))
            for path in (CLAUDE / "schemas").glob("*.schema.json")
        ]
        registry = Registry().with_resources(
            (schema["$id"], Resource.from_contents(schema)) for schema in schemas
        )
        cls.query = Draft202012Validator(
            {"$ref": QUERY}, registry=registry, format_checker=FormatChecker()
        )
        cls.discovery = cls.query.evolve(
            schema={"$ref": RESPONSE + "#/$defs/discovery_receipt_response"}
        )
        cls.discovery_block = cls.query.evolve(
            schema={"$ref": RESPONSE + "#/$defs/discovery_receipt_artifact_response"}
        )
        cls.workflow_block = cls.query.evolve(
            schema={"$ref": RESPONSE + "#/$defs/workflow_draft_artifact_response"}
        )
        cls.canonical_receipt = cls.query.evolve(
            schema={"$ref": RECEIPT + "#/$defs/canonicalReceipt"}
        )

    def test_t_f009_005_reference_identifies_the_complete_large_receipt(self):
        inline = fixture("get-discovery-receipt-stale-response")
        result = fixture("get-discovery-receipt-artifact-response")
        lookup = fixture("get-discovery-receipt-artifact-query")
        block_query = fixture("read-discovery-receipt-artifact-query")
        block = fixture("read-discovery-receipt-artifact-response")
        self.discovery.validate(inline)
        self.discovery.validate(result)
        self.query.validate(lookup)
        self.query.validate(block_query)
        self.discovery_block.validate(block)

        reference = result["result"]["receipt_artifact"]
        body = {**inline["result"]["receipt"], "receipt_id": lookup["payload"]["receipt_id"]}
        source = body["source_refs"][0]
        batches, sources_per_batch = 200, 100
        body["query_ids"] = [f"query-fixture-{batch:03d}" for batch in range(batches)]
        body["source_refs"] = [
            {
                **source,
                "source_id": f"source-doc-{index:05d}",
                "version_id": f"version-doc-{index:05d}",
                "resource_ref_id": f"resource-doc-{index:05d}",
            }
            for index in range(batches * sources_per_batch)
        ]
        # This fixture uses ASCII keys and exact integers.
        canonical = json.dumps(
            body, ensure_ascii=False, sort_keys=True, separators=(",", ":")
        ).encode("utf-8")
        self.assertGreater(len(canonical), 8 * 1024 * 1024)
        self.assertEqual(reference["receipt_id"], body["receipt_id"])
        self.assertEqual(reference["size_bytes"], len(canonical))
        self.assertEqual(reference["digest"], "sha256:" + hashlib.sha256(canonical).hexdigest())
        self.assertEqual(result["result"]["history"], inline["result"]["history"])
        for envelope in (result["result"], block_query, block["result"]):
            self.assertEqual(envelope["project_id"], lookup["project_id"])
        self.assertEqual(block_query["payload"]["artifact_ref"], reference)
        self.assertEqual(block["result"]["artifact_ref"], reference)
        self.assertEqual(block["request_id"], block_query["request_id"])
        start = block_query["payload"]["offset_bytes"]
        end = start + block_query["payload"]["limit_bytes"]
        self.assertEqual(block["result"]["offset_bytes"], start)
        self.assertEqual(block["result"]["next_offset_bytes"], end)
        self.assertEqual(
            base64.b64decode(block["result"]["data_base64"], validate=True), canonical[start:end]
        )
        self.assertEqual(canonical[start:start + 1], b"\xa9")

    def test_t_f009_008_creation_requires_exact_canonical_integers(self):
        inline = fixture("get-discovery-receipt-valid-response")["result"]["receipt"]
        lookup = fixture("get-discovery-receipt-artifact-query")
        body = {**inline, "receipt_id": lookup["payload"]["receipt_id"]}
        maximum = 9_007_199_254_740_991
        coordinates = (
            ("catalog_revision",),
            ("source_refs", 0, "generation"),
            ("source_refs", 0, "acl_revision"),
            ("validation_basis", "intent_revision"),
            ("validation_basis", "scope_revision"),
            ("validation_basis", "event_cursor", "sequence"),
            ("validation_basis", "max_age_ms"),
        )
        for coordinate in coordinates:
            with self.subTest(coordinate=coordinate):
                candidate = deepcopy(body)
                field = candidate
                for part in coordinate[:-1]:
                    field = field[part]
                field[coordinate[-1]] = maximum
                self.canonical_receipt.validate(candidate)
                field[coordinate[-1]] += 1
                with self.assertRaises(ValidationError):
                    self.canonical_receipt.validate(candidate)

    def test_t_f009_008_exclusive_receipt_representations(self):
        response = fixture("get-discovery-receipt-artifact-response")
        result = response["result"]
        inline = fixture("get-discovery-receipt-valid-response")["result"]["receipt"]
        with self.assertRaises(ValidationError):
            self.discovery.validate({**response, "result": {**result, "receipt": inline}})
        del result["receipt_artifact"]
        with self.assertRaises(ValidationError):
            self.discovery.validate(response)

    def test_t_f009_004_read_artifact_guards_only_the_known_variant(self):
        discovery = fixture("read-discovery-receipt-artifact-query")
        payload = discovery["payload"]
        for mutation in (
            {**discovery, "project_id": None},
            {**discovery, "project_id": "00000000-0000-0000-0000-000000000000"},
            {**discovery, "payload": {**payload, "limit_bytes": 1_048_577}},
            {**discovery, "payload": {**payload, "artifact_ref": {
                **payload["artifact_ref"], "draft_id": payload["artifact_ref"]["receipt_id"],
            }}},
        ):
            with self.subTest(mutation=mutation), self.assertRaises(ValidationError):
                self.query.validate(mutation)

        workflow_query = fixture("read-workflow-draft-artifact-query")
        self.query.validate(workflow_query)
        self.query.validate({
            **discovery,
            "project_id": None,
            "payload": {"artifact_ref": {"kind": "report_export", "id": "report-fixture"}},
        })
        workflow_response = fixture("read-workflow-draft-artifact-response")
        self.workflow_block.validate(workflow_response)
        with self.assertRaises(ValidationError):
            self.discovery_block.validate(workflow_response)
