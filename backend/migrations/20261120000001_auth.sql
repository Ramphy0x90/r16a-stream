-- Authentication. Users, web sessions, in-flight login flows and the audit log. SPEC 4.2, 4.8
-- and 6. The worker gets no grant on any of these tables (SPEC 4.9).

create table users (
  id                        uuid primary key default gen_random_uuid(),
  oidc_issuer               text not null check (char_length(oidc_issuer) <= 512),
  oidc_subject              text not null check (char_length(oidc_subject) <= 255),
  display_name              text not null check (char_length(display_name) between 1 and 100),
  -- Last admin claim seen, for display and audit only. Authorisation reads the session row or
  -- the bearer token, never this column.
  is_admin                  boolean not null default false,
  ui_language               text not null default 'en' check (ui_language ~ '^[a-z]{2,3}(-[A-Za-z0-9]{2,8})*$'),
  theme                     text not null default 'dark' check (theme in ('dark', 'light', 'system')),
  preferred_audio_language  text check (preferred_audio_language ~ '^[a-z]{2,3}(-[A-Za-z0-9]{2,8})*$'),
  created_at                timestamptz not null default now(),
  last_seen_at              timestamptz not null default now(),
  disabled_at               timestamptz,
  -- Bearer tokens issued at or before this instant are refused (SPEC 6.8).
  sessions_revoked_at       timestamptz,
  unique (oidc_issuer, oidc_subject)
);
create index users_display_name_trgm_idx on users using gin (f_unaccent(lower(display_name)) gin_trgm_ops);

-- Web BFF sessions. Mobile has no server session, it presents Authentik access tokens.
create table sessions (
  id                   uuid primary key,
  user_id              uuid not null references users (id) on delete cascade,
  -- SHA-256 of the cookie value. The cookie itself is never stored.
  token_hash           bytea not null unique check (octet_length(token_hash) = 32),
  -- From the last verified ID token, re-read on every claim refresh.
  is_admin             boolean not null,
  -- XChaCha20-Poly1305 with the session key named by key_id, AAD bound to the session id, so a
  -- sealed token copied into another row does not open.
  refresh_token_enc    bytea not null,
  key_id               smallint not null,
  -- Refresh runs under a row lock on this row, because Authentik rotates refresh tokens and a
  -- second use of the old one fails.
  claims_refreshed_at  timestamptz not null default now(),
  created_at           timestamptz not null default now(),
  last_seen_at         timestamptz not null default now(),
  idle_expires_at      timestamptz not null,
  absolute_expires_at  timestamptz not null,
  user_agent           text check (char_length(user_agent) <= 256)
);
create index sessions_user_idx on sessions (user_id);
create index sessions_expiry_idx on sessions (idle_expires_at);
create index sessions_absolute_expiry_idx on sessions (absolute_expires_at);

-- In-flight authorization code flows. Rows live at most 10 minutes and are taken exactly once.
create table oidc_flows (
  state_hash         bytea primary key check (octet_length(state_hash) = 32),
  -- SHA-256 of the flow cookie, so a state value alone cannot finish a login in another browser.
  browser_binding    bytea not null check (octet_length(browser_binding) = 32),
  pkce_verifier_enc  bytea not null,
  key_id             smallint not null,
  nonce_hash         bytea not null check (octet_length(nonce_hash) = 32),
  return_to          text not null check (char_length(return_to) <= 512),
  created_at         timestamptz not null default now(),
  expires_at         timestamptz not null
);
create index oidc_flows_expiry_idx on oidc_flows (expires_at);

-- Append-only. Every admin mutation writes one row in the same transaction (SPEC 5.2).
create table audit_log (
  id           bigint generated always as identity primary key,
  actor_id     uuid references users (id) on delete set null,
  action       text not null check (char_length(action) <= 64),
  target_kind  text not null check (char_length(target_kind) <= 32),
  target_id    text check (char_length(target_id) <= 64),
  -- Changed field names and ids, never secrets or free text from users.
  details      jsonb not null default '{}'::jsonb,
  request_id   text check (char_length(request_id) <= 64),
  created_at   timestamptz not null default now()
);
create index audit_log_recent_idx on audit_log (created_at desc, id desc);
create index audit_log_target_idx on audit_log (target_kind, target_id, created_at desc);

-- The app role gets table rights through default privileges. Taking these back makes the log
-- append-only for the API, so a compromised API cannot rewrite history.
revoke update, delete, truncate on audit_log from r16a_stream_app;
