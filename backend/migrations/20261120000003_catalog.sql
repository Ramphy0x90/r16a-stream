-- Catalog. Images (schema only, the pipeline follows in Phase 3b), genres, titles, seasons,
-- episodes, credits, the denormalised language set and the TMDB cache. SPEC 4.3, 4.4, 4.8.

create table images (
  id               uuid primary key default gen_random_uuid(),
  kind             text not null check (kind in ('poster', 'backdrop', 'still', 'profile')),
  status           text not null default 'pending' check (status in ('pending', 'ready', 'failed')),
  -- Uploads bucket. Kept after processing, sources are small.
  source_key       text not null check (char_length(source_key) <= 512),
  source_bytes     bigint check (source_bytes > 0),
  source_width     integer check (source_width between 1 and 16384),
  source_height    integer check (source_height between 1 and 16384),
  -- Average colour for skeletons, in the HSL notation of the design tokens.
  placeholder_hsl  text check (placeholder_hsl ~ '^[0-9]{1,3} [0-9]{1,3}% [0-9]{1,3}%$'),
  created_at       timestamptz not null default now(),
  ready_at         timestamptz
);

create table image_variants (
  image_id    uuid not null references images (id) on delete cascade,
  format      text not null check (format in ('avif', 'webp')),
  width       smallint not null check (width between 16 and 4096),
  height      smallint not null check (height between 16 and 4096),
  -- Media bucket.
  object_key  text not null check (char_length(object_key) <= 512),
  byte_size   integer not null check (byte_size > 0),
  primary key (image_id, format, width)
);

create table genres (
  id       smallint generated always as identity primary key,
  -- Also the i18n key genre.<slug>. The name is the fallback when no translation exists.
  slug     text not null unique check (slug ~ '^[a-z0-9-]{2,40}$'),
  name     text not null check (char_length(name) between 1 and 60),
  tmdb_id  integer unique
);

create table titles (
  id                 uuid primary key default gen_random_uuid(),
  kind               text not null check (kind in ('movie', 'show')),
  name               text not null check (char_length(name) between 1 and 300),
  original_name      text check (char_length(original_name) <= 300),
  -- Lowercase and unaccented, set by the service, the A to Z sort key.
  sort_name          text not null check (char_length(sort_name) between 1 and 300),
  overview           text check (char_length(overview) <= 5000),
  year               smallint check (year between 1870 and 2200),
  -- Movies only. Shows have runtimes per episode.
  runtime_minutes    integer check (runtime_minutes between 1 and 1440),
  tmdb_id            integer,
  poster_image_id    uuid references images (id) on delete set null,
  backdrop_image_id  uuid references images (id) on delete set null,
  status             text not null default 'draft' check (status in ('draft', 'published')),
  -- Sort key of Recently added. Set on first publish, bumped when a new episode becomes ready.
  listed_at          timestamptz,
  created_at         timestamptz not null default now(),
  updated_at         timestamptz not null default now(),
  search_doc         tsvector generated always as (
                       setweight(to_tsvector('simple', f_unaccent(name)), 'A') ||
                       setweight(to_tsvector('simple', f_unaccent(coalesce(original_name, ''))), 'A') ||
                       setweight(to_tsvector('simple', f_unaccent(coalesce(overview, ''))), 'D')
                     ) stored,
  -- Target of the composite keys that pin seasons to shows.
  unique (id, kind),
  unique (kind, tmdb_id),
  check (status = 'draft' or listed_at is not null),
  check (kind = 'movie' or runtime_minutes is null)
);
create index titles_recent_idx      on titles (listed_at desc, id desc) where status = 'published';
create index titles_kind_recent_idx on titles (kind, listed_at desc, id desc) where status = 'published';
create index titles_name_idx        on titles (sort_name, id) where status = 'published';
create index titles_kind_name_idx   on titles (kind, sort_name, id) where status = 'published';
create index titles_search_idx      on titles using gin (search_doc);
create index titles_name_trgm_idx   on titles using gin (f_unaccent(lower(name)) gin_trgm_ops);
create index titles_admin_idx       on titles (updated_at desc, id desc);

