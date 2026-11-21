-- Variants, uploads, transcode attempts, renditions, subtitles and encode GC bookkeeping.
-- SPEC 4.5, 4.9 and 7. A variant is one language of a movie or an episode, a full encoded copy.

create table media_variants (
  id                  uuid primary key default gen_random_uuid(),
  title_id            uuid not null,
  title_kind          text not null check (title_kind in ('movie', 'show')),
  episode_id          uuid,
  language            text not null check (language ~ '^[a-z]{2,3}(-[A-Za-z0-9]{2,8})*$'),
  status              text not null default 'awaiting_upload'
                        check (status in ('awaiting_upload', 'processing', 'ready', 'failed')),
  -- Foreign key added below, after transcode_attempts exists.
  current_attempt_id  uuid,
  duration_ms         bigint check (duration_ms > 0),
  created_at          timestamptz not null default now(),
  updated_at          timestamptz not null default now(),
  ready_at            timestamptz,
  foreign key (title_id, title_kind) references titles (id, kind) on delete cascade,
  foreign key (episode_id, title_id) references episodes (id, title_id) on delete cascade,
  -- Movies have no episode, every show variant belongs to one.
  check ((title_kind = 'movie') = (episode_id is null)),
  -- A re-transcode of a ready variant keeps it ready on the old attempt until the new one switches.
  check ((status = 'ready') = (current_attempt_id is not null)),
  unique (id, title_id)
);
create unique index media_variants_movie_lang_idx   on media_variants (title_id, language)   where episode_id is null;
create unique index media_variants_episode_lang_idx on media_variants (episode_id, language) where episode_id is not null;
create index media_variants_title_idx on media_variants (title_id);

create table uploads (
  id                 uuid primary key default gen_random_uuid(),
  purpose            text not null check (purpose in ('video_source', 'image_source')),
  variant_id         uuid references media_variants (id) on delete set null,
  image_id           uuid references images (id) on delete set null,
  -- Uploads bucket.
  object_key         text not null unique check (char_length(object_key) <= 512),
  s3_upload_id       text not null check (char_length(s3_upload_id) <= 256),
  -- 60 GiB at most.
  size_bytes         bigint not null check (size_bytes between 1 and 64424509440),
  -- 50 MiB parts, at most 1229 of them for 60 GiB.
  part_size          integer not null check (part_size = 52428800),
  part_count         integer not null check (part_count between 1 and 1229),
  original_filename  text check (char_length(original_filename) <= 255),
  status             text not null default 'initiated' check (status in ('initiated', 'completed', 'aborted')),
  -- Ladder estimate, SPEC 7.10.
  estimated_output_bytes bigint not null default 0 check (estimated_output_bytes >= 0),
  -- Released when the transcode ends.
  reserved_bytes     bigint not null default 0 check (reserved_bytes >= 0),
  -- Set when retention removed the object, SPEC 7.9.
  source_deleted_at  timestamptz,
  created_by         uuid not null references users (id),
  created_at         timestamptz not null default now(),
  completed_at       timestamptz,
  -- Initiated uploads past this are aborted by the cleanup task.
  expires_at         timestamptz not null
);
create index uploads_variant_idx  on uploads (variant_id, created_at desc);
create index uploads_stale_idx    on uploads (expires_at) where status = 'initiated';
create index uploads_reserved_idx on uploads (id) where reserved_bytes > 0;
create index uploads_retention_idx on uploads (completed_at) where status = 'completed' and source_deleted_at is null;

alter table jobs add column variant_id uuid references media_variants (id) on delete cascade;
create index jobs_variant_idx on jobs (variant_id, created_at desc);

