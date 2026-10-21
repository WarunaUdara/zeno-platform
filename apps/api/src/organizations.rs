use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, PgPool};

use crate::auth::AuthenticatedIdentity;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CreateOrganizationRequest {
    pub name: String,
    pub slug: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct Organization {
    pub id: String,
    pub name: String,
    pub slug: String,
    pub role: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub(crate) struct OrganizationPage {
    pub items: Vec<Organization>,
    pub next_cursor: Option<String>,
}

#[derive(Clone, Debug)]
pub(crate) struct OrganizationCursor {
    created_at: DateTime<Utc>,
    id: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OrganizationError {
    SlugUnavailable,
    Unavailable,
}

#[derive(FromRow)]
struct OrganizationRow {
    id: String,
    name: String,
    slug: String,
    role: String,
    created_at: DateTime<Utc>,
    member_since: DateTime<Utc>,
}

impl OrganizationCursor {
    pub(crate) fn decode(value: &str) -> Option<Self> {
        if value.len() > 512 {
            return None;
        }
        let decoded = URL_SAFE_NO_PAD.decode(value).ok()?;
        let decoded = std::str::from_utf8(&decoded).ok()?;
        let mut fields = decoded.splitn(3, '\n');
        if fields.next()? != "v1" {
            return None;
        }
        let created_at = DateTime::parse_from_rfc3339(fields.next()?)
            .ok()?
            .with_timezone(&Utc);
        let id = fields.next()?;
        if !crate::model::is_public_id(id) {
            return None;
        }
        Some(Self {
            created_at,
            id: id.to_owned(),
        })
    }
}

pub(crate) fn is_valid_name(name: &str) -> bool {
    let name = name.trim();
    (2..=120).contains(&name.chars().count())
}

pub(crate) fn is_valid_slug(slug: &str) -> bool {
    (3..=62).contains(&slug.len())
        && slug
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        && (slug.as_bytes()[0].is_ascii_lowercase() || slug.as_bytes()[0].is_ascii_digit())
        && (slug.as_bytes()[slug.len() - 1].is_ascii_lowercase()
            || slug.as_bytes()[slug.len() - 1].is_ascii_digit())
}

pub(crate) async fn create(
    pool: &PgPool,
    identity: &AuthenticatedIdentity,
    request: CreateOrganizationRequest,
    id: String,
) -> Result<(Organization, bool), OrganizationError> {
    let name = request.name.trim();
    let mut transaction = pool.begin().await.map_err(|error| {
        crate::db_error::record_as(error, "create", OrganizationError::Unavailable)
    })?;
    let created_id = sqlx::query_scalar::<_, String>(
        "INSERT INTO organizations (id, name, slug) VALUES ($1, $2, $3) \
         ON CONFLICT (slug) DO NOTHING RETURNING id",
    )
    .bind(&id)
    .bind(name)
    .bind(&request.slug)
    .fetch_optional(&mut *transaction)
    .await
    .map_err(|error| crate::db_error::record_as(error, "create", OrganizationError::Unavailable))?;

    let was_created = created_id.is_some();
    if let Some(created_id) = created_id {
        sqlx::query(
            "INSERT INTO organization_memberships \
             (organization_id, identity_issuer, identity_subject, role) \
             VALUES ($1, $2, $3, 'owner')",
        )
        .bind(&created_id)
        .bind(&identity.issuer)
        .bind(&identity.subject)
        .execute(&mut *transaction)
        .await
        .map_err(|error| {
            crate::db_error::record_as(error, "create", OrganizationError::Unavailable)
        })?;
    }

    let row = sqlx::query_as::<_, OrganizationRow>(
        "SELECT o.id, o.name, o.slug, m.role, o.created_at, m.created_at AS member_since \
         FROM organizations AS o \
         JOIN organization_memberships AS m ON m.organization_id = o.id \
         WHERE o.slug = $1 AND m.identity_issuer = $2 AND m.identity_subject = $3 \
           AND o.name = $4 AND m.role = 'owner'",
    )
    .bind(&request.slug)
    .bind(&identity.issuer)
    .bind(&identity.subject)
    .bind(name)
    .fetch_optional(&mut *transaction)
    .await
    .map_err(|error| crate::db_error::record_as(error, "create", OrganizationError::Unavailable))?
    .ok_or(OrganizationError::SlugUnavailable)?;

    transaction.commit().await.map_err(|error| {
        crate::db_error::record_as(error, "create", OrganizationError::Unavailable)
    })?;
    Ok((row.into(), was_created))
}

pub(crate) async fn list(
    pool: &PgPool,
    identity: &AuthenticatedIdentity,
    cursor: Option<OrganizationCursor>,
    limit: i64,
) -> Result<OrganizationPage, OrganizationError> {
    let rows =
        match cursor {
            Some(cursor) => sqlx::query_as::<_, OrganizationRow>(
                "SELECT o.id, o.name, o.slug, m.role, o.created_at, m.created_at AS member_since \
                 FROM organization_memberships AS m \
                 JOIN organizations AS o ON o.id = m.organization_id \
                 WHERE m.identity_issuer = $1 AND m.identity_subject = $2 \
                   AND (m.created_at, m.organization_id) < ($3, $4) \
                 ORDER BY m.created_at DESC, m.organization_id DESC LIMIT $5",
            )
            .bind(&identity.issuer)
            .bind(&identity.subject)
            .bind(cursor.created_at)
            .bind(cursor.id)
            .bind(limit + 1)
            .fetch_all(pool)
            .await,
            None => sqlx::query_as::<_, OrganizationRow>(
                "SELECT o.id, o.name, o.slug, m.role, o.created_at, m.created_at AS member_since \
                 FROM organization_memberships AS m \
                 JOIN organizations AS o ON o.id = m.organization_id \
                 WHERE m.identity_issuer = $1 AND m.identity_subject = $2 \
                 ORDER BY m.created_at DESC, m.organization_id DESC LIMIT $3",
            )
            .bind(&identity.issuer)
            .bind(&identity.subject)
            .bind(limit + 1)
            .fetch_all(pool)
            .await,
        }
        .map_err(|error| {
            crate::db_error::record_as(error, "list", OrganizationError::Unavailable)
        })?;

    let has_more = rows.len() as i64 > limit;
    let next_cursor = if has_more {
        rows.get(limit as usize - 1).map(|row| {
            URL_SAFE_NO_PAD.encode(format!("v1\n{}\n{}", row.member_since.to_rfc3339(), row.id))
        })
    } else {
        None
    };
    let items = rows
        .into_iter()
        .take(limit as usize)
        .map(Organization::from)
        .collect::<Vec<_>>();
    Ok(OrganizationPage { items, next_cursor })
}

impl From<OrganizationRow> for Organization {
    fn from(row: OrganizationRow) -> Self {
        Self {
            id: row.id,
            name: row.name,
            slug: row.slug,
            role: row.role,
            created_at: row.created_at,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{OrganizationCursor, is_valid_name, is_valid_slug};
    use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};

    #[test]
    fn organization_names_and_slugs_have_clear_bounds() {
        assert!(is_valid_name("Campus Arts Society"));
        assert!(!is_valid_name(" "));
        assert!(!is_valid_name(&"n".repeat(121)));
        assert!(is_valid_slug("campus-arts-2027"));
        assert!(!is_valid_slug("Campus-Arts"));
        assert!(!is_valid_slug("-campus"));
        assert!(!is_valid_slug("campus-"));
        assert!(!is_valid_slug("ca"));
    }

    #[test]
    fn organization_cursor_is_bounded_and_requires_a_public_id() {
        let encoded =
            URL_SAFE_NO_PAD.encode("v1\n2026-10-08T00:00:00Z\nzeno_01J9QY7B5C6D7E8F9G0H1J2K3M");
        assert!(OrganizationCursor::decode(&encoded).is_some());
        assert!(OrganizationCursor::decode(&"a".repeat(513)).is_none());
        let malformed = URL_SAFE_NO_PAD.encode("v1\n2026-10-08T00:00:00Z\norg_1");
        assert!(OrganizationCursor::decode(&malformed).is_none());
    }
}
