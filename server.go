package main

import (
	"crypto/rand"
	"crypto/sha256"
	"crypto/subtle"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"io/fs"
	"log"
	"mime"
	"net/http"
	"net/url"
	"os"
	"path"
	"path/filepath"
	"strings"
	"time"
)

const (
	idLen        = 8
	secretBytes  = 16 // -> 32 hex chars
	maxNameRunes = 255
)

type Server struct {
	cfg   Config
	store *Store
}

func NewServer(cfg Config, store *Store) *Server {
	return &Server{cfg: cfg, store: store}
}

func (s *Server) Mux(webRoot fs.FS) *http.ServeMux {
	mux := http.NewServeMux()
	mux.Handle("GET /", http.FileServerFS(webRoot))
	mux.HandleFunc("POST /api/upload", s.handleUpload)
	mux.HandleFunc("GET /f/{id}", s.handleDownload)
	mux.HandleFunc("POST /f/{id}/renew", s.handleRenew)
	return mux
}

func (s *Server) handleUpload(w http.ResponseWriter, r *http.Request) {
	if !s.authOK(r) {
		httpError(w, http.StatusUnauthorized, "invalid or missing upload token")
		return
	}

	name, err := decodeFilename(r.Header.Get("X-Filename"))
	if err != nil {
		httpError(w, http.StatusBadRequest, "%s", err.Error())
		return
	}

	if r.ContentLength > s.cfg.MaxSize {
		httpError(w, http.StatusRequestEntityTooLarge, "file exceeds size limit")
		return
	}
	r.Body = http.MaxBytesReader(w, r.Body, s.cfg.MaxSize)

	id, err := s.newID()
	if err != nil {
		httpError(w, http.StatusInternalServerError, "generate id")
		return
	}

	finalPath := filepath.Join(s.cfg.DataDir, "files", id)
	tmpPath := filepath.Join(s.cfg.DataDir, "tmp", id+".part")
	size, err := writeStream(tmpPath, finalPath, r.Body)
	if err != nil {
		httpError(w, mapBodyErr(err), "save file: %v", err)
		return
	}

	now := time.Now()
	secret, err := randomSecret()
	if err != nil {
		httpError(w, http.StatusInternalServerError, "generate secret")
		return
	}
	link := Link{
		ID:         id,
		SecretHash: hashSecret(secret),
		Filename:   name,
		MIME:       mimeFor(name),
		Size:       size,
		CreatedAt:  now,
		ExpiresAt:  now.Add(s.cfg.TTL),
	}

	if err := s.store.Insert(link); err != nil {
		os.Remove(finalPath)
		httpError(w, http.StatusInternalServerError, "save link")
		return
	}

	log.Printf("uploaded %s (%d bytes, %s) as %s, expires %s",
		name, size, id, link.MIME, link.ExpiresAt.Format(time.RFC3339))
	writeJSON(w, http.StatusOK, map[string]any{
		"id":        link.ID,
		"url":       s.publicURL(r, "/f/"+link.ID),
		"filename":  link.Filename,
		"mime":      link.MIME,
		"size":      link.Size,
		"expiresAt": link.ExpiresAt.UTC().Format(time.RFC3339),
		"secret":    secret,
	})
}

func (s *Server) handleDownload(w http.ResponseWriter, r *http.Request) {
	link, err := s.store.Get(r.PathValue("id"))
	if err != nil {
		if errors.Is(err, ErrNotFound) {
			httpError(w, http.StatusNotFound, "link not found")
			return
		}
		httpError(w, http.StatusInternalServerError, "lookup link")
		return
	}
	if !time.Now().Before(link.ExpiresAt) {
		httpError(w, http.StatusGone, "link expired")
		return
	}

	f, err := os.Open(filepath.Join(s.cfg.DataDir, "files", link.ID))
	if err != nil {
		if os.IsNotExist(err) {
			httpError(w, http.StatusNotFound, "file missing")
			return
		}
		httpError(w, http.StatusInternalServerError, "open file")
		return
	}
	defer f.Close()

	w.Header().Set("Content-Type", link.MIME)
	w.Header().Set("X-Content-Type-Options", "nosniff")
	w.Header().Set("Cache-Control", "no-store")
	// mime.FormatMediaType emits RFC 5987 filename*=utf-8''... for non-ASCII names.
	if cd := mime.FormatMediaType("inline", map[string]string{"filename": link.Filename}); cd != "" {
		w.Header().Set("Content-Disposition", cd)
	}
	http.ServeContent(w, r, link.Filename, link.CreatedAt, f)
}

