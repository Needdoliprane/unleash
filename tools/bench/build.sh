#!/bin/sh
# Construit l'image du banc Docker Unleash (Dockerfile de ce répertoire).
#
# Usage : build.sh [étiquette]
#
# L'étiquette par défaut est UNLEASH_BENCH_IMAGE, sinon celle de bench.sh.
# Le contexte minimal est assemblé sous .claude/cache/bench/image : le Dockerfile et,
# dans context/, package.json et pnpm-lock.yaml d'apps/gui-renderer, qui restent leurs
# sources uniques. C'est la seule étape du banc qui accède au réseau (dépôts Debian,
# registres d'images, registre npm) ; les conteneurs restent sans réseau.
set -eu

bench_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd -P)
repo=$(git -C "$bench_dir" rev-parse --show-toplevel)
context=desktop-linux
image=${1:-${UNLEASH_BENCH_IMAGE:-unleash-bench:1.94.1-bookworm-20261124}}

usage() {
    sed -n '2,4p' "$0" | sed 's/^# \{0,1\}//' >&2
    exit 2
}

[ $# -le 1 ] || usage

context_dir=$repo/.claude/cache/bench/image
rm -rf -- "$context_dir"
mkdir -p "$context_dir/context"
cp "$bench_dir/Dockerfile" "$context_dir/Dockerfile"
cp "$repo/apps/gui-renderer/package.json" "$repo/apps/gui-renderer/pnpm-lock.yaml" "$context_dir/context/"

# L'assistant d'identifiants de Docker Desktop (docker-credential-desktop) vit hors du PATH habituel.
PATH="$PATH:/Applications/Docker.app/Contents/Resources/bin" \
    timeout --kill-after=5s "${UNLEASH_BENCH_BUILD_LIMIT:-3600s}" \
    docker --context "$context" build --tag "$image" "$context_dir"
printf '%s\n' "Image $image construite (contexte : $context_dir)."
