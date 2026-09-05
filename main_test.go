package main

import (
	"bytes"
	"encoding/json"
	"io"
	"net/http"
	"net/http/httptest"
	"net/url"
	"os"
	"path/filepath"
	"testing"
	"testing/fstest"
	"time"
)

type testEnv struct {
	t       *testing.T
	ts      *httptest.Server
	store   *Store
	dataDir string
}

func newTestEnv(t *testing.T, maxSize int64, ttl time.Duration) *testEnv {
	t.Helper()
	dataDir := t.TempDir()
	for _, sub := range []string{"files", "tmp"} {
		if err := os.MkdirAll(filepath.Join(dataDir, sub), 0o755); err != nil {
			t.Fatal(err)
		}
	}
	store, err := OpenStore(filepath.Join(dataDir, "test.db"))
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { store.Close() })

	cfg := Config{
		Token:        "test-token",
		DataDir:      dataDir,
		MaxSize:      maxSize,
		TTL:          ttl,
		CleanupEvery: time.Hour,
	}
	ts := httptest.NewServer(NewServer(cfg, store).Mux(fstest.MapFS{}))
	t.Cleanup(ts.Close)
	return &testEnv{t: t, ts: ts, store: store, dataDir: dataDir}
}

func (e *testEnv) upload(name string, body []byte, token string) (*http.Response, map[string]any) {
	e.t.Helper()
	req, err := http.NewRequest("POST", e.ts.URL+"/api/upload", bytes.NewReader(body))
	if err != nil {
		e.t.Fatal(err)
	}
	req.Header.Set("Authorization", "Bearer "+token)
	req.Header.Set("X-Filename", url.PathEscape(name))
	resp, err := e.ts.Client().Do(req)
	if err != nil {
		e.t.Fatal(err)
	}
	defer resp.Body.Close()
	var data map[string]any
	json.NewDecoder(resp.Body).Decode(&data)
	return resp, data
}

func (e *testEnv) get(url string) *http.Response {
	e.t.Helper()
	resp, err := e.ts.Client().Get(url)
	if err != nil {
		e.t.Fatal(err)
	}
	return resp
}

func (e *testEnv) revoke(id, secret string) *http.Response {
	e.t.Helper()
	req, err := http.NewRequest("DELETE", e.ts.URL+"/f/"+id, nil)
	if err != nil {
		e.t.Fatal(err)
	}
	req.Header.Set("X-Renewal-Secret", secret)
	resp, err := e.ts.Client().Do(req)
	if err != nil {
		e.t.Fatal(err)
	}
	return resp
}

func TestUploadDownloadRoundTrip(t *testing.T) {
	e := newTestEnv(t, 1<<20, time.Hour)

	resp, data := e.upload("笔记.txt", []byte("hello 世界"), "test-token")
	if resp.StatusCode != http.StatusOK {
		t.Fatalf("upload status = %d, want 200", resp.StatusCode)
	}
	url, _ := data["url"].(string)
	if url == "" {
		t.Fatal("upload response missing url")
	}
	if data["secret"].(string) == "" {
		t.Fatal("upload response missing secret")
	}

	dl := e.get(url)
	defer dl.Body.Close()
	if dl.StatusCode != http.StatusOK {
		t.Fatalf("download status = %d, want 200", dl.StatusCode)
	}
	body, _ := io.ReadAll(dl.Body)
	if string(body) != "hello 世界" {
		t.Fatalf("download body = %q", body)
	}
	if ct := dl.Header.Get("Content-Type"); ct != "text/plain; charset=utf-8" {
		t.Fatalf("Content-Type = %q", ct)
	}
	if cd := dl.Header.Get("Content-Disposition"); cd == "" || cd[:6] != "inline" {
		t.Fatalf("Content-Disposition = %q, want inline", cd)
	}
}

func TestUploadRequiresToken(t *testing.T) {
	e := newTestEnv(t, 1<<20, time.Hour)

	if resp, _ := e.upload("a.txt", []byte("x"), "wrong"); resp.StatusCode != http.StatusUnauthorized {
		t.Fatalf("wrong token status = %d, want 401", resp.StatusCode)
	}
}

