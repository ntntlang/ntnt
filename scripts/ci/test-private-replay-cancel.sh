#!/usr/bin/env bash
# Linux regression. Reuse the job's local PostgreSQL image; no target DB.
set -euo pipefail
recipe=$(dirname "$0")/../../examples/database-dev/postgresql/with-private-replay.sh
export real_docker cancel_fixture volume_format
real_docker=$(command -v docker)
image=${NTNT_DB_REPLAY_IMAGE:-postgres:17}
cancel_fixture=$(mktemp -d)
volume_format='{{range .Mounts}}{{if eq .Type "volume"}}{{println .Name}}{{end}}{{end}}'
sentinel=
cleanup() {
    # A failing assertion on a leaky recipe must not itself leak either fixture.
    if [[ -s $cancel_fixture/container ]]; then docker rm -fv "$(<"$cancel_fixture/container")" >/dev/null 2>&1 || :; fi
    if [[ -s $cancel_fixture/volume ]]; then docker volume rm "$(<"$cancel_fixture/volume")" >/dev/null 2>&1 || :; fi
    if [[ -n $sentinel ]]; then docker rm -fv "$sentinel" >/dev/null; fi
    rm -rf -- "$cancel_fixture"
}
trap cleanup EXIT
mkdir "$cancel_fixture/bin" "$cancel_fixture/tmp"
# Same image, label key and anonymous storage, but owned by a different run.
sentinel=$(docker create --pull=never --network=none --label org.ntnt.private-replay=sentinel "$image")
sentinel_volume=$(docker inspect -f "$volume_format" "$sentinel")
cat >"$cancel_fixture/bin/docker" <<'SH'
#!/usr/bin/env bash
set -eu
if [[ $1 != run ]]; then exec "$real_docker" "$@"; fi
# The daemon has created the real container and volume, but the caller never
# gets the ID. Cancel precisely here, without a sleep or a timing race.
"$real_docker" "$@" >"$cancel_fixture/container"
"$real_docker" inspect -f "$volume_format" "$(<"$cancel_fixture/container")" >"$cancel_fixture/volume"
kill -TERM -- "-$recipe_pid"
exit 143
SH
chmod +x "$cancel_fixture/bin/docker"
rc=0
PATH="$cancel_fixture/bin:$PATH" TMPDIR="$cancel_fixture/tmp" NTNT_DB_REPLAY_IMAGE="$image" NTNT_BIN=false \
    setsid --wait bash -c 'export recipe_pid=$$; exec "$@"' _ "$recipe" plan || rc=$?
container=$(<"$cancel_fixture/container")
volume=$(<"$cancel_fixture/volume")
[[ $rc == 143 && -n $container && -n $volume && -n $sentinel_volume ]]
containers=$(docker ps -aq --no-trunc --filter "id=$container")
volumes=$(docker volume ls -q --filter "name=$volume")
if [[ -n $containers || -n $volumes ]]; then
    printf 'Leaked container: %s; leaked volume: %s\n' "$containers" "$volumes" >&2
    exit 1
fi
docker container inspect "$sentinel" >/dev/null
docker volume inspect "$sentinel_volume" >/dev/null
shopt -s nullglob dotglob
leftovers=("$cancel_fixture/tmp"/*)
[[ ${#leftovers[@]} == 0 ]]
printf 'TERM (%s), no ID returned: container %s and volume %s gone; sentinel and its volume intact.\n' "$rc" "$container" "$volume"
