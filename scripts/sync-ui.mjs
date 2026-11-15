// 将工作空间共享前端层 shared/ui 同步到各 app 的 src/shared，
// 保证零构建 Tauri 前端（frontendDist=../src）能引用到公共资产。
// 用法：node scripts/sync-ui.mjs （打包脚本会自动调用）。
import { copyFileSync, mkdirSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = dirname(dirname(fileURLToPath(import.meta.url)));
const src = join(root, 'shared', 'ui');
const apps = ['push-wp', 'ddl-desktop', 'launch-manager'];

for (const app of apps) {
  const dest = join(root, app, 'src', 'shared');
  mkdirSync(dest, { recursive: true });
  for (const file of ['base.css', 'base.js']) {
    copyFileSync(join(src, file), join(dest, file));
  }
  console.log(`synced shared/ui -> ${app}/src/shared`);
}
