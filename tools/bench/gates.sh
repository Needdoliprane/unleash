#!/bin/sh
# Gates du dépôt sur une source déjà transmise au banc (voir bench.sh source).
#
# Usage : gates.sh <conteneur> <label> [renderer|rust|all]
#
# Chaque contrôle garde son journal entier dans $UNLEASH_BENCH_LOGS/<label>-<contrôle>.log.
# Tous les contrôles demandés sont lancés ; le résumé donne le code de chacun.
set -eu

bench_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd -P)
bench_sh=$bench_dir/bench.sh
repo=$(git -C "$bench_dir" rev-parse --show-toplevel)
toolchain=1.94.1
container=$1
label=$2
scope=${3:-all}
logs=${UNLEASH_BENCH_LOGS:-$repo/.claude/cache/bench/logs}
results=
mkdir -p "$logs"

summary() {
    printf '\n%s\n' "Résumé des gates pour $label :"
    printf '%-14s %s\n' contrôle code
    for line in $results; do
        printf '%-14s %s\n' "${line%%:*}" "${line##*:}"
    done
}

step() {
    name=$1
    shift
    limit=$1
    shift
    code=0
    UNLEASH_BENCH_SOURCE_LABEL=$label sh "$bench_sh" run "$container" "$label-$name" "$logs/$label-$name.log" "$limit" "$@" || code=$?
    results="$results $name:$code"
    return 0
}

case $scope in
    renderer|rust|all) ;;
    *) sed -n '2,6p' "$0" | sed 's/^# \{0,1\}//' >&2; exit 2 ;;
esac

if [ "$scope" != rust ]; then
    # Mêmes commandes que .github/workflows/ci.yml, avec le magasin préchargé hors réseau.
    # shellcheck disable=SC2086
    step renderer-install 900 pnpm --dir apps/gui-renderer install --frozen-lockfile --offline --store-dir /opt/pnpm-store
    step renderer-typecheck 900 pnpm --dir apps/gui-renderer run typecheck
    step renderer-lint 900 pnpm --dir apps/gui-renderer run lint
    step renderer-test 1800 pnpm --dir apps/gui-renderer test
    step renderer-build 1800 pnpm --dir apps/gui-renderer run build
fi

if [ "$scope" != renderer ]; then
    cd_args="--locked --offline"
    # shellcheck disable=SC2086
    step fmt 900 cargo "+$toolchain" fmt --all -- --check
    # shellcheck disable=SC2086
    step clippy 5400 cargo "+$toolchain" clippy --workspace --all-targets --all-features $cd_args -- -D warnings
    # shellcheck disable=SC2086
    step test 7200 cargo "+$toolchain" test --workspace --all-features $cd_args --no-fail-fast
    # shellcheck disable=SC2086
    step doc 3600 env RUSTDOCFLAGS=-D\ warnings cargo "+$toolchain" doc --workspace --no-deps $cd_args
fi

summary
