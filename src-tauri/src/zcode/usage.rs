use std::time::Duration;

use rusqlite::{Connection, OpenFlags};

use crate::error::AppResult;
use crate::models::{
    UsageDailyModelPoint, UsageDailyPoint, UsageHeatCell, UsageStatsDto,
};
use crate::zcode::paths::zcode_paths;

/// 直接只读 ZCode CLI 的 `cli/db/db.sqlite`，提取「使用统计」聚合数据。
///
/// 数据源表：`model_usage`（token 主表）/ `session` / `tool_usage`。
/// 不复制、不落地到管理器自身数据库：用读写 flags 打开以兼容 WAL 并发读，
/// 但随即置 `query_only=ON` 保证绝不写入。WAL 模式天然支持「多读单写」，
/// 因此即使 ZCode 正在运行，这里的只读访问也不会阻塞它、也读得到最新数据。
///
/// 所有重计算（SUM / GROUP BY / 按天聚合）都在 SQLite 层完成，前端只渲染精简结果。
/// 时间戳处理：started_at 是毫秒，SQL 里用 `started_at/1000` 转秒并按本地时区切天。
pub fn read_usage_stats() -> AppResult<UsageStatsDto> {
    let paths = zcode_paths()?;
    let db_path = paths.cli_db_path.to_string_lossy().to_string();
    let generated_at = chrono::Utc::now().timestamp();

    let unavailable = |db_path: String| UsageStatsDto {
        available: false,
        db_path,
        generated_at,
        total_tokens: 0,
        input_tokens: 0,
        output_tokens: 0,
        reasoning_tokens: 0,
        cache_read_tokens: 0,
        model_request_count: 0,
        session_count: 0,
        project_count: 0,
        tool_call_count: 0,
        first_at: 0,
        last_at: 0,
        daily: Vec::new(),
        daily_by_model: Vec::new(),
        by_model: Vec::new(),
        by_tool: Vec::new(),
        heatmap: Vec::new(),
    };

    // ZCode 未安装 / 尚无统计库 → 优雅降级（前端显示空态，而非报错）
    if !paths.cli_db_path.exists() {
        return Ok(unavailable(db_path));
    }

    // 不带 CREATE 的读写连接；打不开（权限/独占等）也降级为不可用而非中断仪表盘
    let conn = match Connection::open_with_flags(&paths.cli_db_path, OpenFlags::SQLITE_OPEN_READ_WRITE) {
        Ok(c) => c,
        Err(_) => return Ok(unavailable(db_path)),
    };
    // 仪表盘刷新是交互路径，不能因为 ZCode CLI 正在写 WAL 就把桌面 UI 卡住数秒。
    // 这里宁愿短等待后返回当前可读数据/空态，也不要长时间占住 Tauri invoke。
    conn.busy_timeout(Duration::from_millis(500))?;
    // 只读护栏：即便上面用了读写 flags，这里也禁止任何写操作
    let _ = conn.pragma_update(None, "query_only", true);

    // 表缺失或结构异常时同样降级（容忍未来 ZCode 改表）
    let overview = read_overview(&conn).unwrap_or_default();
    let daily = read_daily(&conn).unwrap_or_default();
    let daily_by_model = read_daily_by_model(&conn).unwrap_or_default();
    // 当前前端只渲染 recent daily / daily_by_model / heatmap。
    // 旧的 by_model / by_tool 是全库 GROUP BY，在长期使用后的 ZCode 统计库上会明显拖慢刷新；
    // 保留 DTO 字段但不再为仪表盘刷新执行这两条重查询。
    let by_model = Vec::new();
    let by_tool = Vec::new();
    let heatmap = read_heatmap(&conn).unwrap_or_default();

    Ok(UsageStatsDto {
        available: true,
        db_path,
        generated_at,
        total_tokens: overview.total_tokens,
        input_tokens: overview.input_tokens,
        output_tokens: overview.output_tokens,
        reasoning_tokens: overview.reasoning_tokens,
        cache_read_tokens: overview.cache_read_tokens,
        model_request_count: overview.model_request_count,
        session_count: overview.session_count,
        project_count: overview.project_count,
        tool_call_count: overview.tool_call_count,
        first_at: overview.first_at,
        last_at: overview.last_at,
        daily,
        daily_by_model,
        by_model,
        by_tool,
        heatmap,
    })
}

#[derive(Default)]
struct Overview {
    total_tokens: i64,
    input_tokens: i64,
    output_tokens: i64,
    reasoning_tokens: i64,
    cache_read_tokens: i64,
    model_request_count: i64,
    first_at: i64,
    last_at: i64,
    session_count: i64,
    project_count: i64,
    tool_call_count: i64,
}

