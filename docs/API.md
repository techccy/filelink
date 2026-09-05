# filelink API 文档

filelink 的全部 HTTP 接口。所有路径均相对于服务地址（下文记作 `BASE_URL`，如 `https://f.example.com`）。

面向 AI agent 的精简调用指南见 [AGENT.md](../AGENT.md)。

## 通用约定

- **鉴权**：仅上传需要 `Authorization: Bearer <UPLOAD_TOKEN>`；下载完全公开；续期使用上传时返回的一次性续期密钥。
- **错误格式**：非 2xx 响应体为纯文本（`text/plain`），一行错误描述，末尾带换行。不是 JSON。
- **时间格式**：所有时间字段均为 UTC 的 RFC 3339 字符串，如 `2026-09-05T08:00:00Z`。
- **上限与有效期**由部署配置决定：单文件上限 `MAX_SIZE_MB`（默认 100MB），链接有效期 `UPLOAD_TTL`（默认 2h）。不确定时以上传响应中的 `expiresAt` 为准。

---

## POST /api/upload — 上传文件

### 请求

| 项 | 说明 |
|---|---|
| 方法 / 路径 | `POST /api/upload` |
| `Authorization` | 必填，`Bearer <UPLOAD_TOKEN>` |
| `X-Filename` | 必填，文件名。非 ASCII 字符须先 percent-encode（即 JS 的 `encodeURIComponent` / Go 的 `url.PathEscape`）；纯 ASCII 文件名可直接写 |
| 请求体 | **文件原始字节**。不是 multipart/form-data，不要包任何信封 |
| `Content-Type`（请求的） | 服务端忽略，实际 MIME 由文件扩展名决定 |

服务端流式写盘，内存占用与文件大小无关；请求体超过上限时返回 413。

### 响应 `200 OK`

```json
{
  "id": "aB3xY9kQ",
  "url": "https://f.example.com/f/aB3xY9kQ",
  "filename": "笔记.md",
  "mime": "text/plain; charset=utf-8",
  "size": 1024,
  "expiresAt": "2026-09-05T08:00:00Z",
  "secret": "8f3c…（32 个十六进制字符）"
}
```

| 字段 | 说明 |
|---|---|
| `id` | 8 位字母数字链接 ID |
| `url` | 公链绝对地址，直接 GET 即得文件内容 |
| `filename` | 净化后的文件名（路径成分被剥离，超 255 字符截断） |
| `mime` | 下载时服务端返回的 `Content-Type` |
| `size` | 字节数 |
| `expiresAt` | 到期时间，到期后链接失效（410） |
| `secret` | 续期密钥，**仅此一次**返回，服务端只存 SHA-256，丢失即无法续期 |

### 错误

| 状态码 | 含义 |
|---|---|
| `400` | 缺少或非法的 `X-Filename`（如编码后为空、`.`、`..`） |
| `401` | 缺少 / 格式错误 / 错误的 Bearer token |
| `413` | 文件超过 `MAX_SIZE_MB` |
| `500` | 服务端写盘或存储失败 |

### 示例

curl（文件名含非 ASCII，先编码）：

```bash
curl -fsS -X POST "$BASE_URL/api/upload" \
  -H "Authorization: Bearer $UPLOAD_TOKEN" \
  -H "X-Filename: $(python3 -c 'import sys,urllib.parse;print(urllib.parse.quote(sys.argv[1]))' "$F")" \
  --data-binary @"$F"
```

文件名为纯 ASCII 时可省去编码一步：`-H "X-Filename: report.pdf"`。

Python：

```python
import requests, urllib.parse

def upload(path: str) -> dict:
    with open(path, "rb") as f:
        r = requests.post(
            f"{BASE_URL}/api/upload",
            headers={
                "Authorization": f"Bearer {UPLOAD_TOKEN}",
                "X-Filename": urllib.parse.quote(path.split("/")[-1]),
            },
            data=f,
        )
    r.raise_for_status()
    return r.json()
```

Node.js：

```js
async function upload(pathname, bytes) {
  const res = await fetch(`${BASE_URL}/api/upload`, {
    method: "POST",
    headers: {
      Authorization: `Bearer ${UPLOAD_TOKEN}`,
      "X-Filename": encodeURIComponent(pathname.split("/").pop()),
    },
    body: bytes, // 原始字节，不要用 FormData
  });
  if (!res.ok) throw new Error(`upload ${res.status}: ${await res.text()}`);
  return res.json();
}
```

---

## GET /f/{id} — 下载文件

无需鉴权。**链接本身就是凭证**：任何拿到 URL 的人都能读取文件内容。

### 响应 `200 OK`

响应体即文件原始内容，带：

- `Content-Type`：按扩展名判定（见下方 MIME 规则）
- `Content-Disposition: inline; filename=...`（非 ASCII 文件名走 RFC 5987 `filename*=utf-8''...`）
- `X-Content-Type-Options: nosniff`、`Cache-Control: no-store`
- 支持 HTTP Range / 条件请求（标准 `http.ServeContent` 行为）

### 错误

| 状态码 | 含义 |
|---|---|
| `404` | 链接不存在（或已被过期清理删除） |
| `410` | 链接已过期，内容不可再取 |

```bash
curl -fsS "$BASE_URL/f/aB3xY9kQ"
```

---

## POST /f/{id}/renew — 续期

把链接有效期延长一个 `UPLOAD_TTL`（默认 +2h），不限次数，但**已过期的链接不可复活**。

### 请求

| 项 | 说明 |
|---|---|
| 方法 / 路径 | `POST /f/{id}/renew` |
| `X-Renewal-Secret` | 必填，上传响应中返回的 `secret` |
| 请求体 | 无 |

### 响应 `200 OK`

```json
{"id":"aB3xY9kQ","url":"https://f.example.com/f/aB3xY9kQ","expiresAt":"2026-09-05T10:00:00Z"}
```

### 错误

| 状态码 | 含义 |
|---|---|
| `400` | 缺少 `X-Renewal-Secret` 请求头 |
| `403` | 续期密钥错误 |
| `404` | 链接不存在 |
| `410` | 链接已过期，不可续期 |

```bash
curl -fsS -X POST "$BASE_URL/f/aB3xY9kQ/renew" \
  -H "X-Renewal-Secret: $SECRET"
```

---

## MIME 规则

下载时的 `Content-Type` 完全由上传时的文件名（扩展名）决定，请求体的 `Content-Type` 不参与判定：

- **文本类**（`.txt` `.md` `.json` `.csv` `.log` `.go` `.py` `.yaml` `.html` `.svg` 等约 100 种，含 `Makefile`、`.gitignore` 等无扩展名文本文件）→ `text/plain; charset=utf-8`：浏览器内联显示，AI 抓取后即为可读文本，且不可能在本域名执行脚本。
- **PDF / 图片 / 音视频**等 → 按标准 MIME（如 `application/pdf`、`image/png`）。
- 其余（含 Office 文档 docx/xlsx 等）→ `application/octet-stream`：对 AI 是二进制乱码，不支持"直接可读"。

## 过期与删除

- 到期瞬间链接即失效（下载 410），磁盘文件由后台任务在下一个清理周期（`CLEANUP_INTERVAL`，默认 1h）真删，不可恢复。
- 上传中途崩溃留下的临时/无主文件，超过两个清理周期后自动清扫。
