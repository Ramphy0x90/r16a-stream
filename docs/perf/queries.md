# Hot queries

Measured by `cd backend && R16A_ENV=development cargo run --bin api -- seed --bench` (SPEC 4.10) on 2026-11-20, against the dev Postgres 15 container on a developer machine. The command loads 10 000 titles (half movies, half shows, 10% drafts), 20 genres with two per title, one to three content languages per title, 20 000 seasons and 100 000 episodes, then runs `VACUUM (ANALYZE)` and each query 7 times with `EXPLAIN (ANALYZE, BUFFERS)`. It removes everything it inserted. Budget is p95 under 50 ms at the database (SPEC 4.10). Every query is far below it.

Queries H6, H7, H10 to H13 belong to later phases and are added to the bench with them.

Findings
- **Search used no index at first.** With the search terms in a CTE row (`from titles t, q`) the planner could not use either GIN index and filtered every published title, 22 ms at this scale and linear with the catalog. The terms are now written inline, which lets PostgreSQL combine `titles_search_idx` and `titles_name_trgm_idx` with a `BitmapOr`. A rare word dropped from about 25 ms to 0.04 ms, a typo from 23 ms to 1.7 ms. A word found in 12% of all titles still reads those rows, 3.3 ms, which only synthetic data with eight repeated words produces.
- **GIN pending list.** Right after a bulk insert the planner avoids the GIN indexes, because new entries wait in the pending list until a vacuum merges them. The bench vacuums after seeding. In production titles arrive one at a time and autovacuum merges the list, so this only matters for a future bulk import, which should end with `VACUUM (ANALYZE) titles`.
- **H2 genre filter** is the slowest browse case at about 1 ms, a semi-join through `title_genres_genre_idx` per page. Fine at this scale.

| # | Query | Median ms | Max ms | Plan |
|---|---|---|---|---|
| H1 | home grid, recently added, first page | 0.01 | 0.03 | Limit |
| H1 | home grid, shows only, page after a cursor deep in the list | 0.02 | 0.04 | Limit |
| H2 | home grid with a genre | 1.12 | 1.41 | Limit |
| H3 | home grid with a content language | 0.13 | 0.30 | Limit |
| H4 | A to Z, page after a cursor | 0.03 | 0.06 | Limit |
| H5 | search, a word in 12% of all titles | 3.33 | 4.30 | Limit |
| H5 | search, a rare word, as most real searches are | 0.04 | 0.14 | Limit |
| H5 | search, a typo | 1.69 | 2.35 | Limit |
| H8 | title page, seasons with episode counts | 0.12 | 0.41 | Sort |
| H9 | episodes of one season | 0.27 | 0.44 | Sort |

## Plans

### H1 home grid, recently added, first page

```
Limit  (cost=0.29..11.81 rows=31 width=65) (actual time=0.004..0.009 rows=31 loops=1)
  Buffers: shared hit=5
  ->  Index Scan using titles_recent_idx on titles t  (cost=0.29..3347.05 rows=9001 width=65) (actual time=0.004..0.007 rows=31 loops=1)
        Buffers: shared hit=5
Planning Time: 0.042 ms
Execution Time: 0.014 ms
```

### H1 home grid, shows only, page after a cursor deep in the list

```
Limit  (cost=0.29..35.09 rows=31 width=65) (actual time=0.005..0.012 rows=31 loops=1)
  Buffers: shared hit=7
  ->  Index Scan using titles_recent_idx on titles t  (cost=0.29..2727.03 rows=2429 width=65) (actual time=0.004..0.011 rows=31 loops=1)
        Index Cond: (ROW(listed_at, id) < ROW((now() - '66:40:00'::interval), 'ffffffff-ffff-ffff-ffff-ffffffffffff'::uuid))
        Filter: (kind = 'show'::text)
        Rows Removed by Filter: 24
        Buffers: shared hit=7
Planning Time: 0.054 ms
Execution Time: 0.019 ms
```

### H2 home grid with a genre

