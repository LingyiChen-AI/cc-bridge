// 用量统计：每日 / 每月按线路堆叠的柱状图（ECharts，按需引入）。
// main.ts 每次 render() 都整页 innerHTML 重建，图表宿主若跟着重建会每次重播入场动画、闪一下；
// 所以宿主节点常驻在模块里：重绘后用它替换新生成的占位 #usage-chart-slot，实例不 dispose，数据没变就不 setOption。
import * as echarts from 'echarts/core';
import { BarChart } from 'echarts/charts';
import { GridComponent } from 'echarts/components';
import { CanvasRenderer } from 'echarts/renderers';
import type { UsageReport, UsageTotals } from './api';

echarts.use([BarChart, GridComponent, CanvasRenderer]);

type Route = 'claude' | 'codex' | 'api';
export interface ChartInput { keys: string[]; series: { route: Route; values: number[] }[]; unit: 'usd' | 'tokens'; selected: string | null }

// 堆叠顺序固定（自下而上），线路色全图一致、深浅色相同
const ROUTES: Route[] = ['claude', 'codex', 'api'];
const COLORS: Record<Route, string> = { claude: '#D97757', codex: '#10A37F', api: '#6E59D9' };
const DIM = 0.35; // 选中某个桶时，其余柱子的不透明度

const tokens = (x: UsageTotals) => x.input + x.cacheRead + x.cacheWrite + x.output;
const pad = (n: number) => String(n).padStart(2, '0');

/** from..to（YYYY-MM-DD）范围内的全部日桶或月桶 key；用 UTC 逐日推进，不受夏令时影响 */
function bucketKeys(from: string, to: string, gran: 'day' | 'month'): string[] {
  const out: string[] = [];
  if (gran === 'month') {
    let [y, m] = from.slice(0, 7).split('-').map(Number);
    const end = to.slice(0, 7);
    for (let k = `${y}-${pad(m)}`; k <= end && out.length < 1200; k = `${y}-${pad(m)}`) {
      out.push(k);
      if (++m > 12) { m = 1; y++; }
    }
    return out;
  }
  const [y, m, d] = from.split('-').map(Number);
  const t = new Date(Date.UTC(y, m - 1, d));
  for (let k = from; k <= to && out.length < 1200; ) {
    out.push(k);
    t.setUTCDate(t.getUTCDate() + 1);
    k = `${t.getUTCFullYear()}-${pad(t.getUTCMonth() + 1)}-${pad(t.getUTCDate())}`;
  }
  return out;
}

/** 报表 → 图表输入：补齐范围内空天 / 空月为 0；范围内总金额为 0（全是未知价格模型）时改画 Tokens */
export function chartInput(r: UsageReport, gran: 'day' | 'month', selected: string | null): ChartInput {
  const keys = bucketKeys(r.from, r.to, gran);
  const unit = r.totals.costUsd === 0 ? 'tokens' : 'usd';
  const at = new Map(r.buckets.map((b) => [b.key, b]));
  const series = ROUTES.filter((rt) => r.byRoute[rt]).map((route) => ({
    route,
    values: keys.map((k) => {
      const x = at.get(k)?.byRoute[route];
      return x ? (unit === 'usd' ? x.costUsd : tokens(x)) : 0;
    }),
  }));
  return { keys, series, unit, selected };
}

/** y 轴刻度：金额 $12 / $1.2K，Tokens 1.2K / 3.4M / 1.1B */
function axisFmt(unit: ChartInput['unit'], v: number): string {
  const short = (n: number) => {
    const a = Math.abs(n);
    if (a >= 1e9) return `${+(n / 1e9).toFixed(1)}B`;
    if (a >= 1e6) return `${+(n / 1e6).toFixed(1)}M`;
    if (a >= 1e3) return `${+(n / 1e3).toFixed(1)}K`;
    return `${+n.toFixed(2)}`;
  };
  return unit === 'usd' ? `$${short(v)}` : short(v);
}