create table title_genres (
  title_id  uuid not null references titles (id) on delete cascade,
  genre_id  smallint not null references genres (id) on delete cascade,
  primary key (title_id, genre_id)
);
create index title_genres_genre_idx on title_genres (genre_id, title_id);

-- Content languages with at least one ready variant, maintained in the transaction that changes
-- a variant's readiness, so the language filter is one index probe instead of a join through
-- episodes and variants.
create table title_languages (
  title_id  uuid not null references titles (id) on delete cascade,
  language  text not null check (char_length(language) <= 35),
  primary key (title_id, language)
);
create index title_languages_language_idx on title_languages (language, title_id);

create table title_credits (
  title_id         uuid not null references titles (id) on delete cascade,
  ord              smallint not null check (ord between 0 and 49),
  person_name      text not null check (char_length(person_name) between 1 and 200),
  character        text check (char_length(character) <= 200),
  role             text not null check (role in ('cast', 'director', 'creator', 'writer')),
  profile_image_id uuid references images (id) on delete set null,
  primary key (title_id, role, ord)
);

create table seasons (
  id               uuid primary key default gen_random_uuid(),
  title_id         uuid not null,
  -- Pins seasons to shows through the composite key below. A movie can never get a season, and
  -- a show with seasons can never become a movie.
  title_kind       text not null default 'show' check (title_kind = 'show'),
  -- 0 holds specials.
  number           smallint not null check (number between 0 and 999),
  name             text check (char_length(name) <= 300),
  overview         text check (char_length(overview) <= 5000),
  year             smallint check (year between 1870 and 2200),
  poster_image_id  uuid references images (id) on delete set null,
  created_at       timestamptz not null default now(),
  updated_at       timestamptz not null default now(),
  foreign key (title_id, title_kind) references titles (id, kind) on delete cascade,
  unique (title_id, number),
  unique (id, title_id)
);

create table episodes (
  id               uuid primary key default gen_random_uuid(),
  season_id        uuid not null,
  title_id         uuid not null,
  number           smallint not null check (number between 1 and 9999),
  name             text check (char_length(name) <= 300),
  overview         text check (char_length(overview) <= 5000),
  runtime_minutes  integer check (runtime_minutes between 1 and 1440),
  air_date         date,
  still_image_id   uuid references images (id) on delete set null,
  created_at       timestamptz not null default now(),
  updated_at       timestamptz not null default now(),
  foreign key (season_id, title_id) references seasons (id, title_id) on delete cascade,
  unique (season_id, number),
  unique (id, title_id)
);
create index episodes_title_idx on episodes (title_id);

create table tmdb_cache (
  -- Kind, TMDB id and language, built in tmdb/service.rs.
  cache_key   text primary key check (char_length(cache_key) <= 200),
  payload     jsonb not null,
  fetched_at  timestamptz not null default now(),
  expires_at  timestamptz not null
);
create index tmdb_cache_expiry_idx on tmdb_cache (expires_at);

-- Worker grants, SPEC 4.9. Exactly the columns the image pipeline and the readiness updates
-- need. No title text, no credits, no TMDB cache.
grant select (id, kind, status, source_key, source_bytes) on images to r16a_stream_worker;
grant update (status, source_width, source_height, placeholder_hsl, ready_at) on images to r16a_stream_worker;
grant select, insert, delete on image_variants to r16a_stream_worker;
grant select, insert on title_languages to r16a_stream_worker;
grant select (id, kind, status, listed_at) on titles to r16a_stream_worker;
grant update (listed_at) on titles to r16a_stream_worker;
grant select (id, title_id, number) on seasons to r16a_stream_worker;
grant select (id, season_id, title_id, number) on episodes to r16a_stream_worker;