```
Limit  (cost=0.71..395.12 rows=31 width=42) (actual time=0.020..0.747 rows=31 loops=1)
  Buffers: shared hit=2087
  ->  Nested Loop Semi Join  (cost=0.71..11451.42 rows=900 width=42) (actual time=0.019..0.745 rows=31 loops=1)
        Buffers: shared hit=2087
        ->  Index Scan using titles_recent_idx on titles t  (cost=0.29..3347.05 rows=9001 width=42) (actual time=0.004..0.054 rows=353 loops=1)
              Buffers: shared hit=32
        ->  Nested Loop  (cost=0.42..0.89 rows=1 width=16) (actual time=0.002..0.002 rows=0 loops=353)
              Buffers: shared hit=2055
              ->  Index Only Scan using title_genres_pkey on title_genres tg  (cost=0.29..0.55 rows=2 width=18) (actual time=0.001..0.001 rows=2 loops=353)
                    Index Cond: (title_id = t.id)
                    Heap Fetches: 0
                    Buffers: shared hit=707
              ->  Index Scan using genres_pkey on genres g  (cost=0.14..0.16 rows=1 width=2) (actual time=0.000..0.000 rows=0 loops=674)
                    Index Cond: (id = tg.genre_id)
                    Filter: (slug = 'bench-genre-3'::text)
                    Rows Removed by Filter: 1
                    Buffers: shared hit=1348
Planning:
  Buffers: shared hit=28
Planning Time: 0.249 ms
Execution Time: 0.769 ms
```

### H3 home grid with a content language

```
Limit  (cost=0.57..78.35 rows=31 width=42) (actual time=0.007..0.116 rows=31 loops=1)
  Buffers: shared hit=198
  ->  Nested Loop  (cost=0.57..7696.16 rows=3067 width=42) (actual time=0.006..0.113 rows=31 loops=1)
        Buffers: shared hit=198
        ->  Index Scan using titles_recent_idx on titles t  (cost=0.29..3347.05 rows=9001 width=42) (actual time=0.002..0.012 rows=94 loops=1)
              Buffers: shared hit=9
        ->  Index Only Scan using title_languages_pkey on title_languages tl  (cost=0.29..0.48 rows=1 width=16) (actual time=0.001..0.001 rows=0 loops=94)
              Index Cond: ((title_id = t.id) AND (language = 'it'::text))
              Heap Fetches: 0
              Buffers: shared hit=189
Planning:
  Buffers: shared hit=24
Planning Time: 0.183 ms
Execution Time: 0.127 ms
```

### H4 A to Z, page after a cursor

```
Limit  (cost=0.29..28.44 rows=31 width=52) (actual time=0.006..0.023 rows=31 loops=1)
  Buffers: shared hit=34
  ->  Index Scan using titles_name_idx on titles t  (cost=0.29..5069.63 rows=5581 width=52) (actual time=0.006..0.021 rows=31 loops=1)
        Index Cond: (ROW(sort_name, id) > ROW('hidden'::text, '00000000-0000-0000-0000-000000000000'::uuid))
        Buffers: shared hit=34
Planning Time: 0.059 ms
Execution Time: 0.031 ms
```

### H5 search, a word in 12% of all titles

```
Limit  (cost=2472.83..2472.91 rows=31 width=20) (actual time=3.130..3.132 rows=31 loops=1)
  Buffers: shared hit=239
  ->  Sort  (cost=2472.83..2478.03 rows=2078 width=20) (actual time=3.129..3.130 rows=31 loops=1)
        Sort Key: ((ts_rank(search_doc, '''harbour'''::tsquery) + similarity(f_unaccent(lower(name)), 'harbour'::text))) DESC, id DESC
        Sort Method: top-N heapsort  Memory: 29kB
        Buffers: shared hit=239
        ->  Bitmap Heap Scan on titles t  (cost=99.49..2410.97 rows=2078 width=20) (actual time=0.334..3.035 rows=1124 loops=1)
              Recheck Cond: ((search_doc @@ '''harbour'''::tsquery) OR (f_unaccent(lower(name)) % 'harbour'::text))
              Filter: (status = 'published'::text)
              Rows Removed by Filter: 124
              Heap Blocks: exact=210
              Buffers: shared hit=239
              ->  BitmapOr  (cost=99.49..99.49 rows=2460 width=0) (actual time=0.302..0.302 rows=0 loops=1)
                    Buffers: shared hit=29
                    ->  Bitmap Index Scan on titles_search_idx  (cost=0.00..21.36 rows=1248 width=0) (actual time=0.074..0.074 rows=1248 loops=1)
                          Index Cond: (search_doc @@ '''harbour'''::tsquery)
                          Buffers: shared hit=4
                    ->  Bitmap Index Scan on titles_name_trgm_idx  (cost=0.00..77.09 rows=1212 width=0) (actual time=0.228..0.228 rows=1248 loops=1)
                          Index Cond: (f_unaccent(lower(name)) % 'harbour'::text)
                          Buffers: shared hit=25
Planning:
  Buffers: shared hit=2
Planning Time: 0.325 ms
Execution Time: 3.144 ms
```