func TestUploadTooLarge(t *testing.T) {
	e := newTestEnv(t, 1024, time.Hour)

	resp, _ := e.upload("big.txt", make([]byte, 2048), "test-token")
	if resp.StatusCode != http.StatusRequestEntityTooLarge {
		t.Fatalf("status = %d, want 413", resp.StatusCode)
	}
}

func TestRenew(t *testing.T) {
	e := newTestEnv(t, 1<<20, time.Hour)

	_, data := e.upload("a.txt", []byte("x"), "test-token")
	id := data["id"].(string)
	oldExpiry := data["expiresAt"].(string)
	secret := data["secret"].(string)

	// Expiry has 1s resolution; wait past the boundary so the renewed expiry
	// is strictly later than the original one.
	time.Sleep(1100 * time.Millisecond)

	renewReq, _ := http.NewRequest("POST", e.ts.URL+"/f/"+id+"/renew", nil)
	renewReq.Header.Set("X-Renewal-Secret", "wrong-secret")
	resp, err := e.ts.Client().Do(renewReq)
	if err != nil {
		t.Fatal(err)
	}
	resp.Body.Close()
	if resp.StatusCode != http.StatusForbidden {
		t.Fatalf("wrong secret status = %d, want 403", resp.StatusCode)
	}

	renewReq2, _ := http.NewRequest("POST", e.ts.URL+"/f/"+id+"/renew", nil)
	renewReq2.Header.Set("X-Renewal-Secret", secret)
	resp2, err := e.ts.Client().Do(renewReq2)
	if err != nil {
		t.Fatal(err)
	}
	defer resp2.Body.Close()
	if resp2.StatusCode != http.StatusOK {
		t.Fatalf("renew status = %d, want 200", resp2.StatusCode)
	}
	var out map[string]any
	json.NewDecoder(resp2.Body).Decode(&out)
	newExpiry := out["expiresAt"].(string)
	if !mustTime(t, newExpiry).After(mustTime(t, oldExpiry)) {
		t.Fatalf("new expiry %s not after old %s", newExpiry, oldExpiry)
	}
}

func TestExpiredLinkGone(t *testing.T) {
	e := newTestEnv(t, 1<<20, -time.Second) // already expired at creation

	_, data := e.upload("a.txt", []byte("x"), "test-token")
	url := data["url"].(string)
	id := data["id"].(string)

	dl := e.get(url)
	dl.Body.Close()
	if dl.StatusCode != http.StatusGone {
		t.Fatalf("download expired status = %d, want 410", dl.StatusCode)
	}

	renewReq, _ := http.NewRequest("POST", e.ts.URL+"/f/"+id+"/renew", nil)
	renewReq.Header.Set("X-Renewal-Secret", data["secret"].(string))
	resp, err := e.ts.Client().Do(renewReq)
	if err != nil {
		t.Fatal(err)
	}
	resp.Body.Close()
	if resp.StatusCode != http.StatusGone {
		t.Fatalf("renew expired status = %d, want 410", resp.StatusCode)
	}
}

func TestNotFound(t *testing.T) {
	e := newTestEnv(t, 1<<20, time.Hour)

	dl := e.get(e.ts.URL + "/f/zzzzzzzz")
	dl.Body.Close()
	if dl.StatusCode != http.StatusNotFound {
		t.Fatalf("status = %d, want 404", dl.StatusCode)
	}
}

func TestFilenameSanitize(t *testing.T) {
	e := newTestEnv(t, 1<<20, time.Hour)

	resp, data := e.upload("../../etc/evil.txt", []byte("x"), "test-token")
	if resp.StatusCode != http.StatusOK {
		t.Fatalf("status = %d", resp.StatusCode)
	}
	if data["filename"] != "evil.txt" {
		t.Fatalf("filename = %q, want evil.txt", data["filename"])
	}

	if resp2, _ := e.upload("", []byte("x"), "test-token"); resp2.StatusCode != http.StatusBadRequest {
		t.Fatalf("missing filename status = %d, want 400", resp2.StatusCode)
	}
}

