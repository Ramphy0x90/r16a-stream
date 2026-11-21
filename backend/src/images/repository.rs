//! SQL for images and image jobs. Worker functions touch only the columns the worker role is
//! granted (SPEC 4.9), `tests/images.rs` runs them as that role.

use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

// ---------------------------------------------------------------- API side

/// A new image with its job, in the caller's transaction.
pub async fn insert_with_job(
    db: &mut PgConnection,
    id: Uuid,
    kind: &str,
    source_key: &str,
    source_bytes: i64,
) -> Result<(), sqlx::Error> {
    insert_pending(db, id, kind, source_key, Some(source_bytes)).await?;
    enqueue(db, id).await?;
    Ok(())
}

/// An image row without a job, for a manual upload that has not completed yet.
pub async fn insert_pending(
    db: &mut PgConnection,
    id: Uuid,
    kind: &str,
    source_key: &str,
    source_bytes: Option<i64>,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "insert into images (id, kind, source_key, source_bytes) values ($1, $2, $3, $4)",
        id,
        kind,
        source_key,
        source_bytes
    )
    .execute(db)
    .await?;
    Ok(())
}

/// Queues the image job and returns its id.
pub async fn enqueue(db: &mut PgConnection, id: Uuid) -> Result<Uuid, sqlx::Error> {
    let job = sqlx::query_scalar!(
        "insert into jobs (kind, payload, image_id, dedupe_key, priority)
         values ('image', jsonb_build_object('imageId', $1::uuid), $1, 'image:' || $1::text, 10)
         returning id",
        id
    )
    .fetch_one(&mut *db)
    .await?;
    // Delivered at commit, wakes an idle worker at once instead of at its next poll.
    sqlx::query!("select pg_notify('r16a_stream_jobs', 'image')")
        .execute(db)
        .await?;
    Ok(job)
}

pub async fn set_source_bytes(db: &mut PgConnection, id: Uuid, bytes: i64) -> Result<(), sqlx::Error> {
    sqlx::query!("update images set source_bytes = $2 where id = $1", id, bytes)
        .execute(db)
        .await?;
    Ok(())
}

pub struct VariantRow {
    pub image_id: Uuid,
    pub placeholder_hsl: Option<String>,
    pub format: String,
    pub width: i16,
    pub object_key: String,
}

/// Every variant of the given ready images, one query per page of cards.
pub async fn ready_variants(db: &PgPool, ids: &[Uuid]) -> Result<Vec<VariantRow>, sqlx::Error> {
    sqlx::query_as!(
        VariantRow,
        "select v.image_id, i.placeholder_hsl, v.format, v.width, v.object_key
         from image_variants v join images i on i.id = v.image_id
         where v.image_id = any($1) and i.status = 'ready'
         order by v.image_id, v.format, v.width",
        ids
    )
    .fetch_all(db)
    .await
}

// ---------------------------------------------------------------- worker side

pub struct ImageSource {
    pub kind: String,
    pub status: String,
    pub source_key: String,
    pub source_bytes: Option<i64>,
}

pub async fn source(db: &PgPool, id: Uuid) -> Result<Option<ImageSource>, sqlx::Error> {
    sqlx::query_as!(
        ImageSource,
        "select kind, status, source_key, source_bytes from images where id = $1",
        id
    )
    .fetch_optional(db)
    .await
}

pub struct NewVariant {
    pub format: &'static str,
    pub width: i16,
    pub height: i16,
    pub object_key: String,
    pub byte_size: i32,
}

/// Replaces the image's variants and marks it ready, in the caller's transaction.
pub async fn finish(
    db: &mut PgConnection,
    id: Uuid,
    width: i32,
    height: i32,
    placeholder: &str,
    variants: &[NewVariant],
) -> Result<(), sqlx::Error> {
    sqlx::query!("delete from image_variants where image_id = $1", id)
        .execute(&mut *db)
        .await?;
    for v in variants {
        sqlx::query!(
            "insert into image_variants (image_id, format, width, height, object_key, byte_size)
             values ($1, $2, $3, $4, $5, $6)",
            id,
            v.format,
            v.width,
            v.height,
            v.object_key,
            v.byte_size
        )
        .execute(&mut *db)
        .await?;
    }
    sqlx::query!(
        "update images set status = 'ready', source_width = $2, source_height = $3, placeholder_hsl = $4,
                ready_at = now()
         where id = $1",
        id,
        width,
        height,
        placeholder
    )
    .execute(db)
    .await?;
    Ok(())
}

pub async fn mark_failed(db: &mut PgConnection, id: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query!("update images set status = 'failed' where id = $1", id)
        .execute(db)
        .await?;
    Ok(())
}
