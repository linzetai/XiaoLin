use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Notification {
    pub id: String,
    pub category: String,
    pub title: String,
    pub body: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    pub is_read: bool,
    pub created_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub read_at: Option<String>,
}

#[derive(sqlx::FromRow)]
struct NotificationRow {
    id: String,
    category: String,
    title: String,
    body: String,
    detail: Option<String>,
    is_read: bool,
    created_at: String,
    read_at: Option<String>,
}

impl From<NotificationRow> for Notification {
    fn from(r: NotificationRow) -> Self {
        Self {
            id: r.id,
            category: r.category,
            title: r.title,
            body: r.body,
            detail: r.detail,
            is_read: r.is_read,
            created_at: r.created_at,
            read_at: r.read_at,
        }
    }
}

pub struct NotificationStore {
    pool: SqlitePool,
}

impl NotificationStore {
    pub async fn open(pool: SqlitePool) -> anyhow::Result<Self> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS notifications (
                id         TEXT PRIMARY KEY,
                category   TEXT NOT NULL DEFAULT 'system',
                title      TEXT NOT NULL,
                body       TEXT NOT NULL DEFAULT '',
                detail     TEXT,
                is_read    INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                read_at    TEXT
            )",
        )
        .execute(&pool)
        .await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_notifications_unread \
             ON notifications (is_read, created_at DESC)",
        )
        .execute(&pool)
        .await?;

        Ok(Self { pool })
    }

    pub async fn insert(
        &self,
        id: &str,
        category: &str,
        title: &str,
        body: &str,
        detail: Option<&str>,
    ) -> anyhow::Result<Notification> {
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO notifications (id, category, title, body, detail, created_at) \
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(id)
        .bind(category)
        .bind(title)
        .bind(body)
        .bind(detail)
        .bind(&now)
        .execute(&self.pool)
        .await?;

        Ok(Notification {
            id: id.to_string(),
            category: category.to_string(),
            title: title.to_string(),
            body: body.to_string(),
            detail: detail.map(String::from),
            is_read: false,
            created_at: now,
            read_at: None,
        })
    }

    pub async fn list(
        &self,
        limit: i64,
        offset: i64,
        unread_only: bool,
    ) -> anyhow::Result<Vec<Notification>> {
        let rows = if unread_only {
            sqlx::query_as::<_, NotificationRow>(
                "SELECT * FROM notifications WHERE is_read = 0 \
                 ORDER BY created_at DESC LIMIT ? OFFSET ?",
            )
            .bind(limit)
            .bind(offset)
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query_as::<_, NotificationRow>(
                "SELECT * FROM notifications ORDER BY created_at DESC LIMIT ? OFFSET ?",
            )
            .bind(limit)
            .bind(offset)
            .fetch_all(&self.pool)
            .await?
        };

        Ok(rows.into_iter().map(Notification::from).collect())
    }

    pub async fn get(&self, id: &str) -> anyhow::Result<Option<Notification>> {
        let row = sqlx::query_as::<_, NotificationRow>(
            "SELECT * FROM notifications WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(Notification::from))
    }

    pub async fn mark_read(&self, id: &str) -> anyhow::Result<bool> {
        let now = Utc::now().to_rfc3339();
        let result = sqlx::query(
            "UPDATE notifications SET is_read = 1, read_at = ? WHERE id = ? AND is_read = 0",
        )
        .bind(&now)
        .bind(id)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected() > 0)
    }

    pub async fn mark_all_read(&self) -> anyhow::Result<u64> {
        let now = Utc::now().to_rfc3339();
        let result = sqlx::query(
            "UPDATE notifications SET is_read = 1, read_at = ? WHERE is_read = 0",
        )
        .bind(&now)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected())
    }

    pub async fn unread_count(&self) -> anyhow::Result<i64> {
        let row: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM notifications WHERE is_read = 0")
                .fetch_one(&self.pool)
                .await?;

        Ok(row.0)
    }

    pub async fn delete(&self, id: &str) -> anyhow::Result<bool> {
        let result = sqlx::query("DELETE FROM notifications WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected() > 0)
    }

    pub async fn clear_read(&self) -> anyhow::Result<u64> {
        let result = sqlx::query("DELETE FROM notifications WHERE is_read = 1")
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected())
    }

    /// Prune oldest notifications beyond a given max total count.
    pub async fn prune(&self, keep: i64) -> anyhow::Result<u64> {
        let result = sqlx::query(
            "DELETE FROM notifications WHERE id NOT IN (
                SELECT id FROM notifications ORDER BY created_at DESC LIMIT ?
            )",
        )
        .bind(keep)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

    async fn test_pool() -> SqlitePool {
        let opts = SqliteConnectOptions::new()
            .filename(":memory:")
            .create_if_missing(true);
        SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn insert_and_get() {
        let pool = test_pool().await;
        let store = NotificationStore::open(pool).await.unwrap();

        let n = store.insert("n1", "cron", "Job Done", "completed", None).await.unwrap();
        assert_eq!(n.id, "n1");
        assert_eq!(n.category, "cron");
        assert!(!n.is_read);

        let fetched = store.get("n1").await.unwrap().unwrap();
        assert_eq!(fetched.title, "Job Done");
        assert_eq!(fetched.body, "completed");
        assert!(!fetched.is_read);
    }

    #[tokio::test]
    async fn insert_with_detail() {
        let pool = test_pool().await;
        let store = NotificationStore::open(pool).await.unwrap();

        let n = store
            .insert("n1", "system", "Update", "v2.0 available", Some("Full changelog here..."))
            .await
            .unwrap();
        assert_eq!(n.detail.as_deref(), Some("Full changelog here..."));

        let fetched = store.get("n1").await.unwrap().unwrap();
        assert_eq!(fetched.detail.as_deref(), Some("Full changelog here..."));
    }

    #[tokio::test]
    async fn list_with_pagination_and_filter() {
        let pool = test_pool().await;
        let store = NotificationStore::open(pool).await.unwrap();

        for i in 0..5 {
            store
                .insert(&format!("n{i}"), "test", &format!("Title {i}"), "", None)
                .await
                .unwrap();
        }
        store.mark_read("n0").await.unwrap();
        store.mark_read("n1").await.unwrap();

        let all = store.list(10, 0, false).await.unwrap();
        assert_eq!(all.len(), 5);

        let unread = store.list(10, 0, true).await.unwrap();
        assert_eq!(unread.len(), 3);

        let page = store.list(2, 0, false).await.unwrap();
        assert_eq!(page.len(), 2);

        let page2 = store.list(2, 2, false).await.unwrap();
        assert_eq!(page2.len(), 2);
    }

    #[tokio::test]
    async fn mark_read_and_unread_count() {
        let pool = test_pool().await;
        let store = NotificationStore::open(pool).await.unwrap();

        store.insert("n1", "cron", "A", "", None).await.unwrap();
        store.insert("n2", "cron", "B", "", None).await.unwrap();
        store.insert("n3", "cron", "C", "", None).await.unwrap();

        assert_eq!(store.unread_count().await.unwrap(), 3);

        assert!(store.mark_read("n1").await.unwrap());
        assert_eq!(store.unread_count().await.unwrap(), 2);

        // Double-read returns false (already read)
        assert!(!store.mark_read("n1").await.unwrap());

        let n1 = store.get("n1").await.unwrap().unwrap();
        assert!(n1.is_read);
        assert!(n1.read_at.is_some());
    }

    #[tokio::test]
    async fn mark_all_read() {
        let pool = test_pool().await;
        let store = NotificationStore::open(pool).await.unwrap();

        store.insert("n1", "cron", "A", "", None).await.unwrap();
        store.insert("n2", "cron", "B", "", None).await.unwrap();

        let affected = store.mark_all_read().await.unwrap();
        assert_eq!(affected, 2);
        assert_eq!(store.unread_count().await.unwrap(), 0);

        // Idempotent
        let affected2 = store.mark_all_read().await.unwrap();
        assert_eq!(affected2, 0);
    }

    #[tokio::test]
    async fn delete_and_clear_read() {
        let pool = test_pool().await;
        let store = NotificationStore::open(pool).await.unwrap();

        store.insert("n1", "cron", "A", "", None).await.unwrap();
        store.insert("n2", "cron", "B", "", None).await.unwrap();
        store.insert("n3", "cron", "C", "", None).await.unwrap();

        assert!(store.delete("n1").await.unwrap());
        assert!(!store.delete("n1").await.unwrap()); // already gone
        assert_eq!(store.list(10, 0, false).await.unwrap().len(), 2);

        store.mark_read("n2").await.unwrap();
        let cleared = store.clear_read().await.unwrap();
        assert_eq!(cleared, 1);
        assert_eq!(store.list(10, 0, false).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn prune_keeps_newest() {
        let pool = test_pool().await;
        let store = NotificationStore::open(pool).await.unwrap();

        for i in 0..10 {
            store
                .insert(&format!("n{i}"), "test", &format!("Title {i}"), "", None)
                .await
                .unwrap();
        }

        let pruned = store.prune(5).await.unwrap();
        assert_eq!(pruned, 5);

        let remaining = store.list(20, 0, false).await.unwrap();
        assert_eq!(remaining.len(), 5);
    }

    #[tokio::test]
    async fn get_nonexistent_returns_none() {
        let pool = test_pool().await;
        let store = NotificationStore::open(pool).await.unwrap();

        assert!(store.get("nope").await.unwrap().is_none());
    }
}
