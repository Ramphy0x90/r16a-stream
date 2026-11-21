-- Staged subtitle files live in the uploads bucket, which only the API key may write. The worker
-- promotes a track to the media bucket, the API deletes the staged file afterwards and records
-- it here so the cleanup does not revisit the track (SPEC 7.9). The worker grants on
-- subtitle_tracks name their columns, so this one stays invisible to the worker.
alter table subtitle_tracks add column staged_removed_at timestamptz;
create index subtitle_tracks_staged_idx on subtitle_tracks (id)
  where staged_removed_at is null and status in ('ready', 'failed');