### H5 search, a rare word, as most real searches are

```
Limit  (cost=275.53..275.60 rows=31 width=20) (actual time=0.095..0.097 rows=1 loops=1)
  Buffers: shared hit=21
  ->  Sort  (cost=275.53..275.64 rows=46 width=20) (actual time=0.094..0.096 rows=1 loops=1)
        Sort Key: ((ts_rank(search_doc, '''tag4243'''::tsquery) + similarity(f_unaccent(lower(name)), 'tag4243'::text))) DESC, id DESC
        Sort Method: quicksort  Memory: 25kB
        Buffers: shared hit=21
        ->  Bitmap Heap Scan on titles t  (cost=80.41..274.26 rows=46 width=20) (actual time=0.088..0.091 rows=1 loops=1)
              Recheck Cond: ((search_doc @@ '''tag4243'''::tsquery) OR (f_unaccent(lower(name)) % 'tag4243'::text))
              Filter: (status = 'published'::text)
              Heap Blocks: exact=1
              Buffers: shared hit=21
              ->  BitmapOr  (cost=80.41..80.41 rows=51 width=0) (actual time=0.029..0.031 rows=0 loops=1)
                    Buffers: shared hit=20
                    ->  Bitmap Index Scan on titles_search_idx  (cost=0.00..12.38 rows=50 width=0) (actual time=0.006..0.006 rows=1 loops=1)
                          Index Cond: (search_doc @@ '''tag4243'''::tsquery)
                          Buffers: shared hit=3
                    ->  Bitmap Index Scan on titles_name_trgm_idx  (cost=0.00..68.01 rows=1 width=0) (actual time=0.023..0.023 rows=1 loops=1)
                          Index Cond: (f_unaccent(lower(name)) % 'tag4243'::text)
                          Buffers: shared hit=17
Planning:
  Buffers: shared hit=2
Planning Time: 0.373 ms
Execution Time: 0.142 ms
```

### H5 search, a typo

```
Limit  (cost=124.55..124.55 rows=1 width=20) (actual time=0.961..0.961 rows=0 loops=1)
  Buffers: shared hit=228
  ->  Sort  (cost=124.55..124.55 rows=1 width=20) (actual time=0.960..0.961 rows=0 loops=1)
        Sort Key: ((ts_rank(search_doc, '''brakng'' & ''rivr'''::tsquery) + similarity(f_unaccent(lower(name)), 'brakng rivr'::text))) DESC, id DESC
        Sort Method: quicksort  Memory: 25kB
        Buffers: shared hit=228
        ->  Bitmap Heap Scan on titles t  (cost=120.01..124.54 rows=1 width=20) (actual time=0.959..0.960 rows=0 loops=1)
              Recheck Cond: ((search_doc @@ '''brakng'' & ''rivr'''::tsquery) OR (f_unaccent(lower(name)) % 'brakng rivr'::text))
              Rows Removed by Index Recheck: 313
              Filter: (status = 'published'::text)
              Heap Blocks: exact=192
              Buffers: shared hit=228
              ->  BitmapOr  (cost=120.01..120.01 rows=1 width=0) (actual time=0.193..0.193 rows=0 loops=1)
                    Buffers: shared hit=36
                    ->  Bitmap Index Scan on titles_search_idx  (cost=0.00..20.00 rows=1 width=0) (actual time=0.003..0.003 rows=0 loops=1)
                          Index Cond: (search_doc @@ '''brakng'' & ''rivr'''::tsquery)
                          Buffers: shared hit=5
                    ->  Bitmap Index Scan on titles_name_trgm_idx  (cost=0.00..100.01 rows=1 width=0) (actual time=0.190..0.190 rows=313 loops=1)
                          Index Cond: (f_unaccent(lower(name)) % 'brakng rivr'::text)
                          Buffers: shared hit=31
Planning:
  Buffers: shared hit=2
Planning Time: 0.287 ms
Execution Time: 0.969 ms
```

