#!/usr/bin/env python3
"""Re-project the canonical Markdown sources into the JSON/Markdown projections.

Usage: python3 tools/design-package/sync.py <package_root> [--revision R7]

Sources -> projections:
  features/*/requirements.md + tests.md  -> catalog.requirements, catalog.features[].requirement_ids,
                                           testing/traceability.json, testing/traceability.md (text cells)
  brief/open-questions.md                -> catalog.questions
  architecture/adr/ADR-*.md              -> catalog.decisions (title, status)
  brief/assumptions.md                   -> catalog.assumptions
  plans/risk-register.md                 -> catalog.risks
  brief/invariants.md                    -> catalog.invariants (statement, origin)
  --revision                             -> `revision` of the four projections
Run regen.py afterwards, then validate.py.  Standard library only.
"""
import json
import re
import sys
from collections import Counter
from pathlib import Path

args = [a for a in sys.argv[1:] if not a.startswith("--")]
ROOT = Path(args[0] if args else ".").resolve()
C = ROOT / ".claude"
REVISION = None
if "--revision" in sys.argv:
    REVISION = sys.argv[sys.argv.index("--revision") + 1]
errors = []


def load(rel):
    return json.loads((C / rel).read_text(encoding="utf-8"))


def dump(rel, data):
    (C / rel).write_text(json.dumps(data, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")


def rows(text, id_pattern, ncells):
    """Yield (id, [cells...]) for Markdown table rows whose first cell matches id_pattern."""
    out = {}
    for line in text.splitlines():
        m = re.match(r"^\|\s*(" + id_pattern + r")\s*\|(.*)\|\s*$", line)
        if not m:
            continue
        cells = [c.strip() for c in m.group(2).split(" | ")]
        if len(cells) < ncells:
            cells = [c.strip() for c in m.group(2).split("|")]
        out[m.group(1)] = cells
    return out


cat = load("catalog/design-catalog.json")
tr = load("testing/traceability.json")
ops = load("catalog/operations.json")

# ----------------------------------------------------------------- requirements
old = {r["requirement_id"]: r for r in cat["requirements"]}
new_reqs = []
for f in cat["features"]:
    req_md = (C / f["documents"]["requirements"]).read_text(encoding="utf-8")
    tst_md = (C / f["documents"]["tests"]).read_text(encoding="utf-8")
    tests = {}
    for line in tst_md.splitlines():
        if not line.startswith("| "):
            continue
        cells = [c.strip() for c in line.strip().strip("|").split(" | ")]
        m = re.match(r"^(T-F\d{3}-\d{3})\b", cells[0])
        if not m or len(cells) < 2:
            continue
        if "→" in cells[0]:
            ids_ = re.findall(r"(AC-F\d{3}-\d{3}|EV-F\d{3}-\d{3})", cells[0])
            ac_ = next((x for x in ids_ if x.startswith("AC")), None)
            ev_ = next((x for x in ids_ if x.startswith("EV")), None)
            tests[m.group(1)] = (ac_, ev_, cells[1].strip())
        else:
            ev_m = re.search(r"EV-F\d{3}-\d{3}", cells[-1])
            ac_m = re.search(r"AC-F\d{3}-\d{3}", " ".join(cells[1:-1]))
            tests[m.group(1)] = (ac_m.group(0) if ac_m else None, ev_m.group(0) if ev_m else None, cells[-1].strip())
    ids = []
    archs = Counter(r["architecture"] for r in old.values() if r["feature_id"] == f["id"])
    for line in req_md.splitlines():
        m = re.match(r"^\|\s*(REQ-[A-Z]+-\d{3})\s*\|\s*(.*?)\s*\|(.*)\|\s*$", line)
        if not m:
            continue
        rid, text, rest = m.groups()
        acm = re.search(r"AC-F(\d{3})-(\d{3})", rest)
        if not acm:
            errors.append(f"{rid}: no AC id in requirements row")
            continue
        ac = acm.group(0)
        tm = re.search(r"T-F\d{3}-\d{3}", rest)
        evm = re.search(r"EV-F\d{3}-\d{3}", rest)
        t = tm.group(0) if tm else f"T-F{acm.group(1)}-{acm.group(2)}"
        ev = evm.group(0) if evm else f"EV-F{acm.group(1)}-{acm.group(2)}"
        if t not in tests:
            errors.append(f"{rid}: test row {t} missing in {f['documents']['tests']}")
            continue
        t_ac, t_ev, evidence = tests[t]
        if (t_ac and t_ac != ac) or (t_ev and t_ev != ev):
            errors.append(f"{rid}: chain {ac}/{t}/{ev} differs from tests.md {t_ac}/{t}/{t_ev}")
        base = old.get(rid) or {
            "requirement_id": rid,
            "architecture": archs.most_common(1)[0][0] if archs else "architecture/system-overview.md",
            "feature_id": f["id"], "feature_path": f["path"], "product_test_status": "not_run",
        }
        rec = dict(base)
        rec.update({"requirement": text, "feature_id": f["id"], "feature_path": f["path"],
                    "acceptance_id": ac, "test_id": t, "evidence_id": ev, "evidence_expected": evidence,
                    "product_test_status": "not_run"})
        ordered = {k: rec[k] for k in ("requirement_id", "requirement", "architecture", "feature_id", "feature_path",
                                       "acceptance_id", "test_id", "evidence_id", "evidence_expected", "product_test_status")}
        new_reqs.append(ordered)
        ids.append(rid)
    f["requirement_ids"] = ids
cat["requirements"] = new_reqs
tr["requirements"] = [dict(r) for r in new_reqs]

tmd_path = C / "testing/traceability.md"
tmd = tmd_path.read_text(encoding="utf-8")
by_id = {r["requirement_id"]: r for r in new_reqs}


def fix_row(m):
    rid = m.group(1)
    r = by_id.get(rid)
    if not r:
        errors.append(f"traceability.md row {rid} not in sources")
        return m.group(0)
    return f"| [{rid}]({m.group(2)}) | {r['requirement']} |{m.group(3)}"


tmd = re.sub(r"^\| \[(REQ-[A-Z]+-\d{3})\]\(([^)]+)\) \| .*? \|( \[AC-.*)$", fix_row, tmd, flags=re.M)
missing_rows = set(by_id) - set(re.findall(r"^\| \[(REQ-[A-Z]+-\d{3})\]", tmd, re.M))
if missing_rows:
    errors.append(f"traceability.md lacks rows for {sorted(missing_rows)}")

# ----------------------------------------------------------------- questions
oq = rows((C / "brief/open-questions.md").read_text(encoding="utf-8"), r"Q-\d{2}", 3)
qs = []
for qid in sorted(oq):
    q, rec, owner = (oq[qid] + ["", "", ""])[:3]
    status = "resolved" if q.startswith("Résolue") else "optional_unconfirmed" if q.startswith("Option") else "open"
    qs.append({"id": qid, "status": status, "source": {"path": "brief/open-questions.md", "subject": qid},
               "question": q, "recommendation": rec, "owner_gate": owner})
cat["questions"] = qs

# ----------------------------------------------------------------- decisions
STATUS = {"accepté": "accepted", "proposé": "proposed", "remplacé": "superseded", "supersédé": "superseded"}
old_dec = {d["id"]: d for d in cat["decisions"]}
decs = []
for p in sorted((C / "architecture/adr").glob("ADR-*.md")):
    t = p.read_text(encoding="utf-8")
    did = re.match(r"(ADR-\d{2})", p.name).group(1)
    title = re.search(r"^# (.+)$", t, re.M).group(1).strip()
    sm = re.search(r"Statut\s*:\s*(accepté|proposé|remplacé|supersédé)", t[:1500], re.I)
    status = STATUS[sm.group(1).lower()] if sm else "proposed"
    d = old_dec.get(did, {"related_ids": []})
    decs.append({"id": did, "status": status, "title": title,
                 "source": {"path": f"architecture/adr/{p.name}", "subject": did}, "related_ids": d.get("related_ids", [])})
cat["decisions"] = decs

# ----------------------------------------------------------------- assumptions, risks, invariants
hs = rows((C / "brief/assumptions.md").read_text(encoding="utf-8"), r"H-\d{2}", 3)
old_h = {h["id"]: h for h in cat["assumptions"]}
cat["assumptions"] = [{"id": hid, "status": old_h.get(hid, {}).get("status", "proposed"),
                       "source": {"path": "brief/assumptions.md", "subject": hid},
                       "hypothesis": c[0], "rationale_or_verification": c[1] if len(c) > 1 else "",
                       "reconsider_when": (c[2] if len(c) > 2 and c[2] else None)} for hid, c in sorted(hs.items())]
rs = rows((C / "plans/risk-register.md").read_text(encoding="utf-8"), r"R-\d{2}", 4)
old_r = {r["id"]: r for r in cat["risks"]}
cat["risks"] = [{"id": rid, "status": old_r.get(rid, {}).get("status", "identified"),
                 "source": {"path": "plans/risk-register.md", "subject": rid},
                 "risk": c[0], "hypothesis_refs": c[1] if len(c) > 1 else "", "mitigation": c[2] if len(c) > 2 else "",
                 "verification_owner": c[3] if len(c) > 3 else ""} for rid, c in sorted(rs.items())]
inv = rows((C / "brief/invariants.md").read_text(encoding="utf-8"), r"INV-\d{3}", 3)
cov = {}
for line in (C / "testing/invariant-coverage.md").read_text(encoding="utf-8").splitlines():
    m = re.match(r"^\|\s*(INV-\d{3}) — .*?\|(.*)\|\s*$", line)
    if m:
        cells = [c.strip() for c in m.group(2).split(" | ")]
        if len(cells) >= 6:
            cov[m.group(1)] = cells
for i in cat["invariants"]:
    if i["id"] in inv:
        i["statement"], i["origin"] = inv[i["id"]][0], inv[i["id"]][1]
        if i["id"] in cov:
            _st, spec, comp, feat, acc, lim = cov[i["id"]][:6]
            refs = re.findall(r"\]\(\.\./([^)#]+)", spec)
            if refs:
                i["specification_refs"] = list(dict.fromkeys(refs))
            i["component"], i["feature_text"], i["acceptance_text"], i["limits"] = comp, feat, acc, lim
    else:
        errors.append(f"invariant {i['id']} missing from invariants.md")

# ----------------------------------------------------------------- revision
if REVISION:
    for doc in (cat, tr, ops):
        doc["revision"] = REVISION
    man = load("catalog/package-manifest.json")
    man["revision"] = REVISION
    dump("catalog/package-manifest.json", man)
    tmd = re.sub(r"Révision R\d+\.", f"Révision {REVISION}.", tmd, count=1)

if errors:
    for e in errors[:25]:
        print("ERROR", e)
    print(f"{len(errors)} errors; nothing written")
    sys.exit(1)
dump("catalog/design-catalog.json", cat)
dump("testing/traceability.json", tr)
dump("catalog/operations.json", ops)
tmd_path.write_text(tmd, encoding="utf-8")
print(f"sync: {len(new_reqs)} requirements, {len(qs)} questions ({Counter(q['status'] for q in qs)}), "
      f"{len(decs)} decisions ({Counter(d['status'] for d in decs)}), {len(cat['assumptions'])} assumptions, "
      f"{len(cat['risks'])} risks, revision={cat['revision']}")
