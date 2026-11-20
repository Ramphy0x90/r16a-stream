-- The job queue (SPEC 7.2), first used by the image pipeline (SPEC 8.2). The variant column and
-- its index arrive with media_variants in Phase 4, by migration.

create table jobs (
  id              uuid primary key default gen_random_uuid(),
  kind            text not null check (kind in ('transcode', 'image', 'subtitle', 'encode_gc')),
  -- Ids only, never URLs, keys or secrets.
  payload         jsonb not null default '{}'::jsonb,
  status          text not null default 'queued'
                    check (status in ('queued', 'running', 'cancelling', 'succeeded', 'failed', 'cancelled')),
  priority        smallint not null default 0,
  -- Real failures only. Storage waits are counted apart and never fail a job.
  attempts        smallint not null default 0,
  max_attempts    smallint not null default 3,
  storage_waits   integer not null default 0,
  blocked_reason  text check (blocked_reason in ('storage_degraded', 'storage_full', 'scratch_full')),
  run_after       timestamptz not null default now(),
  locked_by       text check (char_length(locked_by) <= 128),
  heartbeat_at    timestamptz,
  progress        real check (progress between 0 and 1),
  failure_code    text check (char_length(failure_code) <= 64),
  -- Sanitised, never contains URLs, keys or file paths.
  failure_detail  text check (char_length(failure_detail) <= 2000),
  dedupe_key      text check (char_length(dedupe_key) <= 200),
  image_id        uuid references images (id) on delete cascade,
  created_at      timestamptz not null default now(),
  started_at      timestamptz,
  finished_at     timestamptz
);
create index jobs_claim_idx   on jobs (priority desc, run_after, id) where status = 'queued';
create index jobs_running_idx on jobs (heartbeat_at) where status in ('running', 'cancelling');
create index jobs_admin_idx   on jobs (created_at desc, id desc);
create index jobs_image_idx   on jobs (image_id);
-- At most one pending job per dedupe key, for example one image job per image.
create unique index jobs_dedupe_idx on jobs (dedupe_key) where status in ('queued', 'running', 'cancelling');

-- Worker grants, SPEC 4.9. Claim, heartbeat, finish and enqueue the next recurring job.
grant select, insert, update on jobs to r16a_stream_worker;
