use std::path::{Path, PathBuf};
use std::sync::Mutex;

use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value;

use crate::error::{AppError, AppResult};
use crate::models::{AccountRecord, AccountRowDto, AccountUpsert, ModelQuotaDto};

pub struct Db {
    conn: Mutex<Connection>,
}

impl Db {
    pub fn open_default() -> AppResult<Self> {
        let dir = app_data_dir()?;
        std::fs::create_dir_all(&dir)?;
        let path = dir.join("zcode-manager.db");
        let conn = Connection::open(path)?;
        let db = Self {
            conn: Mutex::new(conn),
        };
        db.migrate()?;
        Ok(db)
    }

    fn conn(&self) -> AppResult<std::sync::MutexGuard<'_, Connection>> {
        self.conn
            .lock()
            .map_err(|_| AppError::Message("数据库锁已损坏".to_string()))
    }

    fn migrate(&self) -> AppResult<()> {
        let conn = self.conn()?;
        conn.execute_batch(
            r#"
            PRAGMA journal_mode=WAL;
            PRAGMA foreign_keys=ON;

            CREATE TABLE IF NOT EXISTS accounts (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                alias TEXT NOT NULL UNIQUE,
                email TEXT,
                user_id TEXT,
                display_name TEXT,
                active_provider TEXT NOT NULL DEFAULT 'zai',
                zai_access_token_enc TEXT NOT NULL,
                zcode_jwt_token_enc TEXT NOT NULL,
                user_info_json TEXT NOT NULL DEFAULT '{}',
                config_providers_json TEXT NOT NULL DEFAULT '{}',
                coding_plan_cache_json TEXT NOT NULL DEFAULT '{}',
                hot_switch_ready INTEGER NOT NULL DEFAULT 0,
                source TEXT NOT NULL,
                note TEXT,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                last_auth_at INTEGER,
                last_quota_refresh_at INTEGER,
                quota_plan_name TEXT,
                quota_plan_status TEXT,
                quota_plan_ends_at INTEGER,
                quota_models_json TEXT NOT NULL DEFAULT '[]',
                quota_error TEXT
            );

            CREATE TABLE IF NOT EXISTS switch_history (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                account_id INTEGER NOT NULL,
                switched_at INTEGER NOT NULL,
                method TEXT NOT NULL,
                success INTEGER NOT NULL,
                message TEXT,
                backup_dir TEXT,
                FOREIGN KEY(account_id) REFERENCES accounts(id) ON DELETE CASCADE
            );
            "#,
        )?;
        Ok(())
    }

    pub fn upsert_account(&self, account: AccountUpsert) -> AppResult<i64> {
        let now = Utc::now().timestamp();
        let conn = self.conn()?;
        let existing_id: Option<i64> = conn
            .query_row(
                "SELECT id FROM accounts WHERE alias = ?1",
                params![account.alias],
                |row| row.get(0),
            )
            .optional()?;

        let user_info = serde_json::to_string(&account.user_info_json)?;
        let providers = serde_json::to_string(&account.config_providers_json)?;
        let cache = serde_json::to_string(&account.coding_plan_cache_json)?;
        let hot = if account.hot_switch_ready { 1 } else { 0 };

        if let Some(id) = existing_id {
            conn.execute(
                r#"
                UPDATE accounts SET
                    email=?2, user_id=?3, display_name=?4, active_provider=?5,
                    zai_access_token_enc=?6, zcode_jwt_token_enc=?7,
                    user_info_json=?8, config_providers_json=?9, coding_plan_cache_json=?10,
                    hot_switch_ready=?11, source=?12, note=?13, updated_at=?14, last_auth_at=?15
                WHERE id=?1
                "#,
                params![
                    id,
                    account.email,
                    account.user_id,
                    account.display_name,
                    account.active_provider,
                    account.zai_access_token_enc,
                    account.zcode_jwt_token_enc,
                    user_info,
                    providers,
                    cache,
                    hot,
                    account.source,
                    account.note,
                    now,
                    account.last_auth_at
                ],
            )?;
            Ok(id)
        } else {
            conn.execute(
                r#"
                INSERT INTO accounts (
                    alias, email, user_id, display_name, active_provider,
                    zai_access_token_enc, zcode_jwt_token_enc, user_info_json,
                    config_providers_json, coding_plan_cache_json, hot_switch_ready,
                    source, note, created_at, updated_at, last_auth_at
                ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)
                "#,
                params![
                    account.alias,
                    account.email,
                    account.user_id,
                    account.display_name,
                    account.active_provider,
                    account.zai_access_token_enc,
                    account.zcode_jwt_token_enc,
                    user_info,
                    providers,
                    cache,
                    hot,
                    account.source,
                    account.note,
                    now,
                    now,
                    account.last_auth_at
                ],
            )?;
            Ok(conn.last_insert_rowid())
        }
    }

    pub fn list_accounts(&self) -> AppResult<Vec<AccountRowDto>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            r#"
            SELECT id, alias, email, user_id, display_name, active_provider,
                   zai_access_token_enc, zcode_jwt_token_enc, user_info_json,
                   config_providers_json, coding_plan_cache_json, hot_switch_ready,
                   source, note, created_at, updated_at, last_auth_at,
                   last_quota_refresh_at, quota_plan_name, quota_plan_status,
                   quota_plan_ends_at, quota_models_json, quota_error
            FROM accounts
            ORDER BY updated_at DESC, id DESC
            "#,
        )?;
        let rows = stmt
            .query_map([], |row| record_from_row(row))?
            .collect::<Result<Vec<_>, _>>()?;
        rows.into_iter().map(record_to_row).collect()
    }

    pub fn get_account(&self, id: i64) -> AppResult<AccountRecord> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            r#"
            SELECT id, alias, email, user_id, display_name, active_provider,
                   zai_access_token_enc, zcode_jwt_token_enc, user_info_json,
                   config_providers_json, coding_plan_cache_json, hot_switch_ready,
                   source, note, created_at, updated_at, last_auth_at,
                   last_quota_refresh_at, quota_plan_name, quota_plan_status,
                   quota_plan_ends_at, quota_models_json, quota_error
            FROM accounts WHERE id=?1
            "#,
        )?;
        stmt.query_row(params![id], |row| record_from_row(row))
            .optional()?
            .ok_or(AppError::AccountNotFound(id))
    }

    /// 找一个有 config 快照、可作为合成模板的账号(排除 exclude_id),取最近更新的。
    /// 用于给缺 config 的浏览器登录账号自动合成 config(start-plan apiKey==jwt 铁律)。
    pub fn find_template_with_config(&self, exclude_id: i64) -> AppResult<Option<AccountRecord>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            r#"
            SELECT id, alias, email, user_id, display_name, active_provider,
                   zai_access_token_enc, zcode_jwt_token_enc, user_info_json,
                   config_providers_json, coding_plan_cache_json, hot_switch_ready,
                   source, note, created_at, updated_at, last_auth_at,
                   last_quota_refresh_at, quota_plan_name, quota_plan_status,
                   quota_plan_ends_at, quota_models_json, quota_error
            FROM accounts
            WHERE id != ?1 AND config_providers_json != '{}'
            ORDER BY updated_at DESC
            "#,
        )?;
        let rows = stmt
            .query_map(params![exclude_id], |row| record_from_row(row))?
            .collect::<Result<Vec<_>, _>>()?;
        // 双保险:过滤掉 config 实际为空对象的(防止存了带空格的 '{}')
        Ok(rows.into_iter().find(|r| {
            r.config_providers_json
                .as_object()
                .map(|m| !m.is_empty())
                .unwrap_or(false)
        }))
    }

    /// 回存自动合成的 config 到指定账号(下次热切换直接复用,并刷新可热切换标记)。
    pub fn set_account_config(
        &self,
        id: i64,
        providers: &Value,
        cache: &Value,
        hot_switch_ready: bool,
    ) -> AppResult<()> {
        let now = Utc::now().timestamp();
        let providers_s = serde_json::to_string(providers)?;
        let cache_s = serde_json::to_string(cache)?;
        let conn = self.conn()?;
        conn.execute(
            r#"
            UPDATE accounts SET
                config_providers_json=?2,
                coding_plan_cache_json=?3,
                hot_switch_ready=?4,
                updated_at=?5
            WHERE id=?1
            "#,
            params![id, providers_s, cache_s, if hot_switch_ready { 1 } else { 0 }, now],
        )?;
        Ok(())
    }

    pub fn update_quota(
        &self,
        id: i64,
        plan_name: Option<String>,
        plan_status: Option<String>,
        plan_ends_at: Option<i64>,
        models: &[ModelQuotaDto],
        error: Option<String>,
    ) -> AppResult<()> {
        let now = Utc::now().timestamp();
        let models_json = serde_json::to_string(models)?;
        let conn = self.conn()?;
        conn.execute(
            r#"
            UPDATE accounts SET
                last_quota_refresh_at=?2,
                quota_plan_name=?3,
                quota_plan_status=?4,
                quota_plan_ends_at=?5,
                quota_models_json=?6,
                quota_error=?7,
                updated_at=?8
            WHERE id=?1
            "#,
            params![id, now, plan_name, plan_status, plan_ends_at, models_json, error, now],
        )?;
        Ok(())
    }

    pub fn update_quota_error(&self, id: i64, error: String) -> AppResult<()> {
        let now = Utc::now().timestamp();
        let conn = self.conn()?;
        conn.execute(
            r#"
            UPDATE accounts SET
                last_quota_refresh_at=?2,
                quota_error=?3,
                updated_at=?4
            WHERE id=?1
            "#,
            params![id, now, error, now],
        )?;
        Ok(())
    }

    pub fn delete_accounts(&self, ids: &[i64]) -> AppResult<usize> {
        let conn = self.conn()?;
        let mut deleted = 0usize;
        for id in ids {
            deleted += conn.execute("DELETE FROM accounts WHERE id=?1", params![id])?;
        }
        Ok(deleted)
    }

    pub fn record_switch(
        &self,
        account_id: i64,
        success: bool,
        message: &str,
        backup_dir: Option<&str>,
    ) -> AppResult<()> {
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO switch_history (account_id, switched_at, method, success, message, backup_dir) VALUES (?1,?2,'hot_switch',?3,?4,?5)",
            params![
                account_id,
                Utc::now().timestamp(),
                if success { 1 } else { 0 },
                message,
                backup_dir
            ],
        )?;
        Ok(())
    }
}

