#!/bin/sh
# Banc de qualification Unleash : sources transmises depuis Git vers un conteneur isolé.
#
# Usage :
#   bench.sh preflight <conteneur>
#   bench.sh create <conteneur> [--cpus N] [--memory T]
#   bench.sh source <conteneur> <label> <révision-git>
#   bench.sh resync <conteneur> <label> <révision-git>
#   bench.sh renderer <conteneur> <label> <journal> <délai-s> <script-pnpm...>
#   bench.sh cargo <conteneur> <label> <journal> <délai-s> <arguments-cargo...>
#   bench.sh run <conteneur> <label> <journal> <délai-s> <commande-invitée...>
#   bench.sh shell <conteneur> <commande...>
#
# Le conteneur n'a pas de réseau, utilise init Docker, un registre Cargo hôte en
# lecture seule et un target Cargo dans un volume Docker (jamais sur le partage SMB).
# `RUSTFLAGS=-D warnings` est fixé comme dans la CI.
set -eu

# Racine du dépôt (checkout ou worktree) qui contient ce script : les sources en sont lues par git archive.
here=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd -P)
repo=$(git -C "$here" rev-parse --show-toplevel)
context=desktop-linux
socket=$HOME/.docker/run/docker.sock
registry=$HOME/.cargo/registry
image=${UNLEASH_BENCH_IMAGE:-unleash-bench:1.94.1-bookworm-20261124}
toolchain=1.94.1
bench_label=${UNLEASH_BENCH_LABEL:-tools/bench}

usage() {
    sed -n '2,17p' "$0" | sed 's/^# \{0,1\}//' >&2
    exit 2
}

dock() {
    limit=$1
    shift
    timeout --kill-after=5s "$limit" docker --context "$context" "$@"
}

api_ok() {
    [ "$(dock 8s context inspect "$context" --format '{{.Endpoints.docker.Host}}')" = "unix://$socket" ] || return 1
    [ "$(curl --connect-timeout 2 --max-time 5 --unix-socket "$socket" -fsS http://localhost/_ping 2>/dev/null)" = OK ]
}

require_bench() {
    api_ok || { printf '%s\n' "L'API Docker ne répond pas sur $socket." >&2; exit 3; }
    state=$(dock 8s inspect --format '{{.State.Status}} {{.HostConfig.NetworkMode}}' "$1")
    [ "$state" = 'running none' ] || {
        printf 'Conteneur %s dans l état %s ; banc arrêté ou réseau inattendu, réconciliation requise.\n' "$1" "$state" >&2
        exit 3
    }
}

# Le démon exige une colonne PID dès que des options ps sont fournies à docker top.
require_idle() {
    processes=$(dock 12s top "$1" -eo pid,comm)
    printf '%s\n' "$processes" | awk '
        NR == 1 { next }
        /^[[:space:]]*[0-9]+[[:space:]]+(docker-init|tini|sleep)[[:space:]]*$/ { next }
        { exit 1 }
    ' || { printf 'Processus à réconcilier dans le banc %s :\n%s\n' "$1" "$processes" >&2; exit 3; }
}

write_probe() {
    dir=$1
    # shellcheck disable=SC2016
    dock 15s exec "$bench" /bin/sh -eu -c '
        mkdir -p "$1"
        marker=$(mktemp "$1/.bench-write.XXXXXX")
        printf "%s\n" ready > "$marker"
        [ "$(cat "$marker")" = ready ]
        rm -- "$marker"
    ' sh "$dir"
}

