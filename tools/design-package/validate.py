#!/usr/bin/env python3
"""Validate the OVPT Unleash design package (read-only).

Usage: python3 tools/design-package/validate.py <package_root>
Checks: CHECKSUMS.sha256 and manifest (inventory, sizes, SHA-256, tree digest),
JSON without duplicate keys, TOML, JSON Schema 2020-12 validity and fixture
validation (local registry only, when the `jsonschema` package is installed),
local Markdown/HTML links and anchors, catalog <-> Markdown concordance
(requirements, questions, decisions, invariants, risks, assumptions, operations,
parity groups) and cross-referenced identifiers.  Exit code 1 on any error.
"""
import hashlib
import json
import os
import re
import sys
import tomllib
import unicodedata
from collections import Counter, defaultdict
from pathlib import Path
from urllib.parse import unquote

sys.path.insert(0, str(Path(__file__).resolve().parent))
from common import ROOT_DIRS, ROOT_FILES, checksum_scope, load_manifest, package_files, walk  # noqa: E402

ROOT = Path(sys.argv[1] if len(sys.argv) > 1 else ".").resolve()
C = ROOT / ".claude"
errors, warnings, info = [], [], []
err, warn = errors.append, warnings.append


def sha(p):
    return hashlib.sha256(p.read_bytes()).hexdigest()


def no_dup(pairs):
    seen = {}
    for k, v in pairs:
        if k in seen:
            raise ValueError(f"duplicate key {k!r}")
        seen[k] = v
    return seen


man = load_manifest(C)
pkg = package_files(C, man)

# ------------------------------------------------------------------ JSON / TOML
jsons = {}
for rel in pkg + ["catalog/package-manifest.json"]:
    p = C / rel
    if rel.endswith(".json"):
        try:
            jsons[rel] = json.loads(p.read_text(encoding="utf-8"), object_pairs_hook=no_dup)
        except Exception as e:  # noqa: BLE001
            err(f"JSON .claude/{rel}: {e}")
    elif rel.endswith(".toml"):
        try:
            tomllib.loads(p.read_text(encoding="utf-8"))
        except Exception as e:  # noqa: BLE001
            err(f"TOML .claude/{rel}: {e}")
for d in ROOT_DIRS:
    for rel in walk(ROOT / d) if (ROOT / d).is_dir() else []:
        if rel.endswith(".toml"):
            try:
                tomllib.loads((ROOT / d / rel).read_text(encoding="utf-8"))
            except Exception as e:  # noqa: BLE001
                err(f"TOML {d}/{rel}: {e}")

# ------------------------------------------------------------------ CHECKSUMS
scope = checksum_scope(ROOT, man)
ck = ROOT / "CHECKSUMS.sha256"
if ck.exists():
    listed = {}
    for ln in ck.read_text(encoding="utf-8").splitlines():
        if ln.strip():
            h, _, p = ln.partition("  ")
            listed[p] = h
    for p in scope:
        if p not in listed:
            err(f"CHECKSUMS: not listed: {p}")
        elif sha(ROOT / p) != listed[p]:
            err(f"CHECKSUMS: hash mismatch: {p}")
    for p in listed:
        if p not in scope:
            err(f"CHECKSUMS: listed but outside scope or missing: {p}")
    info.append(f"CHECKSUMS entries={len(listed)} scope={len(scope)}")
else:
    err("CHECKSUMS.sha256 missing")

# ------------------------------------------------------------------ manifest
mfiles = {f["path"]: f for f in man["files"]}
if [f["path"] for f in man["files"]] != sorted(mfiles):
    err("manifest: files not sorted or duplicated")
for p in pkg:
    if p not in mfiles:
        err(f"manifest: not listed .claude/{p}")
        continue
    b = (C / p).read_bytes()
    if hashlib.sha256(b).hexdigest() != mfiles[p]["sha256"]:
        err(f"manifest: sha mismatch .claude/{p}")
    if len(b) != mfiles[p]["bytes"]:
        err(f"manifest: size mismatch .claude/{p}")
for p in mfiles:
    if p not in pkg:
        err(f"manifest: listed but missing .claude/{p}")
lst = [{"path": f["path"], "sha256": f["sha256"], "bytes": f["bytes"]} for f in sorted(man["files"], key=lambda x: x["path"])]
dig = hashlib.sha256(json.dumps(lst, separators=(",", ":"), ensure_ascii=False).encode()).hexdigest()
if dig != man["source_tree_digest"]:
    err("manifest: source_tree_digest mismatch")
for p, n in Counter(p.lower() for p in pkg).items():
    if n > 1:
        err(f"case collision: {p}")
