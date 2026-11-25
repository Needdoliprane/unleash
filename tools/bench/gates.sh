#!/bin/sh
# Gates du dépôt sur une source déjà transmise au banc (voir bench.sh source).
#
# Usage : gates.sh <conteneur> <label> [renderer|rust|all]
#
# Chaque contrôle garde son journal entier dans $UNLEASH_BENCH_LOGS/<label>-<contrôle>.log.
# Tous les contrôles demandés sont lancés, sauf les contrôles renderer qui suivent une
# installation en échec (un `resync` conserve `node_modules` : un test vert aurait pu tourner
# sous l'ancienne version). Le résumé donne le code de chacun ; le code de sortie du script est
# 0 si tous les contrôles sont verts, 1 sinon (échec ou non exécuté).
set -eu

# Une seule lecture du script, en mémoire : une modification du fichier ne touche que les invocations futures.
# Le drapeau porte le chemin du script : un script lancé par un autre (gates.sh vers bench.sh) se relit aussi.
if [ "${UNLEASH_BENCH_IN_MEMORY:-}" != "$0" ] && [ -f "$0" ]; then
    export UNLEASH_BENCH_IN_MEMORY="$0"
    exec sh -c "$(cat -- "$0")" "$0" "$@"
fi

bench_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd -P)
bench_sh=$bench_dir/bench.sh
repo=$(git -C "$bench_dir" rev-parse --show-toplevel)
toolchain=1.94.1
container=$1
label=$2
scope=${3:-all}
logs=${UNLEASH_BENCH_LOGS:-$repo/.claude/cache/bench/logs}
results=
failed=0
code=0
mkdir -p "$logs"

record() {
    results="$results $1:$2"
}

step() {
    name=$1
    shift
    limit=$1
    shift
    code=0
    UNLEASH_BENCH_SOURCE_LABEL=$label sh "$bench_sh" run "$container" "$label-$name" "$logs/$label-$name.log" "$limit" "$@" || code=$?
    record "$name" "$code"
    [ "$code" -eq 0 ] || failed=1
    return 0
}

skip() {
    printf '%s non exécuté : %s\n' "$1" "$2" >&2
    record "$1" "non-execute"
    failed=1
}

summary() {
    printf '\n%s\n' "Résumé des gates pour $label :"
    printf '%-18s %s\n' contrôle code
    for line in $results; do
        printf '%-18s %s\n' "${line%%:*}" "${line##*:}"
    done
}

case $scope in
    renderer|rust|all) ;;
    *) sed -n '2,9p' "$0" | sed 's/^# \{0,1\}//' >&2; exit 2 ;;
esac

if [ "$scope" != rust ]; then
    # Mêmes commandes que .github/workflows/ci.yml, avec le magasin préchargé hors réseau.
    step renderer-install 900 pnpm --dir apps/gui-renderer install --frozen-lockfile --offline --store-dir /opt/pnpm-store
    if [ "$code" -eq 0 ]; then
        step renderer-typecheck 900 pnpm --dir apps/gui-renderer run typecheck
        step renderer-lint 900 pnpm --dir apps/gui-renderer run lint
        step renderer-test 1800 pnpm --dir apps/gui-renderer test
        step renderer-build 1800 pnpm --dir apps/gui-renderer run build
    else
        skip renderer-typecheck 'installation du renderer en echec'
        skip renderer-lint 'installation du renderer en echec'
        skip renderer-test 'installation du renderer en echec'
        skip renderer-build 'installation du renderer en echec'
    fi
fi

if [ "$scope" != renderer ]; then
    cd_args="--locked --offline"
    # shellcheck disable=SC2086
    step fmt 900 cargo "+$toolchain" fmt --all -- --check
    # shellcheck disable=SC2086
    step clippy 5400 cargo "+$toolchain" clippy --workspace --all-targets --all-features $cd_args -- -D warnings
    # shellcheck disable=SC2086
    step test 10800 cargo "+$toolchain" test --workspace --all-features $cd_args --no-fail-fast
    # shellcheck disable=SC2086
    step doc 3600 env RUSTDOCFLAGS=-D\ warnings cargo "+$toolchain" doc --workspace --no-deps $cd_args
fi

summary
exit "$failed"
