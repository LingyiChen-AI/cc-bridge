// 用本仓库 proxy/ 里自行维护的代理源码编译 sidecar，放到 src-tauri/binaries/claude-codex-<triple>[.exe]
// （不再从上游 GitHub Release 下载）
import { execSync } from 'node:child_process';
import { copyFileSync, mkdirSync, existsSync, statSync, readdirSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const proxyDir = join(root, 'proxy');
const triple = process.env.TAURI_TARGET_TRIPLE || execSync('rustc -vV').toString().match(/host: (\S+)/)[1];
const isWin = triple.includes('windows');
const exe = isWin ? 'claude-codex.exe' : 'claude-codex';
const built = join(proxyDir, 'target', triple, 'release', exe);
const out = join(root, 'src-tauri', 'binaries', `claude-codex-${triple}${isWin ? '.exe' : ''}`);

// 源码比产物新才重新编译
const newest = (d) => readdirSync(d, { withFileTypes: true }).reduce((m, e) => {
  const p = join(d, e.name);
  return Math.max(m, e.isDirectory() ? newest(p) : statSync(p).mtimeMs);
}, 0);
const srcTime = Math.max(newest(join(proxyDir, 'src')), statSync(join(proxyDir, 'Cargo.toml')).mtimeMs, statSync(join(proxyDir, 'Cargo.lock')).mtimeMs);
if (existsSync(out) && statSync(out).mtimeMs >= srcTime) {
  console.log('sidecar 已是最新，跳过编译');
  process.exit(0);
}

execSync(`cargo build --release --target ${triple}`, { cwd: proxyDir, stdio: 'inherit' });
mkdirSync(dirname(out), { recursive: true });
copyFileSync(built, out);
console.log(`sidecar -> ${out}`);