for p in pkg:
    if unicodedata.normalize("NFC", p) != p:
        err(f"non-NFC path: {p}")
info.append(f"manifest files={len(mfiles)} package={len(pkg)}")

# ------------------------------------------------------------------ JSON Schema
try:
    from jsonschema import Draft202012Validator, FormatChecker
    from referencing import Registry, Resource
    from referencing.jsonschema import DRAFT202012

    schemas = {rel: d for rel, d in jsons.items() if rel.startswith("schemas/")}

    def refuse(uri):
        raise LookupError(f"external $ref refused: {uri}")

    reg = Registry(retrieve=refuse).with_resources(
        [(s["$id"], Resource.from_contents(s, default_specification=DRAFT202012)) for s in schemas.values()])
    bykey = {}
    for rel, s in schemas.items():
        try:
            Draft202012Validator.check_schema(s)
        except Exception as e:  # noqa: BLE001
            err(f"schema invalid {rel}: {str(e)[:200]}")
        for key in ("schema_version", "protocol_version"):
            c = s.get("properties", {}).get(key, {}).get("const")
            if c is not None:
                bykey[(key, c)] = rel
    typed_ipc_fixtures = {
        "examples/event-terminal-release-disposition.json": ("schemas/event.schema.json", None),
        "examples/ipc-reconcile-terminal-release-attestation.json": ("schemas/ipc-command.schema.json", None),
        "examples/ipc-reconcile-terminal-release-disposition.json": ("schemas/ipc-command.schema.json", None),
        "examples/ipc-read-settings-model-policies-response.json": (
            "schemas/ipc-query-response.schema.json", "read_settings_model_policies_response",
        ),
    }
    ok = 0
    for rel, d in jsons.items():
        if rel.startswith("schemas/"):
            continue
        if (typed_fixture := typed_ipc_fixtures.get(rel)) is not None:
            target, definition = typed_fixture
            selected = schemas.get(target)
            if selected is None:
                err(f"{rel}: missing schema {target}")
                continue
            if definition is not None and definition not in selected.get("$defs", {}):
                err(f"{rel}: missing {target} definition {definition}")
                continue
            if definition is not None:
                selected = {"$ref": f"{selected['$id']}#/$defs/{definition}"}
        else:
            if not isinstance(d, dict):
                continue
            target = next((bykey[(k, d.get(k))] for k in ("schema_version", "protocol_version") if (k, d.get(k)) in bykey), None)
            if target is None:
                continue
            selected = schemas[target]
        v = Draft202012Validator(selected, registry=reg, format_checker=FormatChecker())
        es = sorted(v.iter_errors(d), key=lambda e: list(e.path))
        for e in es[:5]:
            err(f"{rel} !~ {target}: {'/'.join(map(str, e.path))}: {e.message[:200]}")
        ok += not es
    info.append(f"schemas={len(schemas)} documents valid={ok}")
except ImportError:
    warn("jsonschema/referencing not installed: schema validation skipped (pip install jsonschema)")

# ------------------------------------------------------------------ links
LINK = re.compile(r"(?<!\!)\[[^\]]*\]\(([^)\s]+)(?:\s+\"[^\"]*\")?\)")
HREF = re.compile(r"(?:href|src)=\"([^\"]+)\"")
_anchors = {}


def slug(h):
    h = re.sub(r"[^\w\- ]", "", h.strip().lower(), flags=re.UNICODE)
    return h.replace(" ", "-")


def anchors(p):
    if p not in _anchors:
        s = set()
        t = p.read_text(encoding="utf-8")
        if p.suffix == ".md":
            s = {slug(m) for m in re.findall(r"^#{1,6}\s+(.*)$", t, re.M)}
        elif p.suffix == ".html":
            s = set(re.findall(r"id=\"([^\"]+)\"", t))
        _anchors[p] = s
    return _anchors[p]


n_links = 0
for rel in scope:
    p = ROOT / rel
    if p.suffix not in (".md", ".html"):
        continue
    txt = p.read_text(encoding="utf-8")
    if p.suffix == ".md":
        txt = re.sub(r"```.*?```", "", txt, flags=re.S)
        txt = re.sub(r"`[^`\n]*`", "", txt)
    for t in (LINK.findall(txt) if p.suffix == ".md" else HREF.findall(txt)):
        if re.match(r"^[a-z][a-z0-9+.-]*:", t):
            if p.suffix == ".html" and t.startswith(("http:", "https:")):
                warn(f"remote reference in html {rel}: {t}")
            continue
        n_links += 1
        path, _, frag = t.partition("#")
        tgt = (p.parent / unquote(path)).resolve() if path else p
        try:
            tgt.relative_to(ROOT)
        except ValueError:
            err(f"link escapes package: {rel} -> {t}")
            continue
        if not tgt.exists():
            err(f"broken link: {rel} -> {t}")
        elif frag and tgt.is_file() and tgt.suffix in (".md", ".html") and unquote(frag) not in anchors(tgt):
            warn(f"anchor not found: {rel} -> {t}")
