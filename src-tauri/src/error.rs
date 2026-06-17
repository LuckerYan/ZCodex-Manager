use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("IO 错误: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON 错误: {0}")]
    Json(#[from] serde_json::Error),
    #[error("SQLite 错误: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("HTTP 错误: {0}")]
    Http(#[from] reqwest::Error),
    #[error("加解密失败: {0}")]
    Crypto(String),
    #[error("路径错误: {0}")]
    Path(String),
    #[error("ZCode API 错误: {0}")]
    Api(String),
    #[error("账号不存在: {0}")]
    AccountNotFound(i64),
    #[error("热切换不可用: {0}")]
    HotSwitch(String),
    #[error("{0}")]
    Message(String),
}

#[derive(Debug, Serialize)]
pub struct ErrorPayload {
    pub message: String,
}

impl serde::Serialize for AppError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        ErrorPayload {
            message: self.to_string(),
        }
        .serialize(serializer)
    }
}

pub type AppResult<T> = Result<T, AppError>;
