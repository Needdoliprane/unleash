#!/bin/sh
# Construit l'image du banc Docker Unleash (Dockerfile de ce répertoire).
#
# Usage : build.sh [étiquette]
#
# L'étiquette par défaut est UNLEASH_BENCH_IMAGE, sinon celle de bench.sh.
# Le contexte est assemblé sous .claude/cache/bench/image à partir de Git, jamais d'une copie
# de travail : le Dockerfile et, dans context/ et context-vitest4/, package.json et pnpm-lock.yaml
# d'apps/gui-renderer aux révisions de UNLEASH_BENCH_LOCK_REVISIONS (une ou deux, séparées par
# des espaces ; HEAD par défaut). Le magasin pnpm de l'image reçoit les paquets de chaque verrou.
# C'est la seule étape du banc qui accède au réseau (dépôts Debian, registres d'images, registre
# npm) ; les conteneurs restent sans réseau.
set -eu

# Une seule lecture du script, en mémoire : une modification du fichier ne touche que les invocations futures.
# Le drapeau porte le chemin du script : un script lancé par un autre (gates.sh vers bench.sh) se relit aussi.
if [ "${UNLEASH_BENCH_IN_MEMORY:-}" != "$0" ] && [ -f "$0" ]; then
    export UNLEASH_BENCH_IN_MEMORY="$0"
    exec sh -c "$(cat -- "$0")" "$0" "$@"
fi

bench_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd -P)
repo=$(git -C "$bench_dir" rev-parse --show-toplevel)
context=desktop-linux
image=${1:-${UNLEASH_BENCH_IMAGE:-unleash-bench:1.94.1-bookworm-20261124}}
bench_label=${UNLEASH_BENCH_LABEL:-tools/bench}
lock_revisions=${UNLEASH_BENCH_LOCK_REVISIONS:-HEAD}

usage() {
    sed -n '2,4p' "$0" | sed 's/^# \{0,1\}//' >&2
    exit 2
}

[ $# -le 1 ] || usage

# Chaque révision est résolue en commit complet : une révision inconnue arrête ici, avant docker build.
revisions=
# shellcheck disable=SC2086
for revision in $lock_revisions; do
    commit=$(git -C "$repo" rev-parse --verify "$revision^{commit}") || exit 1
    revisions="$revisions $commit"
done
# shellcheck disable=SC2086
set -- $revisions
[ $# -ge 1 ] && [ $# -le 2 ] || { printf '%s\n' "UNLEASH_BENCH_LOCK_REVISIONS attend une ou deux révisions Git." >&2; exit 2; }
[ $# -eq 2 ] || set -- "$1" "$1"

seed() {
    # $1 commit, $2 répertoire du contexte.
    mkdir -p "$2"
    git -C "$repo" show "$1:apps/gui-renderer/package.json" > "$2/package.json"
    git -C "$repo" show "$1:apps/gui-renderer/pnpm-lock.yaml" > "$2/pnpm-lock.yaml"
}

context_dir=$repo/.claude/cache/bench/image
rm -rf -- "$context_dir"
mkdir -p "$context_dir"
cp "$bench_dir/Dockerfile" "$context_dir/Dockerfile"
seed "$1" "$context_dir/context"
seed "$2" "$context_dir/context-vitest4"
printf 'contexte : %s -> context/ ; %s -> context-vitest4/\n' "$1" "$2"

# L'assistant d'identifiants de Docker Desktop (docker-credential-desktop) vit hors du PATH habituel.
PATH="$PATH:/Applications/Docker.app/Contents/Resources/bin" \
    timeout --kill-after=5s "${UNLEASH_BENCH_BUILD_LIMIT:-3600s}" \
    docker --context "$context" build --label io.unleash.bench="$bench_label" --tag "$image" "$context_dir"
printf '%s\n' "Image $image construite (contexte : $context_dir)."
