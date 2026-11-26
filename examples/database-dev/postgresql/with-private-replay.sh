#!/usr/bin/env bash
# Linux dev/CI recipe for the app_replay database and role in ntnt.toml.
# Runs the normal inspection CLI; never inherits target or approval secrets into Docker.
set -euo pipefail
case ${1:-} in plan|status|verify) ;; *) printf 'usage: %s plan|status|verify [ntnt db options...]\n' "$0" >&2; exit 2;; esac
umask 077
owner="org.ntnt.private-replay=$(openssl rand -hex 16)"
work=$(mktemp -d "${TMPDIR:-/tmp}/ntnt-pg.XXXXXXXX")
launch_uncertain=false
engine() { timeout --kill-after=5s 30s docker "$@"; }
owned_container() { engine ps -aq --no-trunc --filter "label=$owner"; }
dispose_cluster() {
    local cid volumes volume remaining
    cid=$(owned_container) || return 1
    # A failed/timed-out launch may still be in flight at the daemon. Never
    # certify disposal merely because enumeration hasn't found its label yet.
    if [[ -z $cid ]]; then [[ $launch_uncertain == false ]]; return; fi
    [[ $cid =~ ^[0-9a-f]{64}$ ]] || return 1  # require exactly one full ID
    # The image declares its own anonymous data volume (its path varies by major).
    volumes=$(engine container inspect -f '{{range .Mounts}}{{if eq .Type "volume"}}{{println .Name}}{{end}}{{end}}' "$cid") || return 1
    engine rm -fv "$cid" >/dev/null || return 1
    remaining=$(owned_container) || return 1
    [[ -z $remaining ]] || return 1
    while IFS= read -r volume; do
        [[ -n $volume ]] || continue
        if ! remaining=$(engine volume ls -q --filter "name=$volume") || [[ -n $remaining ]]; then
            printf 'Could not confirm removal of data volume %s\n' "$volume" >&2
            return 1
        fi
    done <<< "$volumes"
}
cleanup() {
    local rc=$?
    trap - EXIT
    trap '' HUP INT TERM
    if ! dispose_cluster; then
        printf 'Could not verify replay cleanup (Docker label %s); retaining %s\n' "$owner" "$work" >&2
        exit 1
    fi
    rm -f -- "$work/socket/.s.PGSQL.5432" "$work/socket/.s.PGSQL.5432.lock" || rc=1
    rmdir "$work/socket" "$work" || rc=1
    exit "$rc"
}
mkdir "$work/socket"
trap cleanup EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM
# Only the calling user can traverse work. Use SCRAM even on the private socket.
chmod 0777 "$work/socket"
admin=$(openssl rand -hex 24)
replay=$(openssl rand -hex 24)
# Finish the foreground launch before handling group cancellation: a lookup
# must not race an in-flight create. Keep even a stopped container inspectable.
launch_uncertain=true
POSTGRES_PASSWORD="$admin" setsid --wait timeout --kill-after=5s 30s docker run -d --label "$owner" --pull=never --network=none \
    --mount "type=bind,src=$work/socket,dst=/socket" -e POSTGRES_PASSWORD \
    -e POSTGRES_INITDB_ARGS=--auth-local=scram-sha-256 \
    "${NTNT_DB_REPLAY_IMAGE:-postgres:17}" -c unix_socket_directories=/socket,/var/run/postgresql >/dev/null
id=$(owned_container)
[[ $id =~ ^[0-9a-f]{64}$ ]] || { printf 'Expected one private PostgreSQL container.\n' >&2; exit 1; }
launch_uncertain=false
# The image uses a temporary socket-only server during init. The final server
# accepts TCP inside the isolated container; nothing is published on the host.
ready=false
for ((i=0; i<100; i++)); do
    if engine exec "$id" pg_isready -q -h 127.0.0.1 -U postgres; then ready=true; break
    else rc=$?; case $rc in 1|2) ;; *) exit "$rc";; esac; fi
    sleep 0.2
done
if [[ $ready != true ]]; then printf 'Private PostgreSQL did not start.\n' >&2; exit 1; fi
# Read the identity via the container ID just created, not an arbitrary DSN.
NTNT_DB_REPLAY_CLUSTER=$(PGPASSWORD="$admin" REPLAY_PW="$replay" engine exec -i -e PGPASSWORD -e REPLAY_PW "$id" \
    psql -X -qAt -w -v ON_ERROR_STOP=1 -U postgres -d postgres <<'SQL'
\getenv pw REPLAY_PW
CREATE ROLE app_replay_owner LOGIN PASSWORD :'pw';
CREATE DATABASE app_replay OWNER app_replay_owner TEMPLATE template0;
SELECT system_identifier::text FROM pg_catalog.pg_control_system();
SQL
)
unset admin
export NTNT_DB_REPLAY_CLUSTER
# Quote the socket path for libpq's key/value syntax.
socket=${work//\\/\\\\}/socket
socket=${socket//\'/\\\'}
export APP_REPLAY_DATABASE="host='$socket' user=app_replay_owner dbname=app_replay password=$replay sslmode=disable"
"${NTNT_BIN:-ntnt}" db "$@"