info.append(f"local links checked={n_links}")

# ------------------------------------------------------------------ catalog concordance
cat = jsons.get("catalog/design-catalog.json")
tr = jsons.get("testing/traceability.json")
ops = jsons.get("catalog/operations.json")
if cat and tr:
    creq = {r["requirement_id"]: r for r in cat["requirements"]}
    if len(creq) != len(cat["requirements"]):
        err("catalog: duplicate REQ ids")
    if creq != {r["requirement_id"]: r for r in tr["requirements"]}:
        err("catalog.requirements != traceability.requirements")
    feat_reqs = defaultdict(set)
    for f in cat["features"]:
        for v in f["documents"].values():
            if not (C / v).exists():
                err(f"feature doc missing {v}")
        feat_reqs[f["id"]] |= set(f["requirement_ids"])
    for rid, r in creq.items():
        if rid not in feat_reqs[r["feature_id"]]:
            err(f"{rid} not in feature {r['feature_id']} requirement_ids")
        fdir = C / r["feature_path"]
        if rid not in (fdir / "requirements.md").read_text(encoding="utf-8"):
            err(f"{rid} absent from {r['feature_path']}/requirements.md")
        if r["acceptance_id"] not in (fdir / "acceptance.md").read_text(encoding="utf-8"):
            err(f"{r['acceptance_id']} absent from acceptance.md")
        if r["test_id"] not in (fdir / "tests.md").read_text(encoding="utf-8"):
            err(f"{r['test_id']} absent from tests.md")
        if not (C / r["architecture"]).exists():
            err(f"{rid} architecture path missing {r['architecture']}")
    src = set()
    for f in cat["features"]:
        src |= set(re.findall(r"\bREQ-[A-Z]+-\d{3}\b", (C / f["documents"]["requirements"]).read_text(encoding="utf-8")))
    if src != set(creq):
        err(f"requirements.md ids != catalog: {sorted(src ^ set(creq))[:10]}")
    md = set(re.findall(r"\bREQ-[A-Z]+-\d{3}\b", (C / "testing/traceability.md").read_text(encoding="utf-8")))
    if md != set(creq):
        err(f"traceability.md ids != catalog: {sorted(md ^ set(creq))[:10]}")
    adr_files = sorted((C / "architecture/adr").glob("ADR-*.md"))
    if len(adr_files) != len(cat["decisions"]):
        err(f"ADR files {len(adr_files)} != catalog decisions {len(cat['decisions'])}")
    for d in cat["decisions"]:
        p = C / d["source"]["path"]
        if not p.exists():
            err(f"ADR source missing {d['source']['path']}")
            continue
        head = p.read_text(encoding="utf-8")[:600].lower()
        expected = {"accepted": "accepté", "proposed": "proposé"}.get(d["status"])
        if expected and f"statut : {expected}" not in head:
            err(f"{d['id']} status {d['status']} not reflected in its ADR header")
    oq = (C / "brief/open-questions.md").read_text(encoding="utf-8")
    for q in cat["questions"]:
        row = re.search(r"^\| " + re.escape(q["id"]) + r" \|(.*)$", oq, re.M)
        if not row:
            err(f"question {q['id']} missing in open-questions.md")
            continue
        cells = [c.strip() for c in row.group(1).split("|")]
        for i, key in enumerate(("question", "recommendation", "owner_gate")):
            if i < len(cells) and cells[i] != q[key]:
                err(f"{q['id']} {key} differs between open-questions.md and catalog")
    inv = sorted(set(re.findall(r"\bINV-\d{3}\b", (C / "brief/invariants.md").read_text(encoding="utf-8"))))
    if inv != sorted(i["id"] for i in cat["invariants"]):
        err("invariants.md ids != catalog")
    rr = sorted(set(re.findall(r"^\| (R-\d{2}) \|", (C / "plans/risk-register.md").read_text(encoding="utf-8"), re.M)))
    if rr != sorted(r["id"] for r in cat["risks"]):
        err("risk-register ids != catalog")
    hh = sorted(set(re.findall(r"^\| (H-\d{2}) \|", (C / "brief/assumptions.md").read_text(encoding="utf-8"), re.M)))
    if hh != sorted(h["id"] for h in cat["assumptions"]):
        err("assumptions ids != catalog")
    info.append(f"features={len(cat['features'])} REQ={len(creq)} INV={len(cat['invariants'])} ADR={len(cat['decisions'])} "
                f"H={len(cat['assumptions'])} Q={len(cat['questions'])} R={len(cat['risks'])}")
