-- Foundation. Extensions, the immutable unaccent wrapper, and the two tables the Phase 1
-- health and capacity code uses. Feature tables arrive with their phases. SPEC 4.

-- Both are trusted extensions since PG13, so the owner role may create them because it owns
-- the database. No superuser is needed.
create extension if not exists pg_trgm;
create extension if not exists unaccent;

-- unaccent() is only STABLE, because its dictionary could change, so it cannot appear in an
-- index or a generated column. Pinning the dictionary makes this wrapper safe to mark
-- IMMUTABLE. SQL-standard body, PG14 and later.
create function f_unaccent(text) returns text
  language sql immutable parallel safe strict
  return public.unaccent('public.unaccent'::regdictionary, $1);

-- Latest write probe and encoder per running instance, for the admin storage view.
create table component_health (
  component         text not null check (component in ('api', 'worker')),
  instance          text not null check (char_length(instance) <= 128),
  storage_writable  boolean not null,
  encoder           text check (char_length(encoder) <= 32),
  checked_at        timestamptz not null default now(),
  primary key (component, instance)
);

-- Disk space per Garage node, polled by the API from the read-only metrics endpoint and read
-- by the worker before it writes outputs. The worker cannot reach the metrics port itself.
create table garage_capacity (
  node            text primary key check (char_length(node) <= 128),
  data_avail      bigint not null check (data_avail >= 0),
  data_total      bigint not null check (data_total >= 0),
  meta_avail      bigint check (meta_avail >= 0),
  meta_total      bigint check (meta_total >= 0),
  replication     smallint not null check (replication between 1 and 7),
  checked_at      timestamptz not null default now()
);

-- Worker grants, SPEC 4.9. The worker role has no default privileges, every grant is explicit.
grant select, insert, update on component_health to r16a_stream_worker;
grant select on garage_capacity to r16a_stream_worker;