/// token 总览：仅统计 status='completed' 的请求，避免把 error/cancelled 的噪声算进 token。
fn read_overview(conn: &Connection) -> AppResult<Overview> {
    // model_usage 聚合
    let mut o = Overview::default();
    let row = conn.query_row(
        "SELECT \
            COUNT(*), \
            COALESCE(SUM(computed_total_tokens),0), \
            COALESCE(SUM(input_tokens),0), \
            COALESCE(SUM(output_tokens),0), \
            COALESCE(SUM(reasoning_tokens),0), \
            COALESCE(SUM(cache_read_input_tokens),0), \
            COALESCE(MIN(started_at),0), \
            COALESCE(MAX(started_at),0) \
         FROM model_usage WHERE status = 'completed'",
        [],
        |r| {
            Ok((
                r.get::<_, i64>(0)?,    // count
                r.get::<_, i64>(1)?,    // total
                r.get::<_, i64>(2)?,    // input
                r.get::<_, i64>(3)?,    // output
                r.get::<_, i64>(4)?,    // reasoning
                r.get::<_, i64>(5)?,    // cache_read
                r.get::<_, i64>(6)?,    // first_at
                r.get::<_, i64>(7)?,    // last_at
            ))
        },
    )?;
    o.model_request_count = row.0;
    o.total_tokens = row.1;
    o.input_tokens = row.2;
    o.output_tokens = row.3;
    o.reasoning_tokens = row.4;
    o.cache_read_tokens = row.5;
    o.first_at = row.6;
    o.last_at = row.7;

    // session / project 计数
    let sp = conn
        .query_row("SELECT COUNT(*), COUNT(DISTINCT project_id) FROM session", [], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))
        })
        .unwrap_or((0, 0));
    o.session_count = sp.0;
    o.project_count = sp.1;

    // 工具调用总数
    o.tool_call_count = conn
        .query_row("SELECT COUNT(*) FROM tool_usage", [], |r| r.get::<_, i64>(0))
        .unwrap_or(0);

    Ok(o)
}

/// 最近 30 天按天 token 趋势（本地时区）。
fn read_daily(conn: &Connection) -> AppResult<Vec<UsageDailyPoint>> {
    let mut stmt = conn.prepare(
        "SELECT date(started_at/1000,'unixepoch','localtime') d, \
                COUNT(*), \
                COALESCE(SUM(computed_total_tokens),0) \
         FROM model_usage \
         WHERE status = 'completed' \
           AND started_at/1000 >= CAST(strftime('%s','now','-30 days') AS INTEGER) \
         GROUP BY d ORDER BY d ASC",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(UsageDailyPoint {
            date: r.get::<_, String>(0)?,
            requests: r.get::<_, i64>(1)?,
            tokens: r.get::<_, i64>(2)?,
        })
    })?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

/// 最近 30 天按天 + 按模型 token 趋势（本地时区）。
fn read_daily_by_model(conn: &Connection) -> AppResult<Vec<UsageDailyModelPoint>> {
    let mut stmt = conn.prepare(
        "SELECT date(started_at/1000,'unixepoch','localtime') d, \
                model_id, \
                COUNT(*), \
                COALESCE(SUM(computed_total_tokens),0) \
         FROM model_usage \
         WHERE status = 'completed' \
           AND started_at/1000 >= CAST(strftime('%s','now','-30 days') AS INTEGER) \
         GROUP BY d, model_id ORDER BY d ASC, COALESCE(SUM(computed_total_tokens),0) DESC",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(UsageDailyModelPoint {
            date: r.get::<_, String>(0)?,
            model: r.get::<_, String>(1)?,
            requests: r.get::<_, i64>(2)?,
            tokens: r.get::<_, i64>(3)?,
        })
    })?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

/// 最近 ~84 天（12 周）的每日活跃度，用于 GitHub 风格热力图。
/// count = 当日对话轮数（按 turn_id / parent_user_message_id 去重），更接近 ZCode 原页面 tooltip 的“轮”。
fn read_heatmap(conn: &Connection) -> AppResult<Vec<UsageHeatCell>> {
    let mut stmt = conn.prepare(
        "SELECT date(started_at/1000,'unixepoch','localtime') d, \
                COUNT(DISTINCT COALESCE(turn_id, parent_user_message_id, logical_request_id)) \
         FROM model_usage \
         WHERE started_at/1000 >= CAST(strftime('%s','now','-84 days') AS INTEGER) \
         GROUP BY d ORDER BY d ASC",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(UsageHeatCell {
            date: r.get::<_, String>(0)?,
            count: r.get::<_, i64>(1)?,
        })
    })?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}
