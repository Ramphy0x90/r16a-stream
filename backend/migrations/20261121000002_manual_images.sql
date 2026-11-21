-- Manual poster, backdrop and still uploads (DECISIONS 078, SPEC 8.2). The upload remembers
-- what the image is for until it completes, then the API creates the image and points the
-- title or episode at it. The worker grants on uploads stay as they are, it never reads these.

alter table uploads
  add column image_kind       text check (image_kind in ('poster', 'backdrop', 'still')),
  add column image_title_id   uuid references titles (id) on delete cascade,
  add column image_episode_id uuid references episodes (id) on delete cascade,
  -- A video source has none of these, an image source has a kind and exactly one target,
  -- stills belong to episodes, posters and backdrops to titles.
  add constraint uploads_image_target_check check (
    (purpose = 'video_source' and image_kind is null and image_title_id is null and image_episode_id is null)
    or (purpose = 'image_source' and variant_id is null and (
          (image_kind = 'still' and image_episode_id is not null and image_title_id is null)
       or (image_kind in ('poster', 'backdrop') and image_title_id is not null and image_episode_id is null)))
  ),
  -- Images are at most 20 MiB and go up as one part.
  add constraint uploads_image_size_check check (purpose <> 'image_source' or (size_bytes <= 20971520 and part_count = 1));

