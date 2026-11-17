// 下载钉死版本的上游 claude-codex，校验 sha256，放到 src-tauri/binaries/claude-codex-<triple>[.exe]
import { execSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { existsSync, mkdirSync, readFileSync, writeFileSync, rmSync, readdirSync, statSync, copyFileSync, chmodSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { tmpdir } from 'node:os';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const { version } = JSON.parse(readFileSync(join(root, 'sidecar.json'), 'utf8'));
const triple = process.env.TAURI_TARGET_TRIPLE
  || execSync('rustc -vV').toString().match(/host: (\S+)/)[1];

const ASSETS = {
  'x86_64-pc-windows-msvc': 'windows-amd64.zip',
  'aarch64-pc-windows-msvc': 'windows-arm64.zip',
  'x86_64-apple-darwin': 'darwin-amd64.tar.gz',
  'aarch64-apple-darwin': 'darwin-arm64.tar.gz',
  'x86_64-unknown-linux-gnu': 'linux-amd64.tar.gz',
  'aarch64-unknown-linux-gnu': 'linux-arm64.tar.gz',
};
const asset = ASSETS[triple];
if (!asset) throw new Error(`不支持的平台: ${triple}`);

const isWin = triple.includes('windows');
const outDir = join(root, 'src-tauri', 'binaries');
const outFile = join(outDir, `claude-codex-${triple}${isWin ? '.exe' : ''}`);
const base = `https://github.com/fcakyon/claude-code-with-codex/releases/download/${version}/claude-codex-${asset}`;

async function fetchBuf(url) {
  const proxy = process.env.HTTPS_PROXY || process.env.https_proxy;
  if (proxy) {
    // Node 内置 fetch 不认 HTTPS_PROXY，交给 curl
    return execSync(`curl -fsSL --retry 3 -x "${proxy}" "${url}"`, { maxBuffer: 256 << 20 });
  }
  const r = await fetch(url);
  if (!r.ok) throw new Error(`下载失败 ${r.status}: ${url}`);
  return Buffer.from(await r.arrayBuffer());
}

const sumText = (await fetchBuf(`${base.replace(/\.(zip|tar\.gz)$/, '')}.sha256`)).toString();
const expected = sumText.trim().split(/\s+/)[0].toLowerCase();
const stampFile = outFile + '.sha256';
if (existsSync(outFile) && existsSync(stampFile) && readFileSync(stampFile, 'utf8').trim() === expected) {
  console.log(`sidecar 已是 ${version}，跳过`);
  process.exit(0);
}

const archive = await fetchBuf(base);
const actual = createHash('sha256').update(archive).digest('hex');
if (actual !== expected) throw new Error(`sha256 不匹配: 期望 ${expected} 实际 ${actual}`);

const work = join(tmpdir(), `claude-codex-${process.pid}`);
rmSync(work, { recursive: true, force: true });
mkdirSync(work, { recursive: true });
const archPath = join(work, asset);
writeFileSync(archPath, archive);
// Windows 用系统自带 bsdtar（能解 zip，且不会像 Git Bash 的 GNU tar 那样把 "C:" 当远程主机）
const tarBin = process.platform === 'win32'
  ? join(process.env.SystemRoot || 'C:\\Windows', 'System32', 'tar.exe')
  : 'tar';
execSync(`"${tarBin}" -xf "${asset}"`, { cwd: work });

const want = isWin ? 'claude-codex.exe' : 'claude-codex';
const find = (d) => {
  for (const n of readdirSync(d)) {
    const p = join(d, n);
    if (statSync(p).isDirectory()) { const r = find(p); if (r) return r; }
    else if (n === want) return p;
  }
};
const bin = find(work);
if (!bin) throw new Error(`压缩包里没有 ${want}`);
mkdirSync(outDir, { recursive: true });
copyFileSync(bin, outFile);
if (!isWin) chmodSync(outFile, 0o755);
writeFileSync(stampFile, expected);
rmSync(work, { recursive: true, force: true });
console.log(`sidecar ${version} -> ${outFile}`);