create table transcode_attempts (
  id                 uuid primary key default gen_random_uuid(),
  variant_id         uuid not null references media_variants (id) on delete cascade,
  upload_id          uuid not null references uploads (id),
  job_id             uuid references jobs (id) on delete set null,
  encoder            text not null check (encoder in ('x264', 'qsv', 'vaapi')),
  -- v/<variant>/<attempt>/
  storage_prefix     text not null unique check (char_length(storage_prefix) <= 200),
  status             text not null default 'running' check (status in ('running', 'ready', 'failed', 'abandoned')),
  duration_ms        bigint check (duration_ms > 0),
  audio_codecs       text check (char_length(audio_codecs) <= 64),
  audio_channels     smallint check (audio_channels between 1 and 8),
  audio_playlist_key text check (char_length(audio_playlist_key) <= 512),
  scrub_vtt_key      text check (char_length(scrub_vtt_key) <= 512),
  -- Sum of every object under the prefix, measured after upload.
  total_bytes        bigint check (total_bytes > 0),
  -- Every object listed back with the expected size.
  verified_at        timestamptz,
  created_at         timestamptz not null default now(),
  finished_at        timestamptz,
  unique (id, variant_id),
  check (status <> 'ready' or (verified_at is not null and total_bytes is not null))
);
create index transcode_attempts_variant_idx on transcode_attempts (variant_id, created_at desc);

alter table media_variants
  add foreign key (current_attempt_id, id) references transcode_attempts (id, variant_id);

create table renditions (
  attempt_id         uuid not null references transcode_attempts (id) on delete cascade,
  -- A ladder rung, or the source height below 360.
  height             smallint not null check (height between 144 and 1080),
  width              smallint not null check (width between 16 and 4096),
  -- Peak, bits per second, measured.
  bandwidth          integer not null check (bandwidth > 0),
  average_bandwidth  integer not null check (average_bandwidth > 0),
  codecs             text not null check (char_length(codecs) <= 64),
  frame_rate         numeric(6, 3) not null check (frame_rate > 0),
  playlist_key       text not null check (char_length(playlist_key) <= 512),
  primary key (attempt_id, height)
);

create table subtitle_tracks (
  id           uuid primary key default gen_random_uuid(),
  variant_id   uuid not null references media_variants (id) on delete cascade,
  language     text not null check (language ~ '^[a-z]{2,3}(-[A-Za-z0-9]{2,8})*$'),
  label        text check (char_length(label) <= 60),
  kind         text not null default 'subtitles' check (kind in ('subtitles', 'captions', 'forced')),
  status       text not null default 'pending' check (status in ('pending', 'ready', 'failed')),
  -- Uploads bucket.
  staged_key   text not null check (char_length(staged_key) <= 512),
  -- Media bucket once ready.
  object_key   text check (char_length(object_key) <= 512),
  byte_size    integer check (byte_size between 1 and 2097152),
  created_at   timestamptz not null default now(),
  unique (variant_id, language, kind)
);

-- Encode GC bookkeeping. A prefix must be seen orphaned for 24 hours before deletion (SPEC 7.6).
create table gc_orphans (
  bucket         text not null check (bucket in ('media', 'uploads')),
  prefix         text not null check (char_length(prefix) <= 512),
  first_seen_at  timestamptz not null default now(),
  last_seen_at   timestamptz not null default now(),
  primary key (bucket, prefix)
);

-- Worker grants, SPEC 4.9, exactly as specified. The worker never sees s3_upload_id, who
-- uploaded what, or any text of titles.
grant select on media_variants to r16a_stream_worker;
grant update (status, current_attempt_id, duration_ms, ready_at, updated_at) on media_variants to r16a_stream_worker;
grant select, insert, update on transcode_attempts to r16a_stream_worker;
grant select, insert on renditions to r16a_stream_worker;
grant select (id, purpose, variant_id, image_id, object_key, size_bytes, status, reserved_bytes) on uploads to r16a_stream_worker;
grant update (reserved_bytes) on uploads to r16a_stream_worker;
grant select (id, variant_id, language, kind, status, staged_key, object_key) on subtitle_tracks to r16a_stream_worker;
grant update (status, object_key, byte_size) on subtitle_tracks to r16a_stream_worker;
grant select, insert, update, delete on gc_orphans to r16a_stream_worker;