cmd_preflight() {
    [ $# -eq 1 ] || usage
    bench=$1
    require_bench "$bench"
    require_idle "$bench"
    # shellcheck disable=SC2016
    dock 12s exec "$bench" /bin/sh -eu -c '
        printf "Commande exécutée dans le banc : rustc %s, git %s, node %s, pnpm %s\n" \
            "$(rustc +"$1" --version | cut -d" " -f2)" "$(git --version | cut -d" " -f3)" \
            "$(node --version)" "$(pnpm --version)"
    ' sh "$toolchain"
    for dir in /work /tmp /cargo-target /cargo-home; do
        write_probe "$dir"
    done
    [ "$(dock 10s exec "$bench" readlink /cargo-home/registry)" = /usr/local/cargo/registry ] || {
        printf '%s\n' 'Le registre Cargo du banc ne pointe pas vers le montage en lecture seule.' >&2
        exit 3
    }
    dock 10s exec "$bench" /bin/sh -eu -c 'pkg-config --exists gdk-3.0 webkit2gtk-4.1 && printf "%s\n" "GTK3 et WebKitGTK présents."'
    printf '%s\n' "Prévol du banc $bench réussi : commande exécutée, écritures vérifiées."
}

cmd_create() {
    [ $# -ge 1 ] || usage
    name=$1
    shift
    cpus=4
    memory=20g
    while [ $# -gt 0 ]; do
        case $1 in
            --cpus) cpus=$2; shift 2 ;;
            --memory) memory=$2; shift 2 ;;
            *) usage ;;
        esac
    done
    api_ok
    dock 120s run --detach --name "$name" --label io.unleash.bench="$bench_label" \
        --init --network none --cpus "$cpus" --memory "$memory" --pids-limit 4096 \
        --mount "type=bind,source=$registry,target=/usr/local/cargo/registry,readonly" \
        --mount "type=volume,source=unleash-bench-$name-cargo-home,target=/cargo-home" \
        --mount "type=volume,source=unleash-bench-$name-target,target=/cargo-target" \
        "$image" sleep infinity
    dock 30s exec "$name" /bin/sh -eu -c '
        mkdir -p /work /cargo-home /cargo-target
        ln -sfn /usr/local/cargo/registry /cargo-home/registry
    '
    printf '%s\n' "Conteneur $name créé (réseau aucun, $cpus CPU, $memory)."
}

archive_into() {
    # $1 conteneur, $2 répertoire cible, $3 commit : extrait les chemins suivis en actualisant les dates.
    # shellcheck disable=SC2016
    git -C "$repo" archive --format=tar "$3" \
        Cargo.toml Cargo.lock rust-toolchain.toml rustfmt.toml clippy.toml deny.toml \
        crates apps .claude ':(exclude).claude/state' \
        | dock 600s exec -i "$1" /bin/sh -eu -c 'tar -m -x -C "$1"' sh "$2"
}

