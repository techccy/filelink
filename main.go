package main

import (
	"embed"
	"fmt"
	"io/fs"
	"log"
	"net/http"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"time"
)

//go:embed web
var webFS embed.FS

type Config struct {
	Listen       string
	Token        string
	DataDir      string
	BaseURL      string
	MaxSize      int64
	TTL          time.Duration
	CleanupEvery time.Duration
}

func loadConfig() (Config, error) {
	cfg := Config{
		Listen:       envStr("LISTEN", ":8080"),
		Token:        os.Getenv("UPLOAD_TOKEN"),
		DataDir:      envStr("DATA_DIR", "./data"),
		BaseURL:      strings.TrimRight(envStr("BASE_URL", ""), "/"),
		MaxSize:      int64(envInt("MAX_SIZE_MB", 100)) << 20,
		TTL:          envDur("UPLOAD_TTL", 2*time.Hour),
		CleanupEvery: envDur("CLEANUP_INTERVAL", time.Hour),
	}
	if cfg.Token == "" {
		return cfg, fmt.Errorf("UPLOAD_TOKEN is required")
	}
	if cfg.MaxSize <= 0 {
		return cfg, fmt.Errorf("MAX_SIZE_MB must be positive")
	}
	if cfg.TTL <= 0 {
		return cfg, fmt.Errorf("UPLOAD_TTL must be positive")
	}
	return cfg, nil
}

func envStr(key, def string) string {
	if v := strings.TrimSpace(os.Getenv(key)); v != "" {
		return v
	}
	return def
}

func envInt(key string, def int) int {
	if v := os.Getenv(key); v != "" {
		if n, err := strconv.Atoi(v); err == nil && n > 0 {
			return n
		}
		log.Printf("invalid %s=%q, using default %d", key, v, def)
	}
	return def
}

func envDur(key string, def time.Duration) time.Duration {
	if v := os.Getenv(key); v != "" {
		if d, err := time.ParseDuration(v); err == nil && d > 0 {
			return d
		}
		log.Printf("invalid %s=%q, using default %s", key, v, def)
	}
	return def
}

func main() {
	log.SetFlags(log.LstdFlags | log.LUTC)

	cfg, err := loadConfig()
	if err != nil {
		log.Fatalf("config: %v", err)
	}

	if err := os.MkdirAll(filepath.Join(cfg.DataDir, "files"), 0o755); err != nil {
		log.Fatalf("create data dir: %v", err)
	}
	if err := os.MkdirAll(filepath.Join(cfg.DataDir, "tmp"), 0o755); err != nil {
		log.Fatalf("create tmp dir: %v", err)
	}

	store, err := OpenStore(filepath.Join(cfg.DataDir, "filelink.db"))
	if err != nil {
		log.Fatalf("open store: %v", err)
	}

	srv := NewServer(cfg, store)

	sub, err := fs.Sub(webFS, "web")
	if err != nil {
		log.Fatalf("embed web: %v", err)
	}

	go runJanitor(cfg, store)

	log.Printf("filelink listening on %s (max %dMB, ttl %s, cleanup every %s)",
		cfg.Listen, cfg.MaxSize>>20, cfg.TTL, cfg.CleanupEvery)
	if err := http.ListenAndServe(cfg.Listen, srv.Mux(sub)); err != nil {
		log.Fatalf("server: %v", err)
	}
}

// runJanitor deletes expired links (true delete: row + file), then sweeps
// orphan files left behind by crashed uploads.
func runJanitor(cfg Config, store *Store) {
	janitorOnce(cfg, store)
	for range time.Tick(cfg.CleanupEvery) {
		janitorOnce(cfg, store)
	}
}

func janitorOnce(cfg Config, store *Store) {
	ids, err := store.DeleteExpired(time.Now())
	if err != nil {
		log.Printf("janitor: delete expired: %v", err)
	}
	for _, id := range ids {
		if err := os.Remove(filepath.Join(cfg.DataDir, "files", id)); err != nil && !os.IsNotExist(err) {
			log.Printf("janitor: remove file %s: %v", id, err)
			continue
		}
		log.Printf("janitor: expired, deleted %s", id)
	}
	sweepOrphans(cfg, store)
}

func sweepOrphans(cfg Config, store *Store) {
	grace := 2 * cfg.CleanupEvery
	entries, err := os.ReadDir(filepath.Join(cfg.DataDir, "files"))
	if err != nil {
		return
	}
	for _, e := range entries {
		info, err := e.Info()
		if err != nil || info.IsDir() || time.Since(info.ModTime()) < grace {
			continue
		}
		id := e.Name()
		ok, err := store.HasID(id)
		if err == nil && !ok {
			if err := os.Remove(filepath.Join(cfg.DataDir, "files", id)); err == nil {
				log.Printf("janitor: removed orphan file %s", id)
			}
		}
	}
	tmps, err := os.ReadDir(filepath.Join(cfg.DataDir, "tmp"))
	if err != nil {
		return
	}
	for _, e := range tmps {
		info, err := e.Info()
		if err == nil && !info.IsDir() && time.Since(info.ModTime()) >= grace {
			os.Remove(filepath.Join(cfg.DataDir, "tmp", e.Name()))
		}
	}
}
