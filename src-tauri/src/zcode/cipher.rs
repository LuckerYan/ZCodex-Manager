use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use rand::RngCore;
use sha2::{Digest, Sha256};

use crate::error::{AppError, AppResult};

pub struct ZCodeCipher {
    key: [u8; 32],
}

impl ZCodeCipher {
    pub const PREFIX: &'static str = "enc:v1:";
    const IV_LEN: usize = 12;
    const TAG_LEN: usize = 16;

    pub fn new() -> AppResult<Self> {
        Self::with_secret(resolve_secret()?)
    }

    pub fn with_secret(secret: String) -> AppResult<Self> {
        let digest = Sha256::digest(secret.as_bytes());
        let mut key = [0u8; 32];
        key.copy_from_slice(&digest);
        Ok(Self { key })
    }

    pub fn is_encrypted(value: &str) -> bool {
        value.starts_with(Self::PREFIX)
    }

    pub fn encrypt(&self, plaintext: &str) -> AppResult<String> {
        let cipher = Aes256Gcm::new_from_slice(&self.key)
            .map_err(|e| AppError::Crypto(format!("AES key 初始化失败: {e}")))?;
        let mut iv = [0u8; Self::IV_LEN];
        rand::thread_rng().fill_bytes(&mut iv);
        let mut ct_with_tag = cipher
            .encrypt(Nonce::from_slice(&iv), plaintext.as_bytes())
            .map_err(|e| AppError::Crypto(format!("AES-GCM 加密失败: {e}")))?;
        if ct_with_tag.len() < Self::TAG_LEN {
            return Err(AppError::Crypto("密文长度异常".to_string()));
        }
        let tag = ct_with_tag.split_off(ct_with_tag.len() - Self::TAG_LEN);
        Ok(format!(
            "{}{}.{}.{}",
            Self::PREFIX,
            URL_SAFE_NO_PAD.encode(iv),
            URL_SAFE_NO_PAD.encode(tag),
            URL_SAFE_NO_PAD.encode(ct_with_tag)
        ))
    }

    pub fn decrypt(&self, value: &str) -> AppResult<String> {
        if !Self::is_encrypted(value) {
            return Ok(value.to_string());
        }
        let body = value.trim_start_matches(Self::PREFIX);
        let parts: Vec<&str> = body.split('.').collect();
        if parts.len() != 3 {
            return Err(AppError::Crypto("enc:v1 结构异常".to_string()));
        }
        let iv = URL_SAFE_NO_PAD
            .decode(parts[0])
            .map_err(|e| AppError::Crypto(format!("IV base64 解码失败: {e}")))?;
        let tag = URL_SAFE_NO_PAD
            .decode(parts[1])
            .map_err(|e| AppError::Crypto(format!("tag base64 解码失败: {e}")))?;
        let mut ciphertext = URL_SAFE_NO_PAD
            .decode(parts[2])
            .map_err(|e| AppError::Crypto(format!("ciphertext base64 解码失败: {e}")))?;
        if iv.len() != Self::IV_LEN {
            return Err(AppError::Crypto("invalid IV length".to_string()));
        }
        if tag.len() != Self::TAG_LEN {
            return Err(AppError::Crypto("invalid auth tag length".to_string()));
        }
        ciphertext.extend_from_slice(&tag);
        let cipher = Aes256Gcm::new_from_slice(&self.key)
            .map_err(|e| AppError::Crypto(format!("AES key 初始化失败: {e}")))?;
        let plaintext = cipher
            .decrypt(Nonce::from_slice(&iv), ciphertext.as_ref())
            .map_err(|e| AppError::Crypto(format!("AES-GCM 解密失败: {e}")))?;
        String::from_utf8(plaintext)
            .map_err(|e| AppError::Crypto(format!("UTF-8 解码失败: {e}")))
    }
}

fn resolve_secret() -> AppResult<String> {
    if let Ok(v) = std::env::var("ZCODE_CREDENTIAL_SECRET") {
        let trimmed = v.trim().to_string();
        if !trimmed.is_empty() {
            return Ok(trimmed);
        }
    }
    let platform = match std::env::consts::OS {
        "windows" => "win32",
        "macos" => "darwin",
        other => other,
    };
    let home = dirs::home_dir()
        .ok_or_else(|| AppError::Path("无法定位 home 目录".to_string()))?
        .to_string_lossy()
        .to_string();
    let username = std::env::var("USERNAME")
        .or_else(|_| std::env::var("USER"))
        .unwrap_or_else(|_| "unknown".to_string());
    Ok(format!(
        "zcode-credential-fallback:{platform}:{home}:{username}"
    ))
}
