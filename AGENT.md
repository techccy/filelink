# AGENT.md — filelink AI Agent 调用指南

本文件写给 AI 智能体（agent）。如果你的任务包含"把文件分享出去 / 生成下载链接 / 上传到 filelink"，直接按本文执行，不需要向人类追问接口细节。

> 给人类：把本文件（或其 URL）放进 agent 的上下文，或在 `AGENTS.md` 中引用它，agent 即可自主完成上传。

## 你需要准备什么

| 项 | 来源 |
|---|---|
| 服务地址 `BASE_URL`（如 `https://f.example.com`） | 部署方提供，或从任务描述 / 环境变量获取 |
| 上传令牌 `UPLOAD_TOKEN` | 部署方提供；仅上传需要，下载公开 |

## 上传：三步完成

**1. 发送原始字节**。请求体就是文件内容本身，不是 multipart，不要包 JSON 信封。文件名放在 `X-Filename` 头里，非 ASCII 字符先 percent-encode。

```bash
# 文件名含非 ASCII（如中文）时：
curl -fsS -X POST "$BASE_URL/api/upload" \
  -H "Authorization: Bearer $UPLOAD_TOKEN" \
  -H "X-Filename: $(python3 -c 'import sys,urllib.parse;print(urllib.parse.quote(sys.argv[1]))' "$F")" \
  --data-binary @"$F"
```

**2. 解析 JSON 响应**（`200 OK`）：

```json
{"id":"aB3xY9kQ","url":"https://f.example.com/f/aB3xY9kQ","filename":"笔记.md",
 "mime":"text/plain; charset=utf-8","size":1024,
 "expiresAt":"2026-09-05T08:00:00Z","secret":"8f3c…"}
```

**3. 把 `url` 交给用户**。这个链接打开即文件原始内容，无登录、无下载页——直接把它写进回复、发消息或交给下游程序均可。

## 上传后的规则

- **告知有效期**：回复里带上 `expiresAt`（默认 2 小时），让用户知道链接会过期。
- **保管 `secret`**：这是续期密钥，只在本次响应出现一次。若任务可能需要延长链接，把它连同 `id` 一起记下来；用户不需要它，不要展示在面向用户的文案里。
- **需要延期时**：`POST /f/{id}/renew`，头 `X-Renewal-Secret: <secret>`，每次 +一个有效期。已过期（410）不可复活。
- **需要撤销时**（发错对象、内容有误）：`DELETE /f/{id}`，头 `X-Renewal-Secret: <secret>`。链接立即失效（下载 410），不可恢复。
- **可选校验**：拿不准就 `GET` 一次 `url`，确认 200 且内容完整再交付。

## 出错怎么办

| 状态码 | 原因 | 处理 |
|---|---|---|
| `400` | `X-Filename` 缺失或非法 | 补上文件名；路径成分会被服务端剥离，无需自己净化 |
| `401` | token 缺失或错误 | 检查 `Authorization: Bearer <token>`；token 由人类提供，不要猜测或重试猜测 |
| `413` | 超过大小上限（默认 100MB） | 不要截断文件硬传；向用户报告限制，或改用分卷/其他方式 |
| `404` / `410`（下载或续期时） | 链接不存在 / 已过期 | 重新上传生成新链接 |
| `5xx` | 服务端临时故障 | 可原样重试一次，仍失败则报告 |

错误响应体是**纯文本**一行原因，不是 JSON。

## 注意事项

- **链接即凭证**：任何拿到 `url` 的人都能读文件。不要把链接发给不该看到文件内容的一方；也不要把 `UPLOAD_TOKEN` 或 `secret` 拼进 URL、发到公开频道或写进日志。
- **大小上限**：单文件默认 100MB（Cloudflare 免费版代理上限）。上传前可先看文件大小，超限直接报告，省一次失败请求。
- **AI 可读性**：文本类文件（代码、md、json、csv、log 等）下载时是 `text/plain; charset=utf-8`，抓取链接的一方直接得到可读文本；PDF/图片按原生 MIME；docx/xlsx 等 Office 文档是二进制乱码，需要"AI 可读"时应转成文本或 markdown 再上传。
- **文件名**：服务端按扩展名决定 MIME，所以保留真实扩展名（如 `.md`、`.py`）；别命名为 `file.bin` 之类。
- **临时性**：链接到期即真删，不可恢复。需要长期保存的内容不要只依赖本服务。

## 完整参考

接口全量细节（字段表、错误表、Range 请求、MIME 规则、Python/Node 示例）见 [docs/API.md](docs/API.md)。