cmd_source() {
    [ $# -eq 3 ] || usage
    bench=$1 label=$2 rev=$3
    require_bench "$bench"
    require_idle "$bench"
    commit=$(git -C "$repo" rev-parse --verify "$rev^{commit}")
    dir=/work/src-$label
    # Chemins suivis seulement : ni node_modules, ni dist, ni état, ni preuves.
    # shellcheck disable=SC2016
    dock 15s exec "$bench" /bin/sh -eu -c '[ ! -e "$1" ] || { printf "%s\n" "Source déjà présente : $1" >&2; exit 3; }' sh "$dir"
    # shellcheck disable=SC2016
    dock 15s exec "$bench" /bin/sh -eu -c 'mkdir -p "$1"' sh "$dir"
    archive_into "$bench" "$dir" "$commit"
    printf 'source %s commit=%s dir=%s\n' "$label" "$commit" "$dir"
}

cmd_resync() {
    # Révision suivante d'une source déjà transférée : remplace l'arbre, garde node_modules et dist.
    [ $# -eq 3 ] || usage
    bench=$1 label=$2 rev=$3
    require_bench "$bench"
    require_idle "$bench"
    commit=$(git -C "$repo" rev-parse --verify "$rev^{commit}")
    dir=/work/src-$label
    # shellcheck disable=SC2016
    dock 60s exec "$bench" /bin/sh -eu -c '
        src=$1 keep=$1.keep
        [ -d "$src" ] || { printf "%s\n" "Source absente : $src" >&2; exit 3; }
        rm -rf "$keep"
        mkdir -p "$keep"
        for name in node_modules dist; do
            if [ -d "$src/apps/gui-renderer/$name" ]; then
                mv "$src/apps/gui-renderer/$name" "$keep/$name"
            fi
        done
        rm -rf "$src"
        mkdir -p "$src"
    ' sh "$dir"
    archive_into "$bench" "$dir" "$commit"
    # shellcheck disable=SC2016
    dock 30s exec "$bench" /bin/sh -eu -c '
        src=$1 keep=$1.keep
        mkdir -p "$src/apps/gui-renderer"
        for name in node_modules dist; do
            if [ -d "$keep/$name" ]; then
                mv "$keep/$name" "$src/apps/gui-renderer/$name"
            fi
        done
        rmdir "$keep"
    ' sh "$dir"
    printf 'resync %s commit=%s dir=%s\n' "$label" "$commit" "$dir"
}

run_logged() {
    # $1 conteneur, $2 label, $3 journal, $4 délai, $5 répertoire, reste : commande invitée.
    bench=$1 label=$2 log=$3 limit=$4 dir=$5
    shift 5
    require_idle "$bench"
    status=0
    # shellcheck disable=SC2016
    dock $((limit + 60))s exec "$bench" /bin/sh -c '
        cd "$1" || exit 3
        label=$2 limit=$3
        shift 3
        export CARGO_HOME=/cargo-home CARGO_TARGET_DIR=/cargo-target CARGO_TERM_COLOR=never
        export CI=1
        export RUSTFLAGS="-D warnings"
        timeout --kill-after=20s "$limit" "$@"
        status=$?
        # 0 : succès ; 1 : fmt --check, clippy ou test en échec ; 101 : erreur cargo.
        case "$status" in
            0|1|101) printf "__UNLEASH_BENCH_TERMINAL__ %s %s\n" "$label" "$status" ;;
        esac
        exit "$status"
    ' sh "$dir" "$label" "$limit" "$@" > "$log" 2>&1 || status=$?
    marker="__UNLEASH_BENCH_TERMINAL__ $label $status"
    count=$(grep -Fxc "$marker" "$log" || :)
    if [ "$count" != 1 ]; then
        printf 'Issue inconnue pour %s : code %s, marqueur %s fois, journal %s\n' "$label" "$status" "$count" "$log" >&2
        return 3
    fi
    printf '%s code %s, journal %s\n' "$label" "$status" "$log"
    return "$status"
}

source_label=${UNLEASH_BENCH_SOURCE_LABEL:-}

cmd_cargo() {
    [ $# -ge 5 ] || usage
    bench=$1 label=$2 log=$3 limit=$4
    shift 4
    dir=/work/src-${source_label:-$label}
    require_bench "$bench"
    printf 'cargo %s : %s\n' "$label" "$*" >> "$log"
    run_logged "$bench" "$label" "$log" "$limit" "$dir" cargo "+$toolchain" "$@"
}

cmd_renderer() {
    [ $# -ge 5 ] || usage
    bench=$1 label=$2 log=$3 limit=$4
    shift 4
    dir=/work/src-${source_label:-$label}/apps/gui-renderer
    require_bench "$bench"
    printf 'renderer %s : %s\n' "$label" "$*" >> "$log"
    run_logged "$bench" "$label" "$log" "$limit" "$dir" pnpm "$@"
}

cmd_run() {
    # Commande invitée complète, exécutée dans la source du label (ex. env RUSTDOCFLAGS=...).
    # UNLEASH_BENCH_SOURCE_LABEL détache la source du nom du journal d'étape.
    [ $# -ge 5 ] || usage
    bench=$1 label=$2 log=$3 limit=$4
    shift 4
    dir=/work/src-${source_label:-$label}
    require_bench "$bench"
    printf 'run %s : %s\n' "$label" "$*" >> "$log"
    run_logged "$bench" "$label" "$log" "$limit" "$dir" "$@"
}

cmd_shell() {
    [ $# -ge 2 ] || usage
    bench=$1
    shift
    require_bench "$bench"
    exec timeout --kill-after=5s "${UNLEASH_BENCH_SHELL_LIMIT:-900s}" docker --context "$context" exec "$bench" /bin/sh -c 'cd /work && exec /bin/sh -c "$*"' sh "$@"
}

[ $# -ge 1 ] || usage
command=$1
shift
case $command in
    preflight) cmd_preflight "$@" ;;
    create) cmd_create "$@" ;;
    source) cmd_source "$@" ;;
    resync) cmd_resync "$@" ;;
    renderer) cmd_renderer "$@" ;;
    cargo) cmd_cargo "$@" ;;
    run) cmd_run "$@" ;;
    shell) cmd_shell "$@" ;;
    *) usage ;;
esac
