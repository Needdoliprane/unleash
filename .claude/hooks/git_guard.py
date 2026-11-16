#!/usr/bin/env python3
"""PreToolUse hook (Bash) for the Unleash repository.

Policy (see .claude/rules/worktrees.md and .claude/dev/goal-protocol.md):
  * `git worktree add|move` must target a direct child of <repo>/.claude/worktrees/  -> deny otherwise
  * `git push` other than a plain push to `dev`, `git reset --hard`, `git clean -f*`, deleting main -> ask
Everything else passes through unchanged (exit 0, no decision).
Handles wrappers such as `PATH=... rtk proxy git ...`, `env`, `command`, and nested `sh -c '...'`.
Standard library only.
"""
import json
import os
import shlex
import subprocess
import sys

OPERATORS = {";", "&&", "||", "|", "&", "(", ")", "\n"}
SHELLS = {"sh", "bash", "zsh", "dash", "ksh"}
GIT_GLOBAL_WITH_VALUE = {"-C", "-c", "--git-dir", "--work-tree", "--namespace", "--exec-path", "--config-env"}
WT_ADD_WITH_VALUE = {"-b", "-B", "--reason"}


def decision(kind, reason):
    print(json.dumps({"hookSpecificOutput": {"hookEventName": "PreToolUse", "permissionDecision": kind, "permissionDecisionReason": reason}}, ensure_ascii=False))
    sys.exit(0)


def split(command):
    lex = shlex.shlex(command, posix=True, punctuation_chars=";&|()")
    lex.whitespace_split = True
    lex.commenters = ""
    return list(lex)


def segments(tokens):
    seg = []
    for t in tokens:
        if t in OPERATORS or (t and set(t) <= set(";&|()")):
            if seg:
                yield seg
            seg = []
        else:
            seg.append(t)
    if seg:
        yield seg


def main_root(cwd):
    try:
        out = subprocess.run(["git", "-C", cwd, "rev-parse", "--path-format=absolute", "--git-common-dir"],
                             capture_output=True, text=True, timeout=5)
        if out.returncode == 0:
            common = out.stdout.strip()
            if common.endswith("/.git"):
                return os.path.dirname(common)
    except Exception:
        pass
    return os.environ.get("CLAUDE_PROJECT_DIR") or cwd


def check_target(path, base_cwd):
    target = os.path.realpath(os.path.join(base_cwd, os.path.expanduser(path)))
    allowed = os.path.realpath(os.path.join(main_root(base_cwd), ".claude", "worktrees"))
    if os.path.dirname(target) != allowed:
        decision("deny", f"Worktree refusé : {target} n'est pas un sous-dossier direct de {allowed}. "
                         "Règle du dépôt : worktrees uniquement sous .claude/worktrees/<nom> (voir .claude/rules/worktrees.md).")


PUSH_FORBIDDEN = {"-f", "--force", "--force-with-lease", "--force-if-includes", "--delete", "-d", "--mirror",
                  "--all", "--tags", "--prune", "--follow-tags"}


def check_push(rest):
    """Allow `git push [-u|--set-upstream] [origin] dev|HEAD:dev` only; ask otherwise."""
    opts = [o for o in rest if o.startswith("-")]
    pos = [o for o in rest if not o.startswith("-")]
    bad = [o for o in opts if o in PUSH_FORBIDDEN or o.startswith(("--force", "--delete", "--mirror", "--push-option"))]
    allowed_opts = {"-u", "--set-upstream", "--no-verify", "-q", "--quiet", "--verbose", "-v", "--porcelain", "--atomic"}
    refspecs = pos[1:] if pos else []
    targets = [r.split(":", 1)[-1].lstrip("+").removeprefix("refs/heads/") for r in refspecs]
    forced = any(r.startswith("+") for r in refspecs)
    if not bad and not forced and all(o in allowed_opts for o in opts) and targets and all(x == "dev" for x in targets):
        return
    decision("ask", "Seul `git push [-u] origin dev` (sans force) est autorisé aux agents ; tout autre push (main, tags, "
                    "force, suppression) exige une demande explicite de l'utilisateur.")


def analyze_git(args, cwd):
    i, run_cwd = 0, cwd
    while i < len(args) and args[i].startswith("-"):
        a = args[i]
        if a == "-C" and i + 1 < len(args):
            run_cwd = os.path.join(run_cwd, os.path.expanduser(args[i + 1]))
            i += 2
            continue
        if a in GIT_GLOBAL_WITH_VALUE:
            i += 2
            continue
        i += 1
    if i >= len(args):
        return
    sub, rest = args[i], args[i + 1:]
    if sub == "worktree" and rest:
        action, opts = rest[0], rest[1:]
        if action == "add":
            j = 0
            while j < len(opts):
                o = opts[j]
                if o == "--":
                    j += 1
                    break
                if o in WT_ADD_WITH_VALUE:
                    j += 2
                    continue
                if o.startswith("-"):
                    j += 1
                    continue
                break
            if j < len(opts):
                check_target(opts[j], run_cwd)
        elif action == "move":
            positional = [o for o in opts if not o.startswith("-")]
            if len(positional) >= 2:
                check_target(positional[1], run_cwd)
    elif sub == "push":
        check_push(rest)
    elif sub == "reset" and "--hard" in rest:
        decision("ask", "git reset --hard peut détruire du travail : confirmation de l'utilisateur requise.")
    elif sub == "clean" and any(o.startswith("-") and "f" in o.lstrip("-") for o in rest if not o.startswith("--")):
        decision("ask", "git clean -f supprime des fichiers non suivis : confirmation de l'utilisateur requise.")
    elif sub == "branch" and any(o in ("-D", "--delete", "-d") for o in rest) and any(o in ("main", "master") for o in rest):
        decision("ask", "Suppression de la branche principale : confirmation de l'utilisateur requise.")


def analyze(command, cwd, depth=0):
    if depth > 4:
        return
    try:
        tokens = split(command)
    except ValueError:
        if "worktree" in command and ("add" in command or "move" in command):
            decision("ask", "Commande worktree non analysable : vérifier que la destination est sous .claude/worktrees/.")
        return
    for seg in segments(tokens):
        for k, tok in enumerate(seg):
            base = os.path.basename(tok)
            if base in SHELLS and "-c" in seg[k + 1:]:
                c = seg.index("-c", k + 1)
                if c + 1 < len(seg):
                    analyze(seg[c + 1], cwd, depth + 1)
                break
            if base == "git":
                analyze_git(seg[k + 1:], cwd)
                break


def main():
    try:
        data = json.load(sys.stdin)
    except Exception:
        sys.exit(0)
    if data.get("tool_name") != "Bash":
        sys.exit(0)
    command = (data.get("tool_input") or {}).get("command") or ""
    cwd = data.get("cwd") or os.environ.get("CLAUDE_PROJECT_DIR") or os.getcwd()
    if "git" in command:
        analyze(command, cwd)
    sys.exit(0)


if __name__ == "__main__":
    main()
