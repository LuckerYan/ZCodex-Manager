//! 通过 Chrome DevTools Protocol 远程驱动 ZCode 的 renderer。
//!
//! 逆向依据(app.asar out/main/index.js):ZCode 左下角账号名由 Root 组件 mount 时
//! 调用一次 `oauthService.restoreCachedSession()` 读盘(credentials.json,无内存缓存)
//! 得到,之后不再刷新。因此热切换写盘后,只要让 renderer 重新 mount(`location.reload()`)
//! 就会重新读盘并更新左下角,无需重启 ZCode 进程。
//!
//! ZCode 打包版默认不开 remote-debugging-port(`!app.isPackaged` 才自动开 9229),
//! 需带 `--remote-debugging-port=9229` 启动后才能用本模块。

use std::time::Duration;

use serde::Deserialize;

use crate::error::{AppError, AppResult};

/// ZCode 源码内置的固定调试端口(`appendSwitch("remote-debugging-port","9229")`)。
pub const CDP_PORT: u16 = 9229;
const CDP_HOST: &str = "127.0.0.1";

#[derive(Debug, Deserialize)]
struct CdpTarget {
    #[serde(default, rename = "type")]
    target_type: String,
    #[serde(default)]
    url: String,
    #[serde(default, rename = "webSocketDebuggerUrl")]
    ws_url: String,
}

fn http_client() -> AppResult<reqwest::blocking::Client> {
    Ok(reqwest::blocking::Client::builder()
        .timeout(Duration::from_millis(700))
        .build()?)
}

/// 探测 CDP 调试端口是否就绪(即 ZCode 是否带 --remote-debugging-port 启动)。
pub fn is_available(port: u16) -> bool {
    let url = format!("http://{CDP_HOST}:{port}/json/version");
    http_client()
        .ok()
        .and_then(|c| c.get(url).send().ok())
        .map(|r| r.status().is_success())
        .unwrap_or(false)
}

/// 只更新 ZCode 左下角账号名 DOM，不触发整页刷新。
/// 返回成功修改用户名节点的 renderer 页面数。排除内嵌浏览器里用户打开的网页(http/https)。
pub fn update_account_name(port: u16, account_name: &str) -> AppResult<usize> {
    let client = http_client()?;
    let targets: Vec<CdpTarget> = client
        .get(format!("http://{CDP_HOST}:{port}/json"))
        .send()?
        .json()?;

    let pages: Vec<&CdpTarget> = targets
        .iter()
        .filter(|t| t.target_type == "page" && !t.ws_url.is_empty())
        // 只操作 ZCode 自身界面(app://、file:// 等)，不动用户在浏览面板里打开的网页。
        .filter(|t| !t.url.starts_with("http://") && !t.url.starts_with("https://"))
        .collect();

    if pages.is_empty() {
        return Err(AppError::Message(
            "CDP 已连接但未找到 ZCode renderer 页面(可能 ZCode 未完全启动)".to_string(),
        ));
    }

    let mut updated = 0usize;
    let mut last_error: Option<String> = None;
    for target in pages {
        match update_account_name_one(&target.ws_url, account_name) {
            Ok(n) => updated += n,
            Err(e) => last_error = Some(e.to_string()),
        }
    }

    if updated == 0 {
        if let Some(error) = last_error {
            return Err(AppError::Message(error));
        }
    }
    Ok(updated)
}

