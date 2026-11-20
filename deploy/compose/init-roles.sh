#!/bin/sh
# Creates the database and the three roles on first start of the Postgres container.
# Passwords arrive as psql variables and are quoted by psql itself, so they cannot break out
# of the statement. The same roles exist in production, see deploy/k8s/provision.sql.
set -eu

psql -v ON_ERROR_STOP=1 --username "$POSTGRES_USER" --dbname postgres \
  -v owner_pw="$R16A_OWNER_PASSWORD" -v app_pw="$R16A_APP_PASSWORD" -v worker_pw="$R16A_WORKER_PASSWORD" <<'SQL'
create role r16a_stream_owner login password :'owner_pw' nosuperuser nocreatedb nocreaterole;
create role r16a_stream_app login password :'app_pw' nosuperuser nocreatedb nocreaterole;
create role r16a_stream_worker login password :'worker_pw' nosuperuser nocreatedb nocreaterole;
create database r16a_stream owner r16a_stream_owner;
revoke all on database r16a_stream from public;
grant connect on database r16a_stream to r16a_stream_app, r16a_stream_worker;
SQL

psql -v ON_ERROR_STOP=1 --username "$POSTGRES_USER" --dbname r16a_stream <<'SQL'
alter schema public owner to r16a_stream_owner;
revoke create on schema public from public;
grant usage on schema public to r16a_stream_app, r16a_stream_worker;
-- The app role gets row access to every table the owner creates. The worker role gets nothing
-- by default, its grants are written table by table in the migrations.
alter default privileges for role r16a_stream_owner in schema public
  grant select, insert, update, delete on tables to r16a_stream_app;
alter default privileges for role r16a_stream_owner in schema public
  grant usage, select on sequences to r16a_stream_app;
SQL
