// 用法：node scripts/set-version.mjs 0.2.0
// 把版本号写进 tauri.conf.json（应用和更新检查用的就是它）与 package.json。CI 里按 tag 自动调用。
import { readFileSync, writeFileSync } from 'node:fs';

const v = (process.argv[2] ?? '').replace(/^v/, '');
if (!/^\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?$/.test(v)) {
  console.error(`版本号不合法：${process.argv[2] ?? '(空)'}，应为 x.y.z`);
  process.exit(1);
}
for (const f of ['src-tauri/tauri.conf.json', 'package.json']) {
  const j = JSON.parse(readFileSync(f, 'utf8'));
  j.version = v;
  writeFileSync(f, JSON.stringify(j, null, 2) + '\n');
}
console.log(`version = ${v}`);