if ops:
    names = [o["name"] for o in ops["operations"]]
    if len(names) != len(set(names)):
        err("operations: duplicate names")

    def find_enum(x):
        if isinstance(x, dict):
            if isinstance(x.get("enum"), list) and len(x["enum"]) > 50:
                return x["enum"]
            for v in x.values():
                r = find_enum(v)
                if r:
                    return r
        elif isinstance(x, list):
            for v in x:
                r = find_enum(v)
                if r:
                    return r
        return None

    enum = find_enum(jsons.get("schemas/operation-name.schema.json"))
    if enum is not None and set(enum) != set(names):
        err(f"operation-name enum != operations: {sorted(set(enum) ^ set(names))[:10]}")
    for g in ops["parity_groups"]:
        for n in g["operation_names"]:
            if n not in names:
                err(f"parity {g['id']} unknown operation {n}")
    par = sorted(set(re.findall(r"\bPAR-[A-Z]+-\d{2}\b", (C / "design/gui-tui-parity.md").read_text(encoding="utf-8"))))
    if par != sorted(g["id"] for g in ops["parity_groups"]):
        err("parity ids gui-tui-parity.md != operations.json")
    com = (C / "architecture/client-operation-catalog.md").read_text(encoding="utf-8")
    missing = [n for n in names if not re.search(r"\b" + re.escape(n) + r"\b", com) and n != "Hello"]
    if missing:
        err(f"operations absent from client-operation-catalog.md: {missing[:10]}")
    info.append(f"operations={len(names)} parity_groups={len(ops['parity_groups'])}")

# ------------------------------------------------------------------ identifiers
if cat and ops:
    known = {
        "INV": {i["id"] for i in cat["invariants"]}, "REQ": {r["requirement_id"] for r in cat["requirements"]},
        "Q": {q["id"] for q in cat["questions"]}, "H": {h["id"] for h in cat["assumptions"]},
        "R": {r["id"] for r in cat["risks"]}, "ADR": {d["id"] for d in cat["decisions"]},
        "PAR": {g["id"] for g in ops["parity_groups"]}, "F": {f["id"] for f in cat["features"]},
    }
    defined = {"AC": set(), "T": set(), "EV": set()}
    for f in cat["features"]:
        acc = (C / f["documents"]["acceptance"]).read_text(encoding="utf-8")
        tst = (C / f["documents"]["tests"]).read_text(encoding="utf-8")
        defined["AC"] |= set(re.findall(r"\bAC-F\d{3}-\d{3}\b", acc))
        defined["T"] |= set(re.findall(r"\bT-F\d{3}-\d{3}\b", tst))
        defined["EV"] |= set(re.findall(r"\bEV-F\d{3}-\d{3}\b", acc + tst))
    pats = {"INV": r"\bINV-\d{3}\b", "REQ": r"\bREQ-[A-Z]+-\d{3}\b", "Q": r"\bQ-\d{2}\b", "H": r"\bH-\d{2}\b",
            "R": r"\bR-\d{2}\b", "ADR": r"\bADR-\d{2}\b", "PAR": r"\bPAR-[A-Z]+-\d{2}\b", "F": r"\bF\d{3}\b",
            "AC": r"\bAC-F\d{3}-\d{3}\b", "T": r"\bT-F\d{3}-\d{3}\b", "EV": r"\bEV-F\d{3}-\d{3}\b"}
    allowed_negative = {("T", "T-F001-999"), ("F", "F020")}  # intentional counter-examples
    for rel in scope:
        if not rel.endswith((".md", ".json")):
            continue
        t = (ROOT / rel).read_text(encoding="utf-8")
        for k, pat in pats.items():
            ref = known.get(k) or defined[k]
            for m in set(re.findall(pat, t)):
                if m not in ref and (k, m) not in allowed_negative:
                    err(f"undefined identifier {m} in {rel}")

# ------------------------------------------------------------------ revision coherence
revs = Counter(d["revision"] for d in jsons.values() if isinstance(d, dict) and "revision" in d)
if len(revs) > 1:
    err(f"inconsistent revisions in JSON projections: {dict(revs)}")
info.append(f"revisions={dict(revs)}")
info.append(f"package files={len(pkg)} md={sum(p.endswith('.md') for p in pkg)} json={sum(p.endswith('.json') for p in pkg)}")

for m in info:
    print("INFO ", m)
for m in warnings:
    print("WARN ", m)
for m in errors:
    print("ERROR", m)
print(f"== {len(errors)} errors, {len(warnings)} warnings")
sys.exit(1 if errors else 0)