fn app_data_dir() -> AppResult<PathBuf> {
    let base = dirs::data_dir()
        .or_else(dirs::config_dir)
        .ok_or_else(|| AppError::Path("无法定位用户数据目录".to_string()))?;
    Ok(base.join("zcode-manager"))
}

fn record_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<AccountRecord> {
    let user_info_s: String = row.get(8)?;
    let providers_s: String = row.get(9)?;
    let cache_s: String = row.get(10)?;
    let models_s: String = row.get(21)?;
    Ok(AccountRecord {
        id: row.get(0)?,
        alias: row.get(1)?,
        email: row.get(2)?,
        user_id: row.get(3)?,
        display_name: row.get(4)?,
        active_provider: row.get(5)?,
        zai_access_token_enc: row.get(6)?,
        zcode_jwt_token_enc: row.get(7)?,
        user_info_json: serde_json::from_str(&user_info_s).unwrap_or(Value::Object(Default::default())),
        config_providers_json: serde_json::from_str(&providers_s).unwrap_or(Value::Object(Default::default())),
        coding_plan_cache_json: serde_json::from_str(&cache_s).unwrap_or(Value::Object(Default::default())),
        hot_switch_ready: row.get::<_, i64>(11)? != 0,
        source: row.get(12)?,
        note: row.get(13)?,
        created_at: row.get(14)?,
        updated_at: row.get(15)?,
        last_auth_at: row.get(16)?,
        last_quota_refresh_at: row.get(17)?,
        quota_plan_name: row.get(18)?,
        quota_plan_status: row.get(19)?,
        quota_plan_ends_at: row.get(20)?,
        quota_models_json: serde_json::from_str(&models_s).unwrap_or(Value::Array(vec![])),
        quota_error: row.get(22)?,
    })
}

pub fn record_to_row(record: AccountRecord) -> AppResult<AccountRowDto> {
    let models: Vec<ModelQuotaDto> = serde_json::from_value(record.quota_models_json.clone())
        .unwrap_or_else(|_| Vec::new());
    Ok(AccountRowDto {
        id: record.id,
        alias: record.alias,
        email: record.email,
        user_id: record.user_id,
        display_name: record.display_name,
        hot_switch_ready: record.hot_switch_ready,
        is_active: false,
        source: record.source,
        note: record.note,
        created_at: record.created_at,
        updated_at: record.updated_at,
        last_quota_refresh_at: record.last_quota_refresh_at,
        plan_name: record.quota_plan_name,
        plan_status: record.quota_plan_status,
        plan_ends_at: record.quota_plan_ends_at,
        models,
        quota_error: record.quota_error,
    })
}

pub fn write_json(path: &Path, value: &Value) -> AppResult<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, serde_json::to_string_pretty(value)?)?;
    Ok(())
}