/** 坐标轴文字 / 网格线颜色取自 styles.css 的 token，深色模式自动跟随 */
function themeColors() {
  const cs = getComputedStyle(document.documentElement);
  const v = (name: string, dflt: string) => cs.getPropertyValue(name).trim() || dflt;
  return { text: v('--text-3', '#958d84'), line: v('--border', '#ebe7e2'), axis: v('--border-strong', '#d9d3cc') };
}

function buildOption(input: ChartInput, animation: boolean): echarts.EChartsCoreOption {
  const c = themeColors();
  const { keys, selected, unit } = input;
  const month = keys[0]?.length === 7;
  return {
    animation,
    animationDuration: 500,
    tooltip: { show: false },
    grid: { left: 4, right: 6, top: 10, bottom: 2, containLabel: true },
    xAxis: {
      type: 'category',
      data: keys,
      axisLine: { lineStyle: { color: c.axis } },
      axisTick: { show: false },
      // 日桶只显示 MM-DD；标签放不下时 ECharts 自动隔几个显示，hideOverlap 兜底防重叠
      axisLabel: { color: c.text, fontSize: 10, hideOverlap: true, formatter: (k: string) => (month ? k : k.slice(5)) },
    },
    yAxis: {
      type: 'value',
      splitNumber: 3,
      axisLabel: { color: c.text, fontSize: 10, formatter: (v: number) => axisFmt(unit, v) },
      splitLine: { lineStyle: { color: c.line } },
    },
    series: input.series.map((s) => ({
      type: 'bar',
      name: s.route,
      stack: 'all',
      barMaxWidth: 26,
      cursor: 'pointer',
      itemStyle: { color: COLORS[s.route] },
      emphasis: { disabled: true },
      data: s.values.map((v, i) => ({ value: v, itemStyle: { opacity: selected && keys[i] !== selected ? DIM : 1 } })),
    })),
  };
}

// ───────── 常驻实例 ─────────
let host: HTMLDivElement | null = null;
let chart: echarts.ECharts | null = null;
let lastKey = '';
let lastInput: ChartInput | null = null;
let pick: (key: string) => void = () => {};

/** 宿主在文档里且尺寸与画布不一致才 resize（尺寸相同时 resize 也会重绘一遍，没必要） */
function fit() {
  if (!chart || !host?.isConnected || host.clientWidth === 0) return;
  if (host.clientWidth !== chart.getWidth() || host.clientHeight !== chart.getHeight()) chart.resize();
}

/** 指纹：数据 + 当前主题色；两者都没变就跳过 setOption */
const fingerprint = (input: ChartInput) => JSON.stringify([input, themeColors()]);

function apply(input: ChartInput) {
  if (!chart) return;
  const key = fingerprint(input);
  if (key === lastKey) return;
  const first = lastKey === '';
  lastKey = key;
  chart.setOption(buildOption(input, first), true);
}

/**
 * 把常驻图表挂到本次重绘生成的占位上。slot 为 null（不在统计页 / 「今天」模式）时什么都不做：
 * 宿主已随旧 DOM 一起脱离文档，实例保留，下次回来时搬回并 resize。
 */
export function mountUsageChart(slot: HTMLElement | null, input: ChartInput, onPick: (key: string) => void): void {
  if (!slot) return;
  pick = onPick;
  lastInput = input;
  if (!host) {
    host = document.createElement('div');
    host.className = slot.className;
    slot.replaceWith(host);
    chart = echarts.init(host, undefined, { renderer: 'canvas' });
    // 点整列（含柱子上方空白、高度很小的柱）都算选中该桶，比只认柱子本身好点
    chart.getZr().on('click', (e) => {
      if (!chart || !lastInput) return;
      const p = [e.offsetX, e.offsetY];
      if (!chart.containPixel('grid', p)) return;
      const i = Math.round((chart.convertFromPixel({ gridIndex: 0 }, p) as number[])[0]);
      const k = lastInput.keys[i];
      if (k) pick(k);
    });
    new ResizeObserver(() => fit()).observe(host);
    // 系统切换深浅色：重新取主题色（指纹随之变化）
    matchMedia('(prefers-color-scheme: dark)').addEventListener('change', () => { if (lastInput) apply(lastInput); });
  } else {
    slot.replaceWith(host);
    fit(); // 离开期间窗口宽度可能变了
  }
  apply(input);
}

