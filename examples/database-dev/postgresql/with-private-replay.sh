#!/usr/bin/env bash
# Linux dev/CI recipe for the app_replay database and role in ntnt.toml.
# Runs the normal inspection CLI; never inherits target or approval secrets into Docker.
set -euo pipefail
case ${1:-} in plan|status|verify) ;; *) printf 'usage: %s plan|status|verify [ntnt db options...]\n' "$0" >&2; exit 2;; esac
umask 077
work=$(mktemp -d "${TMPDIR:-/tmp}/ntnt-pg.XXXXXXXX")
id=
cleanup() {
    local rc=$?
    trap - EXIT
    if [[ -n $id ]]; then docker rm -fv "$id" >/dev/null || rc=1; fi
    rm -f -- "$work/socket/.s.PGSQL.5432" "$work/socket/.s.PGSQL.5432.lock" || rc=1
    rmdir "$work/socket" "$work" || rc=1
    exit "$rc"
}
mkdir "$work/socket"
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
# Only the calling user can traverse work. Use SCRAM even on the private socket.
chmod 0777 "$work/socket"
admin=$(openssl rand -hex 24)
replay=$(openssl rand -hex 24)
id=$(POSTGRES_PASSWORD="$admin" docker run --rm -d --pull=never --network=none \
    --mount "type=bind,src=$work/socket,dst=/socket" -e POSTGRES_PASSWORD \
    -e POSTGRES_INITDB_ARGS=--auth-local=scram-sha-256 \
    "${NTNT_DB_REPLAY_IMAGE:-postgres:17}" -c unix_socket_directories=/socket,/var/run/postgresql)
# The image uses a temporary socket-only server during init. The final server
# accepts TCP inside the isolated container; nothing is published on the host.
ready=false
for ((i=0; i<100; i++)); do
    if docker exec "$id" pg_isready -q -h 127.0.0.1 -U postgres; then ready=true; break; fi
    sleep 0.2
done
if [[ $ready != true ]]; then printf 'Private PostgreSQL did not start.\n' >&2; exit 1; fi
# Read the identity via the container ID just created, not an arbitrary DSN.
NTNT_DB_REPLAY_CLUSTER=$(PGPASSWORD="$admin" REPLAY_PW="$replay" docker exec -i -e PGPASSWORD -e REPLAY_PW "$id" \
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
