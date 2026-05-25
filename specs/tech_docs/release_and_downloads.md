# Cameo — 发布与下载机制

> **状态：已实现（脚本就绪，待首个 release 实跑验证）。** 描述 Cameo 客户端怎么发布，
> 以及官网 `cameo_web` 的下载按钮怎么拿到 mac/win 的安装链接。
> 官网侧的同一份契约见 `../../../cameo_web/specs/DOWNLOADS.md`（两边保持一致）。

---

## 1. 两条独立通道（别混为一谈）

发布产出走**两条互不相干**的通道，各有各的 host、生产者、消费者：

| 通道 | Host | 生产者 | 消费者 | 内容 |
|---|---|---|---|---|
| **自动更新 (auto-updater)** | **R2 `r.cameo.ink`** | `publish_release.{sh,ps1}`（rclone → R2）| app 内的 **Tauri updater**（`src-tauri/tauri.conf.json` → `updater.endpoints`）| `.app.tar.gz` / `.nsis.zip` + `.sig` + `{{target}}-{{arch}}.json` 签名 manifest |
| **网站下载 + 开源分发** | **GitHub Releases**（`github.com/hAcKlyc/cameo/releases`）| `publish_release.{sh,ps1}`（`gh release create/upload`）| **cameo_web** 下载按钮 + 在 GitHub 浏览的人 | `.dmg` / `-setup.exe` 安装包 + `latest.json` / `latest_win.json` |

要点：**官网不读 R2**。它读 GitHub Releases——对开源 app 来说这是最自然的下载位（免费、
也是人肉下载处）。自动更新和网站下载是两件事，别让某一方依赖另一方。

---

## 2. 客户端发布流程（生产者）

`build_release.{sh,ps1}` 先用 Tauri 按各 target 构建，然后 `publish_release.{sh,ps1}`：

1. 从 `src-tauri/tauri.conf.json` 读 `version`。
2. **自动更新 → R2**（原有逻辑）：对 `.app.tar.gz`/`.nsis.zip` 用 `TAURI_SIGNING_PRIVATE_KEY`
   签名，写 `darwin-aarch64.json` / `darwin-x86_64.json` / `windows-x86_64.json`，
   rclone 上传 payload + manifest 到 `r2:$R2_BUCKET/{release,update}/…`（对外即 `r.cameo.ink`）。
3. **网站 + GitHub**（为开源下载通道新增）：
   - 生成 `latest.json`（mac）/ `latest_win.json`（win），其中 `downloads[*].url` 指向
     **GitHub release 资产**的下载地址；
   - `git tag -a v<version>` 并 push（幂等）；
   - 若不存在则 `gh release create v<version>`；
   - `gh release upload v<version> <安装包> latest*.json --clobber`。
   - mac 与 win **从各自的机器**发布到**同一个** tag——谁先跑谁建 release，另一个 `--clobber` 上传。

前置条件：`gh` 已登录且有 push/打 tag 权限；`.env` 含 `TAURI_SIGNING_PRIVATE_KEY`(+password)
（更新签名）与 R2 凭据（更新通道）。发版前记得 bump `tauri.conf.json` 的 `version`。

### 资产命名（Tauri 默认）
- macOS：`Cameo_<version>_aarch64.dmg`、`Cameo_<version>_x64.dmg`（`.app.tar.gz` 上传时加 `_<arch>` 后缀避免覆盖）
- Windows：`Cameo_<version>_x64-setup.exe`（NSIS）

---

## 3. 网站消费流程（cameo_web）

1. `detectPlatform()` 测 OS + 架构 → `mac_arm64` / `mac_intel` / `windows`（Apple Silicon
   靠 WebGL renderer 嗅探，默认按 AS）。
2. `useDownloads()` **同源**并行拉 `/update/latest.json` + `/update/latest_win.json`
   （`Promise.allSettled`，缺一个就少一个平台、不报错）。
3. cameo_web 的 worker `GET /update/:file`（白名单 = 这两个文件）**代理**
   `https://github.com/hAcKlyc/cameo/releases/latest/download/<file>`（服务端 fetch →
   无 CORS、边缘缓存 5 分钟）。
4. 主按钮 = 当前平台对应的 URL；下拉列出 manifest 里所有可用 build。
5. **回退**：还没 release（manifest 404）→ 按钮指向 GitHub **releases 页**，不隐藏。

### Manifest 形状（`latest.json` = mac，`latest_win.json` = win）
```json
{
  "version": "0.1.0",
  "pub_date": "2026-05-25T…Z",
  "release_notes": "Cameo v0.1.0",
  "downloads": {
    "mac_arm64": { "name": "Apple Silicon", "url": "https://github.com/hAcKlyc/cameo/releases/download/v0.1.0/Cameo_0.1.0_aarch64.dmg" },
    "mac_intel": { "name": "Intel Mac",     "url": ".../Cameo_0.1.0_x64.dmg" },
    "win_x64":   { "name": "Windows x64",   "url": ".../Cameo_0.1.0_x64-setup.exe" }
  }
}
```
mac/win 拆成两份文件，是因为它们从不同机器、异步发布到同一个 GitHub release。

---

## 4. Tauri 自动更新（与网站下载无关）

`src-tauri/tauri.conf.json`：
- `updater.pubkey` = minisign 公钥；
- `updater.endpoints` = `https://r.cameo.ink/update/{{target}}-{{arch}}.json`。

Tauri 把 `{{target}}`/`{{arch}}` 替换成 `darwin`/`windows` + `aarch64`/`x86_64`，命中
`publish_release.*` 写到 R2 的签名 manifest（`darwin-aarch64.json` 等）。客户端据 `signature`
校验后增量更新，下载的是 `.app.tar.gz`/`.nsis.zip`（**不是** dmg/exe）。

---

## 5. 版本流（merge_release → tag → release）

1. 开发完成、合并 release 分支后 bump `tauri.conf.json` 的 `version`。
2. 在 mac 上 `./build_release.sh` → `./publish_release.sh`；在 Windows 上
   `build_release.ps1` → `publish_release.ps1`。
3. publish 脚本自动 `git tag v<version>` + `gh release create/upload`——tag 与 GitHub
   release 由 publish 脚本负责，不需要手动建。
4. release 成为 GitHub 的 "latest" 后，`releases/latest/download/latest.json` 生效，
   cameo_web 的 `/update` 代理随即返回真实链接，下载按钮自动点亮。

---

## 6. 现状 / 待办

- **尚无任何 release** → `/update/*` 返回 `404 {"error":"no_release"}`，官网按钮暂回退到
  GitHub releases 页；跑出首个 `v*` release（带 `latest.json`）后自动点亮。
- publish 脚本的 GitHub/manifest 段为照搬 MyAgents 适配而来，**首个 release 会实跑验证**。
- `r.cameo.ink` 的 R2 自定义域名需为自动更新通道配置好（与网站下载无关）。