func (s *Server) handleRenew(w http.ResponseWriter, r *http.Request) {
	secret := r.Header.Get("X-Renewal-Secret")
	if secret == "" {
		httpError(w, http.StatusBadRequest, "missing X-Renewal-Secret")
		return
	}

	link, err := s.store.Get(r.PathValue("id"))
	if err != nil {
		if errors.Is(err, ErrNotFound) {
			httpError(w, http.StatusNotFound, "link not found")
			return
		}
		httpError(w, http.StatusInternalServerError, "lookup link")
		return
	}
	if !time.Now().Before(link.ExpiresAt) {
		httpError(w, http.StatusGone, "link expired, cannot renew")
		return
	}
	if subtle.ConstantTimeCompare([]byte(hashSecret(secret)), []byte(link.SecretHash)) != 1 {
		httpError(w, http.StatusForbidden, "invalid renewal secret")
		return
	}

	newExpiry := time.Now().Add(s.cfg.TTL)
	ok, err := s.store.Renew(link.ID, newExpiry)
	if err != nil {
		httpError(w, http.StatusInternalServerError, "renew")
		return
	}
	if !ok {
		httpError(w, http.StatusGone, "link expired, cannot renew")
		return
	}
	log.Printf("renewed %s until %s", link.ID, newExpiry.Format(time.RFC3339))
	writeJSON(w, http.StatusOK, map[string]any{
		"id":        link.ID,
		"url":       s.publicURL(r, "/f/"+link.ID),
		"expiresAt": newExpiry.UTC().Format(time.RFC3339),
	})
}

func (s *Server) authOK(r *http.Request) bool {
	const prefix = "Bearer "
	auth := r.Header.Get("Authorization")
	if len(auth) <= len(prefix) || !strings.EqualFold(auth[:len(prefix)], prefix) {
		return false
	}
	return subtle.ConstantTimeCompare([]byte(auth[len(prefix):]), []byte(s.cfg.Token)) == 1
}

// publicURL prefers the configured BASE_URL; otherwise it rebuilds the
// absolute URL from the request, honouring the proto that cloudflared forwards.
func (s *Server) publicURL(r *http.Request, path string) string {
	if s.cfg.BaseURL != "" {
		return s.cfg.BaseURL + path
	}
	scheme := "http"
	if r.TLS != nil || r.Header.Get("X-Forwarded-Proto") == "https" {
		scheme = "https"
	}
	return scheme + "://" + r.Host + path
}

func (s *Server) newID() (string, error) {
	for range 5 {
		id, err := randomID(idLen)
		if err != nil {
			return "", err
		}
		ok, err := s.store.HasID(id)
		if err != nil {
			return "", err
		}
		if !ok {
			return id, nil
		}
	}
	return "", errors.New("could not allocate unique id")
}

// writeStream copies body to tmpPath, then renames into place atomically.
func writeStream(tmpPath, finalPath string, body io.Reader) (int64, error) {
	f, err := os.Create(tmpPath)
	if err != nil {
		return 0, err
	}
	size, err := io.Copy(f, body)
	if err != nil {
		f.Close()
		os.Remove(tmpPath)
		return 0, err
	}
	if err := f.Close(); err != nil {
		os.Remove(tmpPath)
		return 0, err
	}
	if err := os.Rename(tmpPath, finalPath); err != nil {
		os.Remove(tmpPath)
		return 0, err
	}
	return size, nil
}

func mapBodyErr(err error) int {
	var maxErr *http.MaxBytesError
	if errors.As(err, &maxErr) {
		return http.StatusRequestEntityTooLarge
	}
	return http.StatusInternalServerError
}

// decodeFilename unwraps the percent-encoded X-Filename header and strips any
// path components the client may have attached.
func decodeFilename(raw string) (string, error) {
	if strings.TrimSpace(raw) == "" {
		return "", errors.New("missing X-Filename header")
	}
	name, err := url.PathUnescape(raw)
	if err != nil {
		name = raw
	}
	name = strings.TrimSpace(filepath.ToSlash(name))
	name = path.Base(name)
	if name == "" || name == "." || name == ".." || name == "/" {
		return "", errors.New("invalid filename")
	}
	if len([]rune(name)) > maxNameRunes {
		runes := []rune(name)
		name = string(runes[len(runes)-maxNameRunes:])
	}
	return name, nil
}

const idAlphabet = "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz"

func randomID(n int) (string, error) {
	out := make([]byte, n)
	buf := make([]byte, n)
	max := byte(256 - 256%len(idAlphabet))
	for i := 0; i < n; {
		if _, err := rand.Read(buf); err != nil {
			return "", err
		}
		for _, b := range buf {
			if b >= max {
				continue // rejection sampling: avoid alphabet bias
			}
			out[i] = idAlphabet[int(b)%len(idAlphabet)]
			i++
			if i == n {
				break
			}
		}
	}
	return string(out), nil
}

func randomSecret() (string, error) {
	buf := make([]byte, secretBytes)
	if _, err := rand.Read(buf); err != nil {
		return "", err
	}
	return hex.EncodeToString(buf), nil
}

func hashSecret(secret string) string {
	sum := sha256.Sum256([]byte(secret))
	return hex.EncodeToString(sum[:])
}

func writeJSON(w http.ResponseWriter, status int, v any) {
	w.Header().Set("Content-Type", "application/json; charset=utf-8")
	w.WriteHeader(status)
	json.NewEncoder(w).Encode(v)
}

func httpError(w http.ResponseWriter, status int, format string, args ...any) {
	http.Error(w, fmt.Sprintf(format, args...), status)
}
