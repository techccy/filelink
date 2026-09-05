# filelink

自托管的临时文件分享服务：拖拽上传 → 生成公链（默认 2 小时）→ 凭链接直接读取文件内容。链接是给 **AI / 程序** 用的——打开即原始内容，没有下载页、无需任何交互。

## 功能

- 拖拽（或粘贴）上传，多文件逐个上传，每个文件一条独立公链
- 公链严格 raw 直出：正确 `Content-Type` + `Content-Disposition: inline`，文本类给 `text/plain`，PDF / 图片按原生类型
- 上传令牌鉴权（仅上传需要，下载公开）
- 链接默认 2 小时有效，上传者凭续期密钥一键 +2h，不限次数，也可随时撤销
- 到期真删（记录 + 磁盘文件），不可恢复；每小时清理，附带孤儿文件清扫
- 上传历史保存在浏览器 localStorage，换设备不迁移

## 桌面客户端

[desktop/](desktop/) 提供 macOS 菜单栏客户端（Tauri 2）：文件拖到状态栏图标即上传并自动复制链接，面板内管理历史、倒计时、续期与撤销。构建与说明见 [desktop/README.md](desktop/README.md)。

## 快速开始

### 本地运行

```bash
export UPLOAD_TOKEN=你的令牌
go run .
# 打开 http://127.0.0.1:8080
```

### Docker + Cloudflare Tunnel（推荐部署方式）

```bash
cp .env.example .env       # 填入 UPLOAD_TOKEN 和 TUNNEL_TOKEN
docker compose up -d --build
```

在 Cloudflare Zero Trust 控制台给 Tunnel 配置公共主机名，指向 `http://filelink:8080`。

## 配置（环境变量）

| 变量 | 默认 | 说明 |
|---|---|---|
| `UPLOAD_TOKEN` | 必填 | 上传令牌（Bearer） |
| `DATA_DIR` | `./data` | 数据目录（SQLite + 文件） |
| `BASE_URL` | 自动推导 | 公链前缀，如 `https://f.example.com`；留空则从请求推导 |
| `MAX_SIZE_MB` | `100` | 单文件上限。**Cloudflare 免费版代理请求体上限 100MB**，调大需绕开 CF 代理 |
| `UPLOAD_TTL` | `2h` | 默认有效期，续期一次延长同时长 |
| `CLEANUP_INTERVAL` | `1h` | 过期清理间隔 |
| `LISTEN` | `:8080` | 监听地址 |

## API

完整接口文档见 [docs/API.md](docs/API.md)；面向 AI agent 的调用指南见 [AGENT.md](AGENT.md)（把它放进 agent 上下文即可让其自主上传）。

### 上传

```bash
curl -X POST https://f.example.com/api/upload \
  -H "Authorization: Bearer $UPLOAD_TOKEN" \
  -H "X-Filename: $(python3 -c 'import urllib.parse;print(urllib.parse.quote("笔记.md"))')" \
  --data-binary @笔记.md
```

```json
{"id":"aB3xY9kQ","url":"https://f.example.com/f/aB3xY9kQ","filename":"笔记.md",
 "mime":"text/plain; charset=utf-8","size":1024,
 "expiresAt":"2026-09-05T08:00:00Z","secret":"8f3c…（续期密钥，仅客户端保存）"}
```

请求体即文件原始字节（非 multipart），服务端流式写盘，内存占用与文件大小无关。`X-Filename` 为 `encodeURIComponent` 编码后的文件名。

### 下载

```bash
curl https://f.example.com/f/aB3xY9kQ
```

直接返回文件内容。404 = 不存在，410 = 已过期。

### 续期

```bash
curl -X POST https://f.example.com/f/aB3xY9kQ/renew \
  -H "X-Renewal-Secret: 8f3c…"
```

返回新的 `expiresAt`（+`UPLOAD_TTL`）。过期后不可复活（410）。

### 撤销

```bash
curl -X DELETE https://f.example.com/f/aB3xY9kQ \
  -H "X-Renewal-Secret: 8f3c…"
```

上传者凭续期密钥随时撤销（提前过期）：下载立即 410，磁盘文件下个清理周期真删，不可恢复。

## 行为细节

- **文件类型**：文本类（代码 / md / json / csv / log 等，含 `.html`、`.svg`）一律 `text/plain; charset=utf-8`——浏览器内联显示、AI 直接可读、且绝不可能在本域名执行脚本；PDF / 图片 / 音视频按标准 MIME；其余 `application/octet-stream`。Office 文档（docx 等）对 AI 是二进制乱码，不支持"直接可读"。
- **安全**：上传需 Bearer token；续期密钥只存 SHA-256，不随公链暴露；下载无鉴权——**任何拿到链接的人都能读，链接本身就是凭证**。
- **过期语义**：过期瞬间链接即失效（410），磁盘文件由清理任务在下一个周期删除；已过期不可续期。
- **孤儿清扫**：上传中途崩溃留下的临时/无主文件，超过两个清理周期后自动删除。

## 开发

```bash
go test ./...   # 单元测试覆盖上传/下载/续期/过期/清理/文件名净化
```