func TestBinaryMIME(t *testing.T) {
	e := newTestEnv(t, 1<<20, time.Hour)

	resp, data := e.upload("archive.bin", []byte{0x00, 0x01, 0x02}, "test-token")
	if resp.StatusCode != http.StatusOK {
		t.Fatalf("status = %d", resp.StatusCode)
	}
	if data["mime"] != "application/octet-stream" {
		t.Fatalf("mime = %q", data["mime"])
	}
	dl := e.get(data["url"].(string))
	defer dl.Body.Close()
	if ct := dl.Header.Get("Content-Type"); ct != "application/octet-stream" {
		t.Fatalf("Content-Type = %q", ct)
	}
}

func TestJanitorDeletesExpired(t *testing.T) {
	e := newTestEnv(t, 1<<20, 10*time.Millisecond)

	_, data := e.upload("a.txt", []byte("x"), "test-token")
	id := data["id"].(string)
	time.Sleep(50 * time.Millisecond)

	cfg := Config{DataDir: e.dataDir}
	janitorOnce(cfg, e.store)

	if _, err := e.store.Get(id); err != ErrNotFound {
		t.Fatalf("Get after janitor = %v, want ErrNotFound", err)
	}
	if fileExists(filepath.Join(e.dataDir, "files", id)) {
		t.Fatal("expired file still on disk")
	}
}

func TestRevoke(t *testing.T) {
	e := newTestEnv(t, 1<<20, time.Hour)

	_, data := e.upload("a.txt", []byte("x"), "test-token")
	id := data["id"].(string)
	url := data["url"].(string)
	secret := data["secret"].(string)

	respNoSecret := e.revoke(id, "")
	respNoSecret.Body.Close()
	if respNoSecret.StatusCode != http.StatusBadRequest {
		t.Fatalf("missing secret status = %d, want 400", respNoSecret.StatusCode)
	}
	if resp := e.revoke(id, "wrong-secret"); resp.StatusCode != http.StatusForbidden {
		resp.Body.Close()
		t.Fatalf("wrong secret status = %d, want 403", resp.StatusCode)
	}
	if resp := e.revoke("zzzzzzzz", secret); resp.StatusCode != http.StatusNotFound {
		resp.Body.Close()
		t.Fatalf("unknown id status = %d, want 404", resp.StatusCode)
	}

	resp := e.revoke(id, secret)
	defer resp.Body.Close()
	if resp.StatusCode != http.StatusOK {
		t.Fatalf("revoke status = %d, want 200", resp.StatusCode)
	}

	// A revoked link behaves exactly like an expired one: download 410,
	// renew 410, and re-revoke 410 — until the janitor removes the row.
	dl := e.get(url)
	dl.Body.Close()
	if dl.StatusCode != http.StatusGone {
		t.Fatalf("download revoked status = %d, want 410", dl.StatusCode)
	}
	renewReq, _ := http.NewRequest("POST", e.ts.URL+"/f/"+id+"/renew", nil)
	renewReq.Header.Set("X-Renewal-Secret", secret)
	rr, err := e.ts.Client().Do(renewReq)
	if err != nil {
		t.Fatal(err)
	}
	rr.Body.Close()
	if rr.StatusCode != http.StatusGone {
		t.Fatalf("renew revoked status = %d, want 410", rr.StatusCode)
	}
	if resp2 := e.revoke(id, secret); resp2.StatusCode != http.StatusGone {
		t.Fatalf("re-revoke status = %d, want 410", resp2.StatusCode)
	}
}

func TestRevokeJanitorDeletesFile(t *testing.T) {
	e := newTestEnv(t, 1<<20, time.Hour)

	_, data := e.upload("a.txt", []byte("x"), "test-token")
	id := data["id"].(string)

	if resp := e.revoke(id, data["secret"].(string)); resp.StatusCode != http.StatusOK {
		t.Fatalf("revoke status = %d, want 200", resp.StatusCode)
	}

	janitorOnce(Config{DataDir: e.dataDir}, e.store)

	if _, err := e.store.Get(id); err != ErrNotFound {
		t.Fatalf("Get after janitor = %v, want ErrNotFound", err)
	}
	if fileExists(filepath.Join(e.dataDir, "files", id)) {
		t.Fatal("revoked file still on disk")
	}
}

func mustTime(t *testing.T, s string) time.Time {
	t.Helper()
	ts, err := time.Parse(time.RFC3339, s)
	if err != nil {
		t.Fatal(err)
	}
	return ts
}

func fileExists(path string) bool {
	_, err := os.Stat(path)
	return err == nil
}
