-- Claim refresh without holding a row lock across the call to Authentik (DECISIONS 075). A
-- request takes the lease with one short UPDATE, talks to Authentik outside any transaction,
-- and writes the result in a second short transaction. A lease left behind by a failed or
-- crashed refresh expires on its own, which doubles as a per-session backoff while Authentik
-- is unreachable.
alter table sessions add column refresh_lease_until timestamptz;
