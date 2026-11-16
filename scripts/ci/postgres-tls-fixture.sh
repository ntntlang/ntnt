#!/usr/bin/env bash
# Start a disposable TLS-enabled PostgreSQL for tests/db_tls_postgres.rs.
#
#   scripts/ci/postgres-tls-fixture.sh <postgres-major> <state-dir> [host-port]
#
# Generates throwaway test material in <state-dir>: a CA, a server certificate
# for IP 127.0.0.1 only, and a second unrelated CA. Starts postgres:<major> with
# ssl=on on 127.0.0.1:<host-port> (default 5433) and writes the variables the
# tests read to <state-dir>/env (for `source`) and, in GitHub Actions, to
# $GITHUB_ENV. Test-only: never reuse this material or password elsewhere.
# Clean up with `docker rm -f ntnt-db-tls-<major>` and delete <state-dir>.
set -euo pipefail

version=${1:?usage: postgres-tls-fixture.sh <postgres-major> <state-dir> [host-port]}
dir=${2:?usage: postgres-tls-fixture.sh <postgres-major> <state-dir> [host-port]}
port=${3:-5433}
name=ntnt-db-tls-$version
password=ntnt_tls_fixture_only

mkdir -p "$dir"
chmod 0700 "$dir"
dir=$(cd "$dir" && pwd)
cd "$dir"

ec=(-newkey ec -pkeyopt ec_paramgen_curve:P-256 -nodes)
ca_ext=(-addext "basicConstraints=critical,CA:TRUE" -addext "keyUsage=critical,keyCertSign,cRLSign")
openssl req -x509 "${ec[@]}" -days 2 -subj "/CN=ntnt db TLS fixture CA" "${ca_ext[@]}" \
    -keyout ca.key -out ca.crt 2>/dev/null
openssl req -x509 "${ec[@]}" -days 2 -subj "/CN=ntnt db TLS unrelated CA" "${ca_ext[@]}" \
    -keyout other-ca.key -out other-ca.crt 2>/dev/null
openssl req "${ec[@]}" -subj "/CN=ntnt db TLS fixture server" \
    -keyout server.key -out server.csr 2>/dev/null
cat > server.ext <<'EOF'
basicConstraints=critical,CA:FALSE
keyUsage=critical,digitalSignature
extendedKeyUsage=serverAuth
subjectAltName=IP:127.0.0.1
EOF
openssl x509 -req -in server.csr -CA ca.crt -CAkey ca.key -CAcreateserial -days 2 \
    -extfile server.ext -out server.crt 2>/dev/null
chmod 0600 ./*.key

docker rm -f "$name" >/dev/null 2>&1 || true
# PostgreSQL requires its key to be owned by the server user with mode 0600,
# so copy it inside the container before handing over to the normal entrypoint.
docker run -d --name "$name" -p "127.0.0.1:$port:5432" \
    -e POSTGRES_PASSWORD="$password" -e POSTGRES_DB=ntnt_tls_control \
    -v "$dir:/fixture:ro" --entrypoint bash "mirror.gcr.io/library/postgres:$version" -c '
        install -d -o postgres -g postgres -m 0700 /etc/ntnt-tls &&
        install -o postgres -g postgres -m 0600 /fixture/server.key /etc/ntnt-tls/server.key &&
        install -o postgres -g postgres -m 0644 /fixture/server.crt /etc/ntnt-tls/server.crt &&
        exec docker-entrypoint.sh postgres -c ssl=on \
            -c ssl_cert_file=/etc/ntnt-tls/server.crt -c ssl_key_file=/etc/ntnt-tls/server.key' >/dev/null

# The entrypoint's temporary init server listens only on its Unix socket, so a
# TCP check passes only once the final server is up.
for _ in $(seq 1 60); do
    if docker exec "$name" pg_isready -q -h 127.0.0.1 -U postgres; then
        break
    fi
    sleep 1
done
ssl=$(docker exec -e PGPASSWORD="$password" "$name" \
    psql -h 127.0.0.1 -U postgres -d ntnt_tls_control -tAc 'SHOW ssl')
if [ "$ssl" != "on" ]; then
    echo "PostgreSQL $version fixture did not start with ssl=on" >&2
    docker logs "$name" >&2 || true
    exit 1
fi

cat > env <<EOF
export NTNT_DB_TLS_TEST_ADMIN='host=127.0.0.1 port=$port user=postgres password=$password dbname=ntnt_tls_control sslmode=disable'
export NTNT_DB_TLS_TEST_PORT='$port'
export NTNT_DB_TLS_TEST_CA='$dir/ca.crt'
export NTNT_DB_TLS_TEST_OTHER_CA='$dir/other-ca.crt'
EOF
if [ -n "${GITHUB_ENV:-}" ]; then
    sed -e 's/^export //' -e "s/='\(.*\)'$/=\1/" env >> "$GITHUB_ENV"
fi
echo "TLS PostgreSQL $version fixture: 127.0.0.1:$port; source $dir/env"