fn update_account_name_one(ws_url: &str, account_name: &str) -> AppResult<usize> {
    use tungstenite::{connect, Message};

    let (mut socket, _resp) =
        connect(ws_url).map_err(|e| AppError::Message(format!("CDP WebSocket 连接失败: {e}")))?;

    let account_name_json = serde_json::to_string(account_name)?;
    let expression = format!(
        r#"(() => {{
  const accountName = {account_name_json};
  const updated = new Set();
  const visible = (el) => {{
    if (!el) return false;
    const rect = el.getBoundingClientRect();
    const style = window.getComputedStyle(el);
    return rect.width > 0 && rect.height > 0 && style.visibility !== 'hidden' && style.display !== 'none';
  }};
  const setName = (el) => {{
    if (!el || !visible(el)) return false;
    el.textContent = accountName;
    el.setAttribute('title', accountName);
    const button = el.closest?.('button[data-testid="login-trigger"]') || el.closest?.('button[aria-label]');
    if (button) {{
      button.setAttribute('aria-label', accountName);
      button.setAttribute('title', accountName);
      button.dataset.zcodeManagerAccountName = accountName;
    }}
    updated.add(el);
    return true;
  }};

  // app.asar 中 login-trigger 是稳定 data-testid；radix id / XPath 来自用户实测，只作为 fallback。
  const explicitSelectors = [
    'button[data-testid="login-trigger"] .truncate.text-\\[13px\\].font-semibold.text-foreground',
    'button[data-testid="login-trigger"] div.truncate.font-semibold.text-foreground',
    '#radix-_r_22_ > div > div',
    'body > div > div:nth-child(2) > div > div:nth-child(2) > div > div > div:nth-child(1) > aside > aside > div:nth-child(2) > div:nth-child(1) > div:nth-child(3) > button > div > div'
  ];
  for (const selector of explicitSelectors) {{
    try {{ setName(document.querySelector(selector)); }} catch (_) {{}}
  }};

  try {{
    const trigger = document.querySelector('button[data-testid="login-trigger"]');
    if (trigger) {{
      trigger.setAttribute('aria-label', accountName);
      trigger.setAttribute('title', accountName);
      trigger.dataset.zcodeManagerAccountName = accountName;
      const nameNode = trigger.querySelector('.truncate.text-\\[13px\\].font-semibold.text-foreground')
        || Array.from(trigger.querySelectorAll('div')).find((el) => {{
          const className = String(el.className || '');
          return className.includes('truncate') && className.includes('font-semibold') && className.includes('text-foreground');
        }});
      setName(nameNode);
    }}
  }} catch (_) {{}}

  try {{
    const byXPath = document.evaluate(
      '/html/body/div/div[2]/div/div[2]/div/div/div[1]/aside/aside/div[2]/div[1]/div[3]/button/div/div',
      document,
      null,
      XPathResult.FIRST_ORDERED_NODE_TYPE,
      null
    ).singleNodeValue;
    setName(byXPath);
  }} catch (_) {{}}

  if (updated.size === 0) {{
    const candidates = Array.from(document.querySelectorAll('aside button div'))
      .filter((el) => {{
        const className = String(el.className || '');
        return className.includes('truncate')
          && className.includes('font-semibold')
          && className.includes('text-foreground')
          && visible(el);
      }})
      .map((el) => ({{ el, rect: el.getBoundingClientRect() }}))
      // 左侧栏底部账号按钮最接近左下角，优先更新它，避免误改其它 truncate 标题。
      .sort((a, b) => (b.rect.bottom - a.rect.bottom) || (a.rect.left - b.rect.left));
    if (candidates[0]) setName(candidates[0].el);
  }}

  window.__zcodeManagerLastAccountName = accountName;
  return {{ updated: updated.size, accountName }};
}})()"#
    );

    let cmd = serde_json::json!({
        "id": 1,
        "method": "Runtime.evaluate",
        "params": {
            "expression": expression,
            "returnByValue": true,
            "awaitPromise": false,
        }
    })
    .to_string();

    socket
        .send(Message::text(cmd))
        .map_err(|e| AppError::Message(format!("CDP 发送 Runtime.evaluate 失败: {e}")))?;

    for _ in 0..8 {
        let msg = socket
            .read()
            .map_err(|e| AppError::Message(format!("CDP 读取 Runtime.evaluate 响应失败: {e}")))?;
        let text = match msg {
            Message::Text(text) => text.to_string(),
            Message::Binary(bytes) => String::from_utf8_lossy(&bytes).to_string(),
            _ => continue,
        };
        let value: serde_json::Value = serde_json::from_str(&text)?;
        if value.get("id").and_then(|id| id.as_i64()) != Some(1) {
            continue;
        }
        if let Some(error) = value.get("error") {
            return Err(AppError::Message(format!("CDP Runtime.evaluate 返回错误: {error}")));
        }
        if let Some(exception) = value.pointer("/result/exceptionDetails") {
            return Err(AppError::Message(format!("ZCode 页面执行局部用户名更新脚本失败: {exception}")));
        }
        let updated = value
            .pointer("/result/result/value/updated")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as usize;
        let _ = socket.close(None);
        return Ok(updated);
    }

    let _ = socket.close(None);
    Err(AppError::Message("CDP 未返回 Runtime.evaluate 响应".to_string()))
}