### H8 title page, seasons with episode counts

```
Sort  (cost=37.18..37.19 rows=4 width=26) (actual time=0.099..0.100 rows=4 loops=1)
  Sort Key: s.number
  Sort Method: quicksort  Memory: 25kB
  Buffers: shared hit=563
  InitPlan 2 (returns $1)
    ->  Limit  (cost=0.00..0.25 rows=1 width=16) (actual time=0.073..0.073 rows=1 loops=1)
          Buffers: shared hit=544
          ->  Seq Scan on titles  (cost=0.00..1257.01 rows=4999 width=16) (actual time=0.072..0.072 rows=1 loops=1)
                Filter: ((original_name = 'bench-seed'::text) AND (kind = 'show'::text))
                Rows Removed by Filter: 1
                Buffers: shared hit=544
  ->  Bitmap Heap Scan on seasons s  (cost=4.32..36.89 rows=4 width=26) (actual time=0.086..0.097 rows=4 loops=1)
        Recheck Cond: (title_id = $1)
        Heap Blocks: exact=4
        Buffers: shared hit=563
        ->  Bitmap Index Scan on seasons_title_id_number_key  (cost=0.00..4.32 rows=4 width=0) (actual time=0.077..0.077 rows=4 loops=1)
              Index Cond: (title_id = $1)
              Buffers: shared hit=546
        SubPlan 1
          ->  Aggregate  (cost=4.52..4.53 rows=1 width=8) (actual time=0.003..0.003 rows=1 loops=4)
                Buffers: shared hit=13
                ->  Index Only Scan using episodes_season_id_number_key on episodes e  (cost=0.42..4.50 rows=5 width=0) (actual time=0.002..0.002 rows=5 loops=4)
                      Index Cond: (season_id = s.id)
                      Heap Fetches: 0
                      Buffers: shared hit=13
Planning Time: 0.090 ms
Execution Time: 0.116 ms
```

### H9 episodes of one season

```
Sort  (cost=25.00..25.01 rows=5 width=28) (actual time=0.163..0.164 rows=5 loops=1)
  Sort Key: e.number
  Sort Method: quicksort  Memory: 25kB
  Buffers: shared hit=68
  InitPlan 1 (returns $1)
    ->  Limit  (cost=0.30..1.31 rows=1 width=16) (actual time=0.156..0.156 rows=1 loops=1)
          Buffers: shared hit=60
          ->  Nested Loop  (cost=0.30..5072.10 rows=5000 width=16) (actual time=0.155..0.156 rows=1 loops=1)
                Buffers: shared hit=60
                ->  Seq Scan on seasons s  (cost=0.00..478.00 rows=5000 width=32) (actual time=0.152..0.152 rows=1 loops=1)
                      Filter: (number = 2)
                      Rows Removed by Filter: 5000
                      Buffers: shared hit=57
                ->  Memoize  (cost=0.30..1.32 rows=1 width=16) (actual time=0.003..0.003 rows=1 loops=1)
                      Cache Key: s.title_id
                      Cache Mode: logical
                      Hits: 0  Misses: 1  Evictions: 0  Overflows: 0  Memory Usage: 1kB
                      Buffers: shared hit=3
                      ->  Index Scan using titles_pkey on titles t  (cost=0.29..1.31 rows=1 width=16) (actual time=0.002..0.002 rows=1 loops=1)
                            Index Cond: (id = s.title_id)
                            Filter: (original_name = 'bench-seed'::text)
                            Buffers: shared hit=3
  ->  Bitmap Heap Scan on episodes e  (cost=4.46..23.63 rows=5 width=28) (actual time=0.159..0.161 rows=5 loops=1)
        Recheck Cond: (season_id = $1)
        Heap Blocks: exact=5
        Buffers: shared hit=68
        ->  Bitmap Index Scan on episodes_season_id_number_key  (cost=0.00..4.46 rows=5 width=0) (actual time=0.158..0.158 rows=5 loops=1)
              Index Cond: (season_id = $1)
              Buffers: shared hit=63
Planning:
  Buffers: shared hit=12
Planning Time: 0.113 ms
Execution Time: 0.175 ms
```
