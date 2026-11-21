#!/usr/bin/env python3
"""Render the tracked development overview from the existing goal files."""

import argparse
import re
from pathlib import Path
from tempfile import NamedTemporaryFile


STATE = Path(".claude/state")
PROGRAM = "g0000-v1"
OPEN = {"in_progress", "blocked", "paused"}
UPCOMING = {"pending", "planned"}
STATUS_LABELS = {
    "in_progress": "En cours",
    "blocked": "Bloqué",
    "paused": "En pause",
    "pending": "À venir",
    "planned": "Planifié",
    "done": "Clos",
}
HEADING = re.compile(r"^(#{2,6}) (.+)$", re.MULTILINE)
STATUS = re.compile(r"^- Statut\s*:\s*(\w+)\s*$", re.MULTILINE)


def section(text, title):
    headings = list(HEADING.finditer(text))
    for index, heading in enumerate(headings):
        if heading[2] == title:
            following = headings[index + 1:]
            end = next((h.start() for h in following if len(h[1]) <= len(heading[1])), len(text))
            return text[heading.end():end].strip()
    return ""


def goal_status(text, name):
    match = STATUS.search(text.split("\n## ", 1)[0])
    if not match or match[1] not in STATUS_LABELS:
        raise ValueError(f"{name} a un statut absent ou inconnu")
    return match[1]


def program_rows(text):
    plan = section(text, "Plan du programme (Sous-goals)")
    rows = [line for line in plan.splitlines() if line.startswith("|")]
    if len(rows) < 3:
        raise ValueError("Le plan V1 ne contient aucun jalon")
    seen = set()
    for row in rows[2:]:
        cells = [cell.strip() for cell in row.strip("|").split("|")]
        if len(cells) != 5:
            raise ValueError(f"Ligne de jalon invalide dans {PROGRAM}")
        name, milestone, objective, _, status = cells
        if name in seen or status not in STATUS_LABELS:
            raise ValueError(f"Entrée de jalon incohérente pour {name}")
        seen.add(name)
        yield name, milestone, objective, status


def render(root):
    goals = root / STATE / "goals"
    files = {
        path.parent.name: path.read_text(encoding="utf-8")
        for path in sorted(goals.glob("*/goal.md"))
    }
    program = files.pop(PROGRAM, None)
    if program is None:
        raise ValueError(f"Fiche du programme {PROGRAM} introuvable")
    rows = list(program_rows(program))
    for name, _, _, status in rows:
        source = files.get(name)
        if source is None:
            if status not in UPCOMING:
                raise ValueError(f"Fiche manquante pour le jalon {name}")
        else:
            recorded = goal_status(source, name)
            if recorded != status and {recorded, status} - UPCOMING:
                raise ValueError(f"Les statuts de la fiche et du programme diffèrent pour {name}")

    output = [
        "# Travaux en cours et à venir",
        "",
        "Vue générée à partir des fiches `goal.md`. Les décisions et preuves sont conservées dans leurs journaux.",
        "",
        f"[Programme V1](goals/{PROGRAM}/goal.md) · {sum(row[3] == 'done' for row in rows)}/{len(rows)} jalons clos",
        "",
        "## Travaux en cours",
    ]
    open_rows = [row for row in rows if row[3] in OPEN]
    in_program = {row[0] for row in rows}
    other_goals = [
        (name, source.splitlines()[0].removeprefix("# "), goal_status(source, name))
        for name, source in files.items()
        if name not in in_program
    ]
    for name, milestone, objective, status in open_rows:
        output.extend(["", f"### {milestone} — {STATUS_LABELS[status]}", "", objective])
        append_actions(output, name, files[name])
    for name, title, status in other_goals:
        if status in OPEN:
            output.extend(["", f"### {title} — {STATUS_LABELS[status]}"])
            append_actions(output, name, files[name])
    output.extend(["", "## Travaux à venir", ""])
    for name, milestone, objective, status in rows:
        if status in UPCOMING:
            output.append(f"- **{milestone}** — {objective}")
    for name, title, status in other_goals:
        if status in UPCOMING:
            output.append(f"- [{title}](goals/{name}/goal.md) — {STATUS_LABELS[status]}")
    return "\n".join(output).rstrip() + "\n"


def append_actions(output, name, source):
    output.extend(["", f"[Fiche](goals/{name}/goal.md) · [Journal](goals/{name}/journal.md)"])
    actions = section(source, "Actions de clôture ouvertes")
    if actions:
        output.extend(["", actions])


def replace_view(target, content):
    with NamedTemporaryFile(
        dir=target.parent, prefix=".current-state-", mode="w", encoding="utf-8", delete=False
    ) as output:
        temporary = Path(output.name)
        try:
            output.write(content)
            output.close()
            temporary.replace(target)
        finally:
            temporary.unlink(missing_ok=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("root", type=Path, nargs="?", default=Path("."))
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    target = args.root / STATE / "current-state.md"
    try:
        content = render(args.root)
        current = target.read_text(encoding="utf-8") if target.is_file() else None
        if current == content:
            print(f"{target} à jour")
        elif args.check:
            parser.exit(1, f"{target} obsolète ou absent\n")
        else:
            replace_view(target, content)
            print(f"{target} actualisé")
    except (OSError, ValueError) as error:
        parser.exit(1, f"{error}\n")


if __name__ == "__main__":
    main()
