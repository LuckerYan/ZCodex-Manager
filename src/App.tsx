import { useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { save, open, ask } from "@tauri-apps/plugin-dialog";
import "./App.css";

type ModelQuota = {
  model: string;
  show_name: string;
  total_units: number;
  used_units: number;
  remaining_units: number;
  available_units: number;
  remaining_percent: number;
  period_end?: number | null;
  expires_at?: number | null;
};

type AccountRow = {
  id: number;
  alias: string;
  email?: string | null;
  user_id?: string | null;
  display_name?: string | null;
  hot_switch_ready: boolean;
  is_active: boolean;
  source: string;
  note?: string | null;
  created_at: number;
  updated_at: number;
  last_quota_refresh_at?: number | null;
  plan_name?: string | null;
  plan_status?: string | null;
  plan_ends_at?: number | null;
  models: ModelQuota[];
  quota_error?: string | null;
};

type AuthStart = {
  flow_id: string;
  authorize_url: string;
  expires_at: number;
  poll_interval_sec: number;
};

type ZcodeStatus = {
  zcode_v2_dir: string;
  credentials_exists: boolean;
  config_exists: boolean;
  cache_exists: boolean;
  agent_pids: number[];
  zcode_running: boolean;
  cdp_available: boolean;
};

type UsageDailyPoint = {
  date: string;
  tokens: number;
  requests: number;
};

type UsageDailyModelPoint = {
  date: string;
  model: string;
  tokens: number;
  requests: number;
};

type UsageModelRow = {
  model: string;
  requests: number;
  tokens: number;
  input: number;
  output: number;
};

type UsageHeatCell = {
  date: string;
  count: number;
};

type UsageStats = {
  available: boolean;
  db_path: string;
  generated_at: number;
  total_tokens: number;
  input_tokens: number;
  output_tokens: number;
  reasoning_tokens: number;
  cache_read_tokens: number;
  model_request_count: number;
  session_count: number;
  project_count: number;
  tool_call_count: number;
  first_at: number;
  last_at: number;
  daily: UsageDailyPoint[];
  daily_by_model: UsageDailyModelPoint[];
  by_model: UsageModelRow[];
  heatmap: UsageHeatCell[];
};

type ModelUsageSlice = {
  model: string;
  tokens: number;
  requests: number;
  color: string;
  percent: number;
};

type DailyModelUsage = {
  date: string;
  tokens: number;
  requests: number;
  models: ModelUsageSlice[];
};

type UsageRange = 7 | 30;
type Toast = { tone: "ok" | "warn" | "bad" | "info"; text: string };
type Page = "dashboard" | "accounts" | "settings" | "about";

type AutoSwitchSettings = {
  enabled: boolean;
  thresholdPercent: number;
  intervalSec: 15 | 30 | 60;
  model: string;
};

type AutoSwitchStatus = {
  checking: boolean;
  lastCheckAt?: number;
  lastSwitchAt?: number;
  message: string;
  tone: "idle" | "ok" | "warn" | "bad";
};

const NAV: Array<{ id: Page; label: string; icon: string }> = [
  { id: "dashboard", label: "仪表盘", icon: "◆" },
  { id: "accounts", label: "账号管理", icon: "●" },
  { id: "settings", label: "设置", icon: "▣" },
  { id: "about", label: "关于", icon: "ⓘ" },
];

const MODEL_COLORS = ["#4198f7", "#49c878", "#f6c85f", "#b986ff", "#ff7a90", "#50cfd3", "#f28c42", "#8cc7ff"];
const AUTO_SWITCH_STORAGE_KEY = "zcode-manager:auto-switch:v1";
const ANY_MODEL = "__any__";
const DEFAULT_AUTO_SWITCH: AutoSwitchSettings = {
  enabled: false,
  thresholdPercent: 2,
  intervalSec: 30,
  model: ANY_MODEL,
};

function fmtInt(v: number | null | undefined) {
  if (v === null || v === undefined) return "--";
  return new Intl.NumberFormat("en-US").format(v);
}

function fmtTime(ts?: number | null) {
  if (!ts) return "--";
  return new Date(ts * 1000).toLocaleString("zh-CN", {
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
  });
}

function fmtStatusTime(ms?: number) {
  if (!ms) return "--";
  return new Date(ms).toLocaleTimeString("zh-CN", { hour: "2-digit", minute: "2-digit", second: "2-digit" });
}

function fmtUsageNumber(v: number | null | undefined) {
  if (v === null || v === undefined) return "--";
  if (Math.abs(v) >= 10_000) {
    const n = v / 10_000;
    const fixed = n >= 100 ? n.toFixed(1) : n >= 10 ? n.toFixed(1) : n.toFixed(2);
    return `${fixed.replace(/\.0$/, "")}万`;
  }
  return fmtInt(v);
}

function fmtDateShort(iso: string) {
  const [, month, day] = iso.split("-").map(Number);
  return `${month}月${day}日`;
}

function modelColor(model: string, index = 0) {
  const canonical = model.toLowerCase();
  if (canonical.includes("turbo")) return "#49c878";
  if (canonical.includes("5.2")) return "#4198f7";
  let hash = 0;
  for (const ch of model) hash = (hash * 31 + ch.charCodeAt(0)) >>> 0;
  return MODEL_COLORS[(hash + index) % MODEL_COLORS.length];
}

function dateKey(d: Date) {
  const y = d.getFullYear();
  const m = `${d.getMonth() + 1}`.padStart(2, "0");
  const day = `${d.getDate()}`.padStart(2, "0");
  return `${y}-${m}-${day}`;
}

function addDays(d: Date, days: number) {
  const next = new Date(d);
  next.setDate(next.getDate() + days);
  return next;
}

function buildDailyWindow(stats: UsageStats | null, range: UsageRange) {
  const map = new Map((stats?.daily ?? []).map((d) => [d.date, d]));
  const today = new Date();
  // 与 ZCode 设置页保持一致：最近 N 天展示为“往前 N 天 + 今天”的闭区间。
  return Array.from({ length: range + 1 }, (_, i) => {
    const date = dateKey(addDays(today, i - range));
    const row = map.get(date);
    return { date, tokens: row?.tokens ?? 0, requests: row?.requests ?? 0 };
  });
}

function buildDailyModelWindow(stats: UsageStats | null, range: UsageRange) {
  const dayMap = new Map<string, UsageDailyModelPoint[]>();
  for (const row of stats?.daily_by_model ?? []) {
    const rows = dayMap.get(row.date) ?? [];
    rows.push(row);
    dayMap.set(row.date, rows);
  }
  const today = new Date();
  return Array.from({ length: range + 1 }, (_, i): DailyModelUsage => {
    const date = dateKey(addDays(today, i - range));
    const rows = [...(dayMap.get(date) ?? [])].sort((a, b) => b.tokens - a.tokens);
    const tokens = rows.reduce((sum, row) => sum + row.tokens, 0);
    const requests = rows.reduce((sum, row) => sum + row.requests, 0);
    const models = rows.map((row, idx) => ({
      model: row.model,
      tokens: row.tokens,
      requests: row.requests,
      color: modelColor(row.model, idx),
      percent: tokens > 0 ? (row.tokens / tokens) * 100 : 0,
    }));
    return { date, tokens, requests, models };
  });
}

function modelTotalsForRange(days: DailyModelUsage[]) {
  const map = new Map<string, { tokens: number; requests: number; firstIndex: number }>();
  for (const day of days) {
    for (const model of day.models) {
      const prev = map.get(model.model);
      if (prev) {
        prev.tokens += model.tokens;
        prev.requests += model.requests;
      } else {
        map.set(model.model, { tokens: model.tokens, requests: model.requests, firstIndex: map.size });
      }
    }
  }
  const total = [...map.values()].reduce((sum, row) => sum + row.tokens, 0);
  return [...map.entries()]
    .map(([model, row]) => ({
      model,
      tokens: row.tokens,
      requests: row.requests,
      color: modelColor(model, row.firstIndex),
      percent: total > 0 ? (row.tokens / total) * 100 : 0,
    }))
    .sort((a, b) => b.tokens - a.tokens);
}

function buildHeatmapWindow(stats: UsageStats | null) {
  const map = new Map((stats?.heatmap ?? []).map((d) => [d.date, d.count]));
  const tokenMap = new Map((stats?.daily ?? []).map((d) => [d.date, d.tokens]));
  const today = new Date();
  const start = addDays(today, -83);
  return Array.from({ length: 84 }, (_, i) => {
    const date = dateKey(addDays(start, i));
    return { date, count: map.get(date) ?? 0, tokens: tokenMap.get(date) ?? 0 };
  });
}

function calcCurrentStreak(days: UsageDailyPoint[]) {
  let streak = 0;
  for (let i = days.length - 1; i >= 0; i -= 1) {
    if (days[i].requests <= 0 && days[i].tokens <= 0) break;
    streak += 1;
  }
  return streak;
}

function trendAxisDays(days: Array<{ date: string }>) {
  if (days.length <= 9) return days;
  const ticks: Array<{ date: string }> = [];
  for (let i = 0; i < days.length; i += 5) ticks.push(days[i]);
  const last = days[days.length - 1];
  if (ticks[ticks.length - 1]?.date !== last.date) ticks.push(last);
  return ticks;
}

function errorText(e: unknown) {
  if (typeof e === "string") return e;
  if (e && typeof e === "object" && "message" in e) return String((e as { message: unknown }).message);
  return String(e);
}

function shortErrorText(e: unknown) {
  const text = errorText(e)
    .replace(/^Error:\s*/i, "")
    .replace(/\s+/g, " ")
    .trim();
  return text.length > 72 ? `${text.slice(0, 72)}…` : text;
}

function clamp(n: number, min: number, max: number) {
  return Math.min(max, Math.max(min, n));
}

function normalizeAutoSwitchSettings(value: Partial<AutoSwitchSettings> | null | undefined): AutoSwitchSettings {
  const rawInterval = Number(value?.intervalSec);
  const intervalSec: AutoSwitchSettings["intervalSec"] =
    rawInterval === 15 || rawInterval === 60 ? rawInterval : 30;
  return {
    enabled: Boolean(value?.enabled),
    thresholdPercent: clamp(Number(value?.thresholdPercent ?? DEFAULT_AUTO_SWITCH.thresholdPercent) || 2, 0.1, 100),
    intervalSec,
    model: typeof value?.model === "string" && value.model ? value.model : ANY_MODEL,
  };
}

function readAutoSwitchSettings(): AutoSwitchSettings {
  try {
    const raw = window.localStorage.getItem(AUTO_SWITCH_STORAGE_KEY);
    return normalizeAutoSwitchSettings(raw ? JSON.parse(raw) : DEFAULT_AUTO_SWITCH);
  } catch {
    return DEFAULT_AUTO_SWITCH;
  }
}

function accountLabel(account: AccountRow | null | undefined) {
  if (!account) return "--";
  return account.display_name || account.email || account.alias;
}

function modelLabel(model: ModelQuota | null | undefined) {
  if (!model) return "无模型";
  return model.show_name || model.model;
}

function uniqueModelOptions(accounts: AccountRow[]) {
  const map = new Map<string, string>();
  for (const account of accounts) {
    for (const model of account.models) {
      if (!map.has(model.model)) map.set(model.model, model.show_name || model.model);
    }
  }
  return [...map.entries()]
    .map(([value, label]) => ({ value, label }))
    .sort((a, b) => a.label.localeCompare(b.label, "zh-CN"));
}

function findWatchedModel(account: AccountRow | null | undefined, settings: AutoSwitchSettings) {
  if (!account?.models.length) return null;
  if (settings.model !== ANY_MODEL) {
    return account.models.find((m) => m.model === settings.model || m.show_name === settings.model) ?? null;
  }
  return [...account.models].sort((a, b) => a.remaining_percent - b.remaining_percent)[0] ?? null;
}

function findBestSwitchCandidate(
  accounts: AccountRow[],
  currentId: number,
  triggerModel: ModelQuota,
  settings: AutoSwitchSettings,
) {
  const threshold = settings.thresholdPercent;
  const candidates = accounts
    .filter((account) => account.id !== currentId && !account.quota_error && account.models.length > 0)
    .map((account) => {
      const sameModel = account.models.find((m) => m.model === triggerModel.model);
      const hotSwitchBonus = account.hot_switch_ready ? 1000 : 0;
      return { account, model: sameModel, score: hotSwitchBonus + (sameModel?.remaining_percent ?? 0) };
    })
    .filter((item): item is { account: AccountRow; model: ModelQuota; score: number } => Boolean(item.model && item.model.remaining_percent > threshold))
    .sort((a, b) => b.score - a.score);
  return candidates[0] ?? null;
}

export default function App() {
  const [page, setPage] = useState<Page>("dashboard");
  const [accounts, setAccounts] = useState<AccountRow[]>([]);
  const [selected, setSelected] = useState<number | null>(null);
  const [status, setStatus] = useState<ZcodeStatus | null>(null);
  const [usageStats, setUsageStats] = useState<UsageStats | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [quotaBusy, setQuotaBusy] = useState<string | null>(null);
  const [usageRefreshing, setUsageRefreshing] = useState(false);
  const [accountsReloading, setAccountsReloading] = useState(false);
  const [autoSwitch, setAutoSwitch] = useState<AutoSwitchSettings>(() => readAutoSwitchSettings());
  const [autoStatus, setAutoStatus] = useState<AutoSwitchStatus>({
    checking: false,
    message: "自动切换尚未开启",
    tone: "idle",
  });
  const [toasts, setToasts] = useState<Array<Toast & { id: number }>>([]);
  const [isAuthing, setIsAuthing] = useState(false);
  const cancelRef = useRef<(() => void) | null>(null);
  const toastSeq = useRef(0);
  const usageRefreshSeq = useRef(0);
  const accountsRef = useRef<AccountRow[]>([]);
  const autoSwitchRef = useRef(autoSwitch);
  const busyRef = useRef<string | null>(null);
  const quotaBusyRef = useRef<string | null>(null);
  const isAuthingRef = useRef(false);
  const autoMonitorBusyRef = useRef(false);

  // 所有提示统一走悬浮 toast，自动消失（保留 setToast 调用签名，方便各处复用）
  const setToast = (t: Toast | null) => {
    if (!t) return;
    const id = ++toastSeq.current;
    setToasts((prev) => [...prev, { ...t, id }]);
    const ttl = t.tone === "bad" ? 6000 : t.tone === "warn" ? 5000 : 3500;
    setTimeout(() => setToasts((prev) => prev.filter((x) => x.id !== id)), ttl);
  };

  const selectedAccount = useMemo(
    () => accounts.find((a) => a.id === selected) ?? accounts[0] ?? null,
    [accounts, selected],
  );

  useEffect(() => { accountsRef.current = accounts; }, [accounts]);
  useEffect(() => { autoSwitchRef.current = autoSwitch; }, [autoSwitch]);
  useEffect(() => { busyRef.current = busy; }, [busy]);
  useEffect(() => { quotaBusyRef.current = quotaBusy; }, [quotaBusy]);
  useEffect(() => { isAuthingRef.current = isAuthing; }, [isAuthing]);
  useEffect(() => {
    window.localStorage.setItem(AUTO_SWITCH_STORAGE_KEY, JSON.stringify(autoSwitch));
  }, [autoSwitch]);

  const totals = useMemo(() => {
    const hot = accounts.filter((a) => a.hot_switch_ready).length;
    const validQuota = accounts.filter((a) => a.models.length > 0 && !a.quota_error).length;
    const tokenOnly = accounts.filter((a) => !a.hot_switch_ready).length;
    const glm52 = accounts.reduce((sum, a) => sum + (a.models.find((m) => m.model === "GLM-5.2")?.remaining_units ?? 0), 0);
    const turbo = accounts.reduce((sum, a) => sum + (a.models.find((m) => m.model === "GLM-5-Turbo")?.remaining_units ?? 0), 0);
    return { hot, validQuota, tokenOnly, glm52, turbo };
  }, [accounts]);

  async function loadAccounts() {
    const rows = await invoke<AccountRow[]>("list_accounts");
    setAccounts(rows);
    // 优先选中当前激活账号
    const active = rows.find((a) => a.is_active);
    if (active) {
      setSelected(active.id);
    } else if (!selected && rows[0]) {
      setSelected(rows[0].id);
    }
  }

  function refreshStatus() {
    void invoke<ZcodeStatus>("zcode_status")
      .then(setStatus)
      .catch(() => setToast({ tone: "warn", text: "状态读取失败" }));
  }

  async function refreshUsageStats(showToast = false) {
    if (usageRefreshing) return;
    const seq = ++usageRefreshSeq.current;
    setUsageRefreshing(true);
    if (showToast) setToast({ tone: "info", text: "刷新中" });
    try {
      const stats = await invoke<UsageStats>("read_usage_stats");
      if (seq === usageRefreshSeq.current) {
        setUsageStats(stats);
        if (showToast) setToast({ tone: "ok", text: "已刷新" });
      }
    } catch (e) {
      if (seq === usageRefreshSeq.current) {
        setToast({ tone: "warn", text: `统计失败：${shortErrorText(e)}` });
      }
    } finally {
      if (seq === usageRefreshSeq.current) setUsageRefreshing(false);
    }
  }

  async function load() {
    await loadAccounts();
    refreshStatus();
    void refreshUsageStats(false);
  }

  async function reloadAccounts() {
    if (accountsReloading) return;
    setAccountsReloading(true);
    try {
      await loadAccounts();
      refreshStatus();
      setToast({ tone: "ok", text: "已扫描" });
    } catch (e) {
      setToast({ tone: "bad", text: `扫描失败：${shortErrorText(e)}` });
    } finally {
      setAccountsReloading(false);
    }
  }

  async function run<T>(label: string, task: () => Promise<T>, ok?: string | ((r: T) => string)) {
    setBusy(label);
    setToast({ tone: "info", text: `${label}中` });
    try {
      const result = await task();
      const okText = typeof ok === "function" ? ok(result) : (ok ?? `${label} 完成`);
      setToast({ tone: "ok", text: okText });
      await loadAccounts();
      refreshStatus();
      return result;
    } catch (e) {
      setToast({ tone: "bad", text: `${label}失败：${shortErrorText(e)}` });
      throw e;
    } finally {
      setBusy(null);
    }
  }

  async function runQuotaRefresh<T>(label: string, task: () => Promise<T>, ok?: string | ((r: T) => string)) {
    if (quotaBusy) return;
    setQuotaBusy(label);
    setToast({ tone: "info", text: `${label}中` });
    try {
      const result = await task();
      const okText = typeof ok === "function" ? ok(result) : (ok ?? `${label} 完成`);
      setToast({ tone: "ok", text: okText });
      await loadAccounts();
      return result;
    } catch (e) {
      setToast({ tone: "bad", text: `${label}失败：${shortErrorText(e)}` });
      throw e;
    } finally {
      setQuotaBusy(null);
    }
  }

  async function importCurrent() {
    await run(
      "同步",
      () => invoke<AccountRow>("import_current", { alias: null, note: null }),
      "同步完成",
    );
  }

  async function deleteAccounts(ids: number[]) {
    if (ids.length === 0) return;
    const ok = await ask(`确定删除所选 ${ids.length} 个账号吗？此操作不可恢复。`, {
      title: "删除账号",
      kind: "warning",
    });
    if (!ok) return;
    await run("删除账号", () => invoke<number>("delete_accounts", { ids }), (n) => `已删除 ${n} 个账号`);
  }

  async function exportAccounts(ids: number[]) {
    if (ids.length === 0) {
      setToast({ tone: "warn", text: "先勾选账号" });
      return;
    }
    const path = await save({
      defaultPath: "zcode-accounts.json",
      filters: [{ name: "JSON", extensions: ["json"] }],
    });
    if (!path) return;
    await run(
      "导出账号",
      () => invoke<number>("export_accounts_to_path", { path, ids }),
      (n) => `已导出 ${n} 个`,
    );
  }

  async function importAccountsFile() {
    const selected = await open({
      multiple: false,
      filters: [{ name: "JSON", extensions: ["json"] }],
    });
    if (!selected || typeof selected !== "string") return;
    await run(
      "导入账号",
      () => invoke<number>("import_accounts_from_path", { path: selected }),
      (n) => `已导入 ${n} 个`,
    );
  }

  async function startAuth() {
    setBusy("Auth");
    setIsAuthing(true);
    try {
      const flow = await invoke<AuthStart>("start_browser_auth", { alias: null, note: null });
      setToast({ tone: "info", text: "请授权" });

      let cancelled = false;
      cancelRef.current = () => { cancelled = true; };

      const deadline = Date.now() + Math.max(10_000, flow.expires_at * 1000 - Date.now());
      while (!cancelled && Date.now() < deadline) {
        await new Promise((r) => setTimeout(r, Math.max(1500, flow.poll_interval_sec * 1000)));
        if (cancelled) break;
        const done = await invoke<{ account: AccountRow } | null>("poll_auth", {
          flowIdToken: flow.flow_id,
          alias: "",
          note: null,
        });
        if (done?.account) {
          setToast({ tone: "ok", text: "授权成功" });
          await loadAccounts();
          setSelected(done.account.id);
          setPage("accounts");
          void invoke<AccountRow>("refresh_quota", { accountId: done.account.id })
            .then(() => loadAccounts())
            .catch((e) => setToast({ tone: "warn", text: `额度失败：${shortErrorText(e)}` }));
          return;
        }
      }
      setToast({ tone: cancelled ? "info" : "warn", text: cancelled ? "授权已取消" : "授权超时" });
    } catch (e) {
      setToast({ tone: "bad", text: `授权失败：${shortErrorText(e)}` });
    } finally {
      setBusy(null);
      setIsAuthing(false);
      cancelRef.current = null;
    }
  }

  function cancelAuth() {
    cancelRef.current?.();
    setIsAuthing(false);
    setBusy(null);
  }

  async function refreshOne(id: number) {
    await runQuotaRefresh("刷新额度", () => invoke<AccountRow>("refresh_quota", { accountId: id }), "额度已刷新");
  }

  async function refreshAll() {
    await runQuotaRefresh("刷新全部额度", () => invoke<AccountRow[]>("refresh_all_quotas"), "全部已刷新");
  }

  // 根据勾选刷新：未勾选则刷新全部，勾选 N 个则只刷这 N 个
  async function refreshSelected(ids: number[]) {
    if (ids.length === 0) {
      await refreshAll();
      return;
    }
    await runQuotaRefresh(
      "刷新所选额度",
      () => invoke<AccountRow[]>("refresh_quotas", { ids }),
      `已刷新 ${ids.length} 个`,
    );
  }

  async function hotSwitch(id: number) {
    setBusy("热切换");
    setToast({ tone: "info", text: "切换中" });
    try {
      const res = await invoke<{ message: string }>("hot_switch", { accountId: id, noBackup: false });
      setToast({ tone: res.message?.includes("失败") ? "warn" : "ok", text: "已切换" });
      await loadAccounts();
      refreshStatus();
    } catch (e) {
      setToast({ tone: "bad", text: `切换失败：${shortErrorText(e)}` });
    } finally {
      setBusy(null);
    }
  }

  function updateAutoSwitch(patch: Partial<AutoSwitchSettings>) {
    setAutoSwitch((prev) => normalizeAutoSwitchSettings({ ...prev, ...patch }));
  }

  async function runAutoMonitor(manual = false) {
    if (autoMonitorBusyRef.current) return;
    const settings = autoSwitchRef.current;
    if (!settings.enabled && !manual) return;
    if (busyRef.current || quotaBusyRef.current || isAuthingRef.current) {
      const label = busyRef.current || quotaBusyRef.current || "Auth";
      setAutoStatus((prev) => ({
        ...prev,
        checking: false,
        message: `等待当前任务完成：${label}`,
        tone: "warn",
      }));
      return;
    }

    const rows = accountsRef.current;
    const current = rows.find((a) => a.is_active) ?? selectedAccount;
    if (!current) {
      setAutoStatus({ checking: false, lastCheckAt: Date.now(), message: "没有可监听的当前账号", tone: "warn" });
      return;
    }

    autoMonitorBusyRef.current = true;
    setAutoStatus((prev) => ({
      ...prev,
      checking: true,
      message: `正在刷新当前账号：${accountLabel(current)}`,
      tone: "idle",
    }));

    try {
      const freshCurrent = await invoke<AccountRow>("refresh_quota", { accountId: current.id });
      const currentWithActive = { ...freshCurrent, is_active: true };
      setAccounts((prev) => prev.map((a) => (a.id === current.id ? currentWithActive : a)));

      const watched = findWatchedModel(currentWithActive, settings);
      const watchedName = watched ? modelLabel(watched) : (settings.model === ANY_MODEL ? "任意模型" : settings.model);
      const lastCheckAt = Date.now();
      if (!watched) {
        setAutoStatus({
          checking: false,
          lastCheckAt,
          message: `${accountLabel(current)} 没有 ${watchedName} 的额度数据`,
          tone: "warn",
        });
        return;
      }

      if (watched.remaining_percent > settings.thresholdPercent) {
        setAutoStatus({
          checking: false,
          lastCheckAt,
          message: `${accountLabel(current)} / ${modelLabel(watched)} 剩余 ${watched.remaining_percent.toFixed(2)}%，未触发`,
          tone: "ok",
        });
        return;
      }

      setAutoStatus((prev) => ({
        ...prev,
        checking: true,
        lastCheckAt,
        message: `${modelLabel(watched)} 已低于 ${settings.thresholdPercent}%，正在刷新候选账号`,
        tone: "warn",
      }));

      const allRows = await invoke<AccountRow[]>("refresh_all_quotas");
      const withActive = allRows.map((a) => ({ ...a, is_active: a.id === current.id }));
      setAccounts(withActive);
      const candidate = findBestSwitchCandidate(withActive, current.id, watched, settings);
      if (!candidate) {
        setAutoStatus({
          checking: false,
          lastCheckAt: Date.now(),
          message: `${modelLabel(watched)} 已到阈值，但没有找到同模型余额高于阈值的候选账号`,
          tone: "bad",
        });
        setToast({ tone: "warn", text: "无候选账号" });
        return;
      }

      const res = await invoke<{ message: string }>("hot_switch", { accountId: candidate.account.id, noBackup: false });
      setToast({
        tone: "ok",
        text: "已自动切换",
      });
      setAutoStatus({
        checking: false,
        lastCheckAt: Date.now(),
        lastSwitchAt: Date.now(),
        message: res.message || `已自动切换到 ${accountLabel(candidate.account)}`,
        tone: "ok",
      });
      await loadAccounts();
      refreshStatus();
    } catch (e) {
      setAutoStatus({
        checking: false,
        lastCheckAt: Date.now(),
        message: `监听失败：${errorText(e)}`,
        tone: "bad",
      });
      if (manual) setToast({ tone: "bad", text: `检测失败：${shortErrorText(e)}` });
    } finally {
      autoMonitorBusyRef.current = false;
    }
  }

  async function relaunchZcodeDebug() {
    setBusy("调试模式重启 ZCode");
    setToast({ tone: "info", text: "重启中" });
    try {
      const res = await invoke<{ message: string; cdp_available?: boolean }>("relaunch_zcode_debug");
      setToast({ tone: res.cdp_available ? "ok" : "warn", text: "ZCode 已重启" });
      await loadAccounts();
      refreshStatus();
    } catch (e) {
      setToast({ tone: "bad", text: `重启失败：${shortErrorText(e)}` });
    } finally {
      setBusy(null);
    }
  }

  useEffect(() => {
    load().catch((e) => setToast({ tone: "bad", text: `加载失败：${shortErrorText(e)}` }));
    const timer = window.setInterval(() => refreshStatus(), 3000);
    const onVisible = () => {
      if (!document.hidden) refreshStatus();
    };
    document.addEventListener("visibilitychange", onVisible);
    return () => {
      window.clearInterval(timer);
      document.removeEventListener("visibilitychange", onVisible);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    if (!autoSwitch.enabled) {
      setAutoStatus((prev) => ({
        ...prev,
        checking: false,
        message: "自动切换尚未开启",
        tone: "idle",
      }));
      return;
    }
    setAutoStatus((prev) => ({
      ...prev,
      message: `自动监听中：每 ${autoSwitch.intervalSec} 秒刷新当前账号`,
      tone: "ok",
    }));
    void runAutoMonitor(false);
    const timer = window.setInterval(() => void runAutoMonitor(false), autoSwitch.intervalSec * 1000);
    return () => window.clearInterval(timer);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [autoSwitch.enabled, autoSwitch.intervalSec]);

  return (
    <main className="app-frame">
      <aside className="side-nav">
        <div className="brand-block">
          <div className="brand-mark">Z</div>
        </div>
        <nav>
          {NAV.map((item) => (
            <button key={item.id} className={`nav-item ${page === item.id ? "active" : ""}`} onClick={() => setPage(item.id)}>
              <span>{item.icon}</span>
              <strong>{item.label}</strong>
            </button>
          ))}
        </nav>
        <div className="side-status">
          <span className={`dot ${status?.zcode_running ? "on" : ""}`} />
          <small>{status?.zcode_running ? "ZCode" : "未运行"}</small>
        </div>
      </aside>

      <section className="page-shell">
        <header className="topbar">
          <h1>{NAV.find((n) => n.id === page)?.label}</h1>
          <div className="top-actions">
            <button
              className="ghost"
              onClick={relaunchZcodeDebug}
              disabled={!!busy}
              title="带调试端口(9229)重启 ZCode，启用热切换后自动刷新左下角账号名（仅需一次）"
            >
              {status?.cdp_available ? "调试模式 ✓" : "调试模式重启 ZCode"}
            </button>
          </div>
        </header>

        {page === "dashboard" && <DashboardPage accounts={accounts} totals={totals} usageStats={usageStats} reloadUsage={() => refreshUsageStats(true)} usageRefreshing={usageRefreshing} />}
        {page === "accounts" && (
          <AccountsPage
            accounts={accounts}
            selectedAccount={selectedAccount}
            setSelected={setSelected}
            refreshOne={refreshOne}
            hotSwitch={hotSwitch}
            busy={busy}
            quotaBusy={quotaBusy}
            deleteAccounts={deleteAccounts}
            exportAccounts={exportAccounts}
            importAccountsFile={importAccountsFile}
            startAuth={startAuth}
            cancelAuth={cancelAuth}
            isAuthing={isAuthing}
            importCurrent={importCurrent}
            refreshSelected={refreshSelected}
            reload={reloadAccounts}
            reloading={accountsReloading}
          />
        )}
        {page === "settings" && (
          <SettingsPage
            accounts={accounts}
            currentAccount={accounts.find((a) => a.is_active) ?? selectedAccount}
            settings={autoSwitch}
            status={autoStatus}
            updateSettings={updateAutoSwitch}
            runNow={() => runAutoMonitor(true)}
          />
        )}
        {page === "about" && <AboutPage />}
      </section>

      <div className="toast-stack">
        {toasts.map((t) => (
          <div key={t.id} className={`toast ${t.tone}`}>{t.text}</div>
        ))}
      </div>
    </main>
  );
}

function DashboardPage({ accounts, totals, usageStats, reloadUsage, usageRefreshing }: { accounts: AccountRow[]; totals: { hot: number; validQuota: number; tokenOnly: number; glm52: number; turbo: number }; usageStats: UsageStats | null; reloadUsage: () => Promise<void>; usageRefreshing: boolean }) {
  return (
    <div className="page-grid dashboard-grid">
      <MetricCard label="账号总数" value={accounts.length} hint={`${totals.hot} 可热切换`} />
      <MetricCard label="额度有效" value={totals.validQuota} hint={`${totals.tokenOnly} Token only`} />
      <MetricCard label="GLM-5.2 剩余" value={fmtInt(totals.glm52)} hint="汇总" />
      <MetricCard label="GLM-5-Turbo 剩余" value={fmtInt(totals.turbo)} hint="汇总" />
      <UsageStatsSection stats={usageStats} reloadUsage={reloadUsage} usageRefreshing={usageRefreshing} />
    </div>
  );
}

function UsageStatsSection({ stats, reloadUsage, usageRefreshing }: { stats: UsageStats | null; reloadUsage: () => Promise<void>; usageRefreshing: boolean }) {
  const [range, setRange] = useState<UsageRange>(30);
  const daily = useMemo(() => buildDailyWindow(stats, range), [stats, range]);
  const dailyByModel = useMemo(() => buildDailyModelWindow(stats, range), [stats, range]);
  const modelTotals = useMemo(() => modelTotalsForRange(dailyByModel), [dailyByModel]);
  const heatmap = useMemo(() => buildHeatmapWindow(stats), [stats]);
  const rangeTokens = dailyByModel.reduce((sum, d) => sum + d.tokens, 0) || daily.reduce((sum, d) => sum + d.tokens, 0);
  const rangeMessages = dailyByModel.reduce((sum, d) => sum + d.requests, 0) || daily.reduce((sum, d) => sum + d.requests, 0);
  const activeDays = daily.filter((d) => d.tokens > 0 || d.requests > 0).length;
  const currentStreak = calcCurrentStreak(daily);
  const topModel = modelTotals[0];

  return (
    <section className="usage-board">
      <div className="usage-head">
        <p>时间范围</p>
        <div className="usage-head-actions">
          <div className="range-tabs" aria-label="使用统计时间范围">
            <button className={range === 7 ? "active" : ""} onClick={() => setRange(7)}>最近 7 天</button>
            <button className={range === 30 ? "active" : ""} onClick={() => setRange(30)}>最近 30 天</button>
          </div>
          <button className="usage-refresh ghost" onClick={() => reloadUsage()} disabled={usageRefreshing}>
            {usageRefreshing ? "刷新中..." : "⟳ 刷新"}
          </button>
        </div>
      </div>

      {!stats && usageRefreshing ? (
        <div className="usage-empty">
          <strong>正在读取使用统计</strong>
          <span>正在后台读取 ZCode CLI 统计库，窗口可继续操作。</span>
        </div>
      ) : !stats?.available ? (
        <div className="usage-empty">
          <strong>暂无使用统计</strong>
          <span>未找到 ZCode CLI 使用统计库，或当前还没有模型请求记录。</span>
        </div>
      ) : (
        <>
          <div className="usage-metrics">
            <UsageMiniCard icon="♨" label="tokens 用量" value={fmtUsageNumber(rangeTokens || stats.total_tokens)} />
            <UsageMiniCard icon="▱" label="会话数量" value={fmtInt(stats.session_count)} />
            <UsageMiniCard icon="□" label="消息数量" value={fmtInt(rangeMessages || stats.model_request_count)} />
            <UsageMiniCard icon="▦" label="活跃天数" value={fmtInt(activeDays)} />
            <UsageMiniCard icon="▣" label="当前连续天数" value={fmtInt(currentStreak)} />
            <UsageMiniCard
              icon="⌁"
              label="最常用模型"
              value={topModel?.model ?? "--"}
              hint={topModel ? `占比 ${topModel.percent < 1 && topModel.percent > 0 ? topModel.percent.toFixed(1) : Math.round(topModel.percent)}%` : "暂无数据"}
              compact
            />
          </div>

          <section className="usage-panel heat-panel">
            <div className="usage-panel-title">
              <h3>活跃热力图</h3>
              <div className="heat-legend"><span>较少</span><i /><i /><i /><i /><i /><span>较多</span></div>
            </div>
            <Heatmap cells={heatmap} />
          </section>

          <section className="usage-panel trend-panel">
            <h3>按天 Token 趋势</h3>
            <DailyTrend days={dailyByModel} models={modelTotals} />
          </section>

          <section className="usage-panel model-usage-panel">
            <h3>模型用量</h3>
            <ModelUsage models={modelTotals} centerTokens={rangeTokens || stats.total_tokens} />
          </section>
        </>
      )}

    </section>
  );
}

function UsageMiniCard({ icon, label, value, hint, compact = false }: { icon: string; label: string; value: string | number; hint?: string; compact?: boolean }) {
  return (
    <div className={`usage-mini-card ${compact ? "compact" : ""}`}>
      <span className="usage-mini-label"><i>{icon}</i>{label}</span>
      <strong>{value}</strong>
      {hint && <small>{hint}</small>}
    </div>
  );
}

function Heatmap({ cells }: { cells: Array<{ date: string; count: number; tokens: number }> }) {
  const max = Math.max(1, ...cells.map((c) => c.count));
  return (
    <div className="heatmap-grid">
      {cells.map((cell) => {
        const level = cell.count === 0 ? 0 : Math.max(1, Math.ceil((cell.count / max) * 5));
        return (
          <span
            key={cell.date}
            className={`heat-cell level-${level}`}
          >
            <span className="usage-tooltip heat-tooltip">
              {fmtDateShort(cell.date)}：{fmtUsageNumber(cell.tokens)} Tokens · {cell.count} 轮
            </span>
          </span>
        );
      })}
    </div>
  );
}

function DailyTrend({ days, models }: { days: DailyModelUsage[]; models: ModelUsageSlice[] }) {
  const max = Math.max(1, ...days.map((d) => d.tokens));
  const axis = trendAxisDays(days);
  return (
    <>
      <div className="daily-chart" style={{ gridTemplateColumns: `repeat(${days.length}, 1fr)` }}>
        {days.map((day) => {
          const rows = day.models.length > 0 ? day.models : [{ model: "无数据", tokens: 0, requests: 0, color: "#4198f7", percent: 0 }];
          return (
          <span key={day.date} className="daily-bar-slot">
            <span className="daily-bar" style={{ height: `${Math.max(1, (day.tokens / max) * 100)}%` }}>
              {rows.map((row) => (
                <span
                  key={row.model}
                  className="daily-segment"
                  style={{
                    background: row.color,
                    height: `${day.tokens > 0 ? Math.max(2, (row.tokens / day.tokens) * 100) : 100}%`,
                  }}
                />
              ))}
              <span className="usage-tooltip daily-tooltip">
                <strong>{fmtDateShort(day.date)} - {fmtUsageNumber(day.tokens)} tokens</strong>
                {day.models.length === 0 ? (
                  <span><i style={{ background: "#4198f7" }} />暂无模型<em>0</em></span>
                ) : day.models.map((row) => (
                  <span key={row.model}><i style={{ background: row.color }} />{row.model}<em>{fmtInt(row.tokens)}</em></span>
                ))}
              </span>
            </span>
          </span>
          );
        })}
      </div>
      <div className="chart-axis">
        {axis.map((day) => <span key={day.date}>{fmtDateShort(day.date)}</span>)}
      </div>
      <div className="chart-legend multi">
        {models.length === 0 ? <span><i style={{ background: "#4198f7" }} />暂无模型</span> : models.map((model) => (
          <span key={model.model}><i style={{ background: model.color }} />{model.model}</span>
        ))}
      </div>
    </>
  );
}

function ModelUsage({ models, centerTokens }: { models: ModelUsageSlice[]; centerTokens: number }) {
  const [hoveredModel, setHoveredModel] = useState<ModelUsageSlice | null>(null);
  const total = Math.max(1, models.reduce((sum, m) => sum + m.tokens, 0));
  const radius = 42;
  const circumference = 2 * Math.PI * radius;
  let offset = 0;
  const tooltipModel = hoveredModel ?? models[0] ?? null;

  return (
    <div className="model-usage-content">
      <div className="donut">
        <svg className="donut-svg" viewBox="0 0 120 120" aria-label="模型用量占比">
          <circle className="donut-track" cx="60" cy="60" r={radius} />
          {models.length === 0 ? (
            <circle className="donut-segment-svg" cx="60" cy="60" r={radius} stroke="#303030" />
          ) : models.map((model) => {
            const length = (model.tokens / total) * circumference;
            const dashOffset = -offset;
            offset += length;
            return (
              <circle
                key={model.model}
                className="donut-segment-svg"
                cx="60"
                cy="60"
                r={radius}
                stroke={model.color}
                strokeDasharray={`${length} ${circumference - length}`}
                strokeDashoffset={dashOffset}
                onMouseEnter={() => setHoveredModel(model)}
                onMouseLeave={() => setHoveredModel(null)}
              />
            );
          })}
        </svg>
        <div className="donut-center">
          <strong>{fmtUsageNumber(centerTokens)}</strong>
          <span>tokens</span>
        </div>
        {tooltipModel && (
          <span className="usage-tooltip donut-tooltip">
            <strong><i style={{ background: tooltipModel.color }} />{tooltipModel.model}</strong>
            <span>{fmtUsageNumber(tooltipModel.tokens)} tokens <em>{tooltipModel.percent < 1 && tooltipModel.percent > 0 ? tooltipModel.percent.toFixed(1) : Math.round(tooltipModel.percent)}%</em></span>
          </span>
        )}
      </div>
      <div className="model-list">
        {models.length === 0 ? (
          <p className="muted">暂无模型用量</p>
        ) : models.map((model) => {
          const percent = model.percent < 1 && model.percent > 0 ? model.percent.toFixed(1) : Math.round(model.percent);
          return (
            <div className="model-row" key={model.model}>
              <span><i style={{ background: model.color }} />{model.model}</span>
              <strong>{fmtUsageNumber(model.tokens)} tokens</strong>
              <em>{percent}%</em>
            </div>
          );
        })}
      </div>
    </div>
  );
}

function AccountsPage({ accounts, selectedAccount, setSelected, refreshOne, hotSwitch, busy, quotaBusy, deleteAccounts, exportAccounts, importAccountsFile, startAuth, cancelAuth, isAuthing, importCurrent, refreshSelected, reload, reloading }: { accounts: AccountRow[]; selectedAccount: AccountRow | null; setSelected: (id: number) => void; refreshOne: (id: number) => Promise<void>; hotSwitch: (id: number) => Promise<void>; busy: string | null; quotaBusy: string | null; deleteAccounts: (ids: number[]) => Promise<void>; exportAccounts: (ids: number[]) => Promise<void>; importAccountsFile: () => Promise<void>; startAuth: () => Promise<void>; cancelAuth: () => void; isAuthing: boolean; importCurrent: () => Promise<void>; refreshSelected: (ids: number[]) => Promise<void>; reload: () => Promise<void>; reloading: boolean }) {
  const [detailId, setDetailId] = useState<number | null>(null);
  const [checked, setChecked] = useState<Set<number>>(new Set());
  const detailAccount = accounts.find((a) => a.id === detailId) ?? null;
  const openDetail = (id: number) => { setSelected(id); setDetailId(id); };

  // 账号列表变化时，剔除已不存在的勾选项
  useEffect(() => {
    setChecked((prev) => {
      const next = new Set<number>();
      for (const a of accounts) if (prev.has(a.id)) next.add(a.id);
      return next.size === prev.size ? prev : next;
    });
  }, [accounts]);

  const allChecked = accounts.length > 0 && checked.size === accounts.length;
  const toggleAll = () => setChecked(allChecked ? new Set() : new Set(accounts.map((a) => a.id)));
  const toggleOne = (id: number) =>
    setChecked((prev) => {
      const n = new Set(prev);
      if (n.has(id)) n.delete(id); else n.add(id);
      return n;
    });
  const ids = [...checked];

  return (
    <section className="workspace split-page">
      <div className="accounts">
        <div className="accounts-toolbar">
          <span className="muted toolbar-count">{checked.size > 0 ? `已选 ${checked.size} 个账号` : `共 ${accounts.length} 个账号`}</span>
          <div className="spacer" />
          <button className="ghost" onClick={() => reload()} disabled={!!busy || reloading}>{reloading ? "扫描中..." : "重新扫描"}</button>
          {isAuthing ? (
            <button className="danger" onClick={cancelAuth}>取消授权</button>
          ) : (
            <button onClick={startAuth} disabled={!!busy}>开始 Auth</button>
          )}
          <button onClick={importCurrent} disabled={!!busy || isAuthing}>同步</button>
          <button className="ghost" onClick={importAccountsFile} disabled={!!busy}>导入</button>
          <button className="ghost" onClick={() => exportAccounts(ids)} disabled={!!busy || checked.size === 0}>导出</button>
          <button onClick={() => refreshSelected(ids)} disabled={!!busy || !!quotaBusy || accounts.length === 0}>
            {quotaBusy ? "刷新中..." : (checked.size > 0 ? `刷新 (${checked.size})` : "刷新")}
          </button>
          <button className="danger" onClick={() => deleteAccounts(ids)} disabled={!!busy || checked.size === 0}>删除</button>
        </div>
        {accounts.length === 0 ? (
          <Empty text="暂无账号，请点击「开始 Auth」「同步」或「导入」添加" />
        ) : (
          <table className="account-table">
            <thead>
              <tr>
                <th className="col-check"><input type="checkbox" checked={allChecked} onChange={toggleAll} aria-label="全选" /></th>
                <th>账号</th>
                <th>套餐</th>
                <th>状态</th>
                <th>到期</th>
                <th className="col-actions">操作</th>
              </tr>
            </thead>
            <tbody>
              {accounts.map((account) => (
                <AccountTableRow
                  key={account.id}
                  account={account}
                  selected={selectedAccount?.id === account.id}
                  checked={checked.has(account.id)}
                  onToggle={() => toggleOne(account.id)}
                  onDetail={() => openDetail(account.id)}
                  refreshOne={refreshOne}
                  hotSwitch={hotSwitch}
                  busy={busy}
                  quotaBusy={quotaBusy}
                />
              ))}
            </tbody>
          </table>
        )}
      </div>
      {detailAccount && (
        <div className="modal-overlay" onClick={() => setDetailId(null)}>
          <div className="modal-card" onClick={(e) => e.stopPropagation()}>
            <button className="modal-close" onClick={() => setDetailId(null)} aria-label="关闭">×</button>
            <AccountDetail account={detailAccount} />
          </div>
        </div>
      )}
    </section>
  );
}

function SettingsPage({
  accounts,
  currentAccount,
  settings,
  status,
  updateSettings,
  runNow,
}: {
  accounts: AccountRow[];
  currentAccount: AccountRow | null;
  settings: AutoSwitchSettings;
  status: AutoSwitchStatus;
  updateSettings: (patch: Partial<AutoSwitchSettings>) => void;
  runNow: () => Promise<void>;
}) {
  const modelOptions = useMemo(() => uniqueModelOptions(accounts), [accounts]);
  const watched = findWatchedModel(currentAccount, settings);
  const candidateCount = useMemo(() => {
    if (!currentAccount || !watched) return 0;
    return accounts.filter((a) => {
      if (a.id === currentAccount.id || a.quota_error) return false;
      const m = a.models.find((x) => x.model === watched.model);
      return Boolean(m && m.remaining_percent > settings.thresholdPercent);
    }).length;
  }, [accounts, currentAccount, settings, watched]);

  return (
    <section className="settings-page">
      <div className="settings-hero">
        <div>
          <h2>自动切换账号</h2>
        </div>
        <label className={`switch-card ${settings.enabled ? "on" : ""}`}>
          <input
            type="checkbox"
            checked={settings.enabled}
            onChange={(e) => updateSettings({ enabled: e.currentTarget.checked })}
          />
          <span />
          <strong>{settings.enabled ? "已启用" : "未启用"}</strong>
        </label>
      </div>

      <div className="settings-layout">
        <section className="settings-panel primary">
          <div className="section-head">
            <div>
              <h2>自动切换条件</h2>
            </div>
            <button className="ghost" onClick={runNow} disabled={status.checking}>
              {status.checking ? "检测中..." : "立即检测一次"}
            </button>
          </div>

          <div className="setting-field">
            <label>监听模型</label>
            <select
              value={settings.model}
              onChange={(e) => updateSettings({ model: e.currentTarget.value })}
            >
              <option value={ANY_MODEL}>任意模型</option>
              {modelOptions.map((m) => (
                <option key={m.value} value={m.value}>{m.label}</option>
              ))}
            </select>
          </div>

          <div className="threshold-control">
            <div className="threshold-readout">
              <span>触发阈值</span>
              <strong>{settings.thresholdPercent.toFixed(settings.thresholdPercent < 1 ? 1 : 0)}%</strong>
            </div>
            <input
              type="range"
              min="0.1"
              max="100"
              step="0.1"
              value={settings.thresholdPercent}
              onChange={(e) => updateSettings({ thresholdPercent: Number(e.currentTarget.value) })}
            />
            <div className="threshold-row">
              <input
                type="number"
                min="0.1"
                max="100"
                step="0.1"
                value={settings.thresholdPercent}
                onChange={(e) => updateSettings({ thresholdPercent: Number(e.currentTarget.value) })}
              />
              <span>%</span>
            </div>
          </div>

          <div className="setting-field">
            <label>自动刷新间隔</label>
            <div className="interval-pills">
              {[15, 30, 60].map((sec) => (
                <button
                  key={sec}
                  className={settings.intervalSec === sec ? "active" : ""}
                  onClick={() => updateSettings({ intervalSec: sec as AutoSwitchSettings["intervalSec"] })}
                >
                  每 {sec} 秒
                </button>
              ))}
            </div>
          </div>
        </section>

        <aside className="settings-panel monitor">
          <h2>监听状态</h2>
          <div className={`monitor-card ${status.tone}`}>
            <span className={`dot ${settings.enabled ? "on" : ""}`} />
            <strong>{status.checking ? "正在检查" : settings.enabled ? "监听中" : "已暂停"}</strong>
          </div>
          <div className="monitor-facts">
            <div><span>当前账号</span><strong>{accountLabel(currentAccount)}</strong></div>
            <div><span>当前模型</span><strong>{watched ? modelLabel(watched) : "--"}</strong></div>
            <div><span>当前剩余</span><strong>{watched ? `${watched.remaining_percent.toFixed(2)}%` : "--"}</strong></div>
            <div><span>有余额候选</span><strong>{candidateCount}</strong></div>
            <div><span>上次检测</span><strong>{fmtStatusTime(status.lastCheckAt)}</strong></div>
            <div><span>上次切换</span><strong>{fmtStatusTime(status.lastSwitchAt)}</strong></div>
          </div>
        </aside>
      </div>
    </section>
  );
}

function AboutPage() {
  return (
    <article className="about-article">
      <header className="about-hero">
        <h2>ZCode 管理器</h2>
        <p>
          一个用于管理多个 ZCode（Z.ai Coding Plan）账号的桌面工具：集中保存账号凭证、查询与刷新各账号额度，
          并支持在不重启 ZCode 的前提下「热切换」当前生效账号。
        </p>
      </header>

      <section>
        <h3>核心功能</h3>
        <ul className="about-list">
          <li><strong>多账号管理</strong> —— 增删账号、批量导入 / 导出 JSON（明文 token，便于跨机迁移）。</li>
          <li><strong>OAuth 授权登录</strong> —— 通过浏览器完成 Z.ai 授权，自动抓取并加密保存账号凭证。</li>
          <li><strong>同步当前账号</strong> —— 一键把当前 ZCode 正在使用的账号导入到管理器。</li>
          <li><strong>额度查询与刷新</strong> —— 按套餐 / 模型展示剩余额度，支持「刷新全部」或「按勾选刷新」。</li>
          <li><strong>账号热切换</strong> —— 借助 CDP 调试端口（9229）改写运行中的 ZCode 凭证，免重启即时切号。</li>
          <li><strong>运行状态监控</strong> —— 实时查看 ZCode 进程、凭证 / 配置文件、调试端口等状态。</li>
        </ul>
      </section>

      <section>
        <h3>整体架构</h3>
        <p>
          基于 <strong>Tauri 2</strong> 的桌面应用，采用「前端 WebView + Rust 原生后端」双层结构，
          两侧通过 Tauri 的 <code>invoke</code> 命令通道通信。
        </p>
        <ul className="about-list">
          <li><strong>前端（UI 层）</strong> —— React 19 + TypeScript + Vite，负责账号列表、仪表盘、状态展示与交互。</li>
          <li><strong>后端（命令层）</strong> —— Rust，对前端暴露 <code>list_accounts</code> / <code>refresh_quotas</code> / <code>hot_switch</code> 等命令。</li>
          <li><strong>数据层</strong> —— 本地 SQLite 数据库（rusqlite，内置编译），token 经 AES-GCM 加密后落盘。</li>
          <li><strong>ZCode 对接层</strong> —— 读写 ZCode v2 的 credentials / config 文件，并通过 CDP（WebSocket）控制运行中的 ZCode 进程。</li>
        </ul>
      </section>

      <section>
        <h3>关键技术栈与语言</h3>
        <p>
          <strong>前端</strong>采用 TypeScript / React 19，由 Vite 7 构建，通过 @tauri-apps/api（invoke 与 dialog 插件）与后端通信；
          <strong>后端</strong>以 Rust 编写，基于 Tauri 2 应用框架，使用 rusqlite 做 SQLite 持久化、aes-gcm 加密本地凭证、
          reqwest 发起额度 / 授权 HTTP 请求、tungstenite 通过 CDP WebSocket 实现热切换，并辅以
          serde / serde_json、chrono、sha2、base64 等基础库。
        </p>
      </section>
    </article>
  );
}

function MetricCard({ label, value, hint }: { label: string; value: string | number; hint: string }) {
  return <section className="metric-card"><span>{label}</span><strong>{value}</strong><small>{hint}</small></section>;
}

function Empty({ text }: { text: string }) {
  return <div className="empty-state"><p>{text}</p></div>;
}

function AccountTableRow({ account, selected, checked, onToggle, onDetail, refreshOne, hotSwitch, busy, quotaBusy }: { account: AccountRow; selected: boolean; checked: boolean; onToggle: () => void; onDetail: () => void; refreshOne: (id: number) => Promise<void>; hotSwitch: (id: number) => Promise<void>; busy: string | null; quotaBusy: string | null }) {
  const quotaError = account.quota_error
    || (account.last_quota_refresh_at && !account.plan_name && account.models.length === 0
      ? "刷新完成，但接口未返回套餐或模型额度数据"
      : null);
  return (
    <tr className={`account-tr ${selected ? "selected" : ""}`}>
      <td className="col-check"><input type="checkbox" checked={checked} onChange={onToggle} aria-label="选择账号" /></td>
      <td>
        <div className="account-cell">
          <span className="avatar">{(account.display_name || account.email || account.alias).slice(0, 2).toUpperCase()}</span>
          <div className="account-main">
            <strong>{account.display_name || account.alias}{account.is_active ? <i className="active-mark">● 当前</i> : null}</strong>
            <small>{account.email ?? account.alias}</small>
          </div>
        </div>
      </td>
      <td title={quotaError ?? undefined}>{quotaError ? "无额度数据" : (account.plan_name ?? "未刷新")}</td>
      <td title={quotaError ?? undefined}>{quotaError ?? account.plan_status ?? "--"}</td>
      <td>{fmtTime(account.plan_ends_at)}</td>
      <td className="col-actions">
        <div className="row-actions">
          <button onClick={() => refreshOne(account.id)} disabled={!!busy || !!quotaBusy}>刷新</button>
          <button onClick={() => hotSwitch(account.id)} disabled={!!busy}>切换</button>
          <button onClick={onDetail}>详情</button>
        </div>
      </td>
    </tr>
  );
}

function AccountDetail({ account }: { account: AccountRow | null }) {
  if (!account) return <aside className="detail"><Empty text="选择一个账号查看详情" /></aside>;
  return (
    <aside className="detail">
      <div className="detail-head">
        <div>
          <h2>{account.display_name || account.alias}</h2>
          <p>{account.email ?? "无邮箱"}</p>
        </div>
        <span className={`badge ${account.hot_switch_ready ? "ready" : "missing"}`}>{account.hot_switch_ready ? "可热切换" : "仅 Token"}</span>
      </div>
      <div className="plan-card">
        <div><span>套餐</span><strong>{account.plan_name ?? "未刷新"}</strong></div>
        <div><span>状态</span><strong>{account.plan_status ?? "--"}</strong></div>
        <div><span>到期</span><strong>{fmtTime(account.plan_ends_at)}</strong></div>
      </div>
      {account.quota_error && <div className="error-box">{account.quota_error}</div>}
      <div className="models">
        {account.models.length === 0 ? <p className="muted">暂无额度数据</p> : account.models.map((model) => (
          <div className="model-card" key={model.model}>
            <div className="model-title"><strong>{model.show_name}</strong><span>{model.remaining_percent.toFixed(2)}%</span></div>
            <div className="bar"><i style={{ width: `${Math.max(0, Math.min(100, model.remaining_percent))}%` }} /></div>
            <p>{fmtInt(model.remaining_units)} / {fmtInt(model.total_units)} · 重置 {fmtTime(model.period_end)}</p>
          </div>
        ))}
      </div>
    </aside>
  );
}
