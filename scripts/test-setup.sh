#!/bin/bash
set -e

DB_USER=main_serve
DB_PASS="changeme"
DB_NAME=main_serve_test

POSTGRES_CONTAINER=main-serve-postgres-test
MYSQL_CONTAINER=main-serve-mysql-test

# ---------------------------------------------------------------------------
# Cleanup function: runs on exit (normal, error, or signal)
# ---------------------------------------------------------------------------
cleanup() {
  echo ""
  echo "=== Cleaning up test database containers ==="
  docker rm -f "$POSTGRES_CONTAINER" "$MYSQL_CONTAINER" 2>/dev/null || true
  echo "=== Done ==="
}
trap cleanup EXIT INT TERM HUP

# ---------------------------------------------------------------------------
# Remove stale containers from previous interrupted runs
# ---------------------------------------------------------------------------
echo "=== Removing stale containers (if any) ==="
docker rm -f "$POSTGRES_CONTAINER" "$MYSQL_CONTAINER" 2>/dev/null || true

echo "=== Starting test database containers ==="

echo "Starting PostgreSQL..."
docker run -d \
  --name "$POSTGRES_CONTAINER" \
  -e POSTGRES_DB="$DB_NAME" \
  -e POSTGRES_USER="$DB_USER" \
  -e POSTGRES_PASSWORD="$DB_PASS" \
  -p 5432:5432 \
  --health-cmd "pg_isready -U $DB_USER -d $DB_NAME" \
  --health-interval 10s \
  --health-timeout 5s \
  --health-retries 10 \
  postgres:17

echo "Starting MySQL..."
docker run -d \
  --name "$MYSQL_CONTAINER" \
  -e MYSQL_DATABASE="$DB_NAME" \
  -e MYSQL_USER="$DB_USER" \
  -e MYSQL_PASSWORD="$DB_PASS" \
  -e MYSQL_ROOT_PASSWORD="root" \
  -p 3306:3306 \
  --health-cmd="mysqladmin ping -h 127.0.0.1 -u $DB_USER -p$DB_PASS" \
  --health-interval=10s \
  --health-timeout=5s \
  --health-retries=20 \
  mysql:8.4

echo "Setting MySQL authentication plugin to mysql_native_password..."
for i in $(seq 1 30); do
  if docker inspect --format='{{.State.Health.Status}}' "$MYSQL_CONTAINER" 2>/dev/null | grep -q healthy; then
    break
  fi
  sleep 2
done
docker exec "$MYSQL_CONTAINER" mysql -u root -proot -e "ALTER USER '$DB_USER'@'%' IDENTIFIED WITH mysql_native_password BY '$DB_PASS'; FLUSH PRIVILEGES;" 2>/dev/null || true

echo "Waiting for databases to be ready..."
echo "PostgreSQL:"
for i in $(seq 1 30); do
  if docker inspect --format='{{.State.Health.Status}}' "$POSTGRES_CONTAINER" 2>/dev/null | grep -q healthy; then
    break
  fi
  sleep 2
done
echo "MySQL:"
for i in $(seq 1 30); do
  if docker inspect --format='{{.State.Health.Status}}' "$MYSQL_CONTAINER" 2>/dev/null | grep -q healthy; then
    break
  fi
  sleep 2
done

echo "=== Running tests against all backends ==="
export TEST_POSTGRES_URL="postgres://${DB_USER}:${DB_PASS}@127.0.0.1:5432/${DB_NAME}"
export TEST_MYSQL_URL="mysql://${DB_USER}:${DB_PASS}@127.0.0.1:3306/${DB_NAME}"

cargo test --verbose -- --test-threads=1

echo "=== Done ==="
