package main

import (
	"database/sql"
	"errors"
	"fmt"
	"time"

	_ "modernc.org/sqlite"
)

var ErrNotFound = errors.New("link not found")

type Link struct {
	ID         string
	SecretHash string
	Filename   string
	MIME       string
	Size       int64
	CreatedAt  time.Time
	ExpiresAt  time.Time
}

type Store struct {
	db *sql.DB
}

func OpenStore(path string) (*Store, error) {
	// Single connection serializes writes; pragmas via DSN apply to every
	// new connection the pool might create.
	dsn := fmt.Sprintf("file:%s?_pragma=busy_timeout(5000)&_pragma=journal_mode(WAL)&_pragma=synchronous(NORMAL)", path)
	db, err := sql.Open("sqlite", dsn)
	if err != nil {
		return nil, err
	}
	db.SetMaxOpenConns(1)
	schema := `
CREATE TABLE IF NOT EXISTS links (
	id          TEXT PRIMARY KEY,
	secret_hash TEXT NOT NULL,
	filename    TEXT NOT NULL,
	mime        TEXT NOT NULL,
	size        INTEGER NOT NULL,
	created_at  INTEGER NOT NULL,
	expires_at  INTEGER NOT NULL
);`
	if _, err := db.Exec(schema); err != nil {
		db.Close()
		return nil, err
	}
	return &Store{db: db}, nil
}

func (s *Store) Close() error { return s.db.Close() }

func (s *Store) Insert(l Link) error {
	_, err := s.db.Exec(
		`INSERT INTO links (id, secret_hash, filename, mime, size, created_at, expires_at)
		 VALUES (?, ?, ?, ?, ?, ?, ?)`,
		l.ID, l.SecretHash, l.Filename, l.MIME, l.Size, l.CreatedAt.Unix(), l.ExpiresAt.Unix())
	return err
}

func (s *Store) Get(id string) (Link, error) {
	row := s.db.QueryRow(
		`SELECT id, secret_hash, filename, mime, size, created_at, expires_at FROM links WHERE id = ?`, id)
	var l Link
	var createdAt, expiresAt int64
	if err := row.Scan(&l.ID, &l.SecretHash, &l.Filename, &l.MIME, &l.Size, &createdAt, &expiresAt); err != nil {
		if errors.Is(err, sql.ErrNoRows) {
			return l, ErrNotFound
		}
		return l, err
	}
	l.CreatedAt = time.Unix(createdAt, 0)
	l.ExpiresAt = time.Unix(expiresAt, 0)
	return l, nil
}

func (s *Store) HasID(id string) (bool, error) {
	var one int
	err := s.db.QueryRow(`SELECT 1 FROM links WHERE id = ?`, id).Scan(&one)
	if errors.Is(err, sql.ErrNoRows) {
		return false, nil
	}
	if err != nil {
		return false, err
	}
	return true, nil
}

// Renew pushes the expiry forward and reports whether the row was still alive.
func (s *Store) Renew(id string, expiresAt time.Time) (bool, error) {
	res, err := s.db.Exec(`UPDATE links SET expires_at = ? WHERE id = ? AND expires_at > ?`,
		expiresAt.Unix(), id, time.Now().Unix())
	if err != nil {
		return false, err
	}
	n, err := res.RowsAffected()
	if err != nil {
		return false, err
	}
	return n > 0, nil
}

// Revoke expires a link immediately (early expiry). Reports whether the row
// was still alive; the janitor removes row and file on its next pass.
func (s *Store) Revoke(id string, now time.Time) (bool, error) {
	res, err := s.db.Exec(`UPDATE links SET expires_at = ? WHERE id = ? AND expires_at > ?`,
		now.Unix(), id, now.Unix())
	if err != nil {
		return false, err
	}
	n, err := res.RowsAffected()
	if err != nil {
		return false, err
	}
	return n > 0, nil
}

// DeleteExpired removes expired rows and returns their ids so the caller can
// delete the backing files.
func (s *Store) DeleteExpired(before time.Time) ([]string, error) {
	rows, err := s.db.Query(`SELECT id FROM links WHERE expires_at <= ?`, before.Unix())
	if err != nil {
		return nil, err
	}
	var ids []string
	for rows.Next() {
		var id string
		if err := rows.Scan(&id); err != nil {
			rows.Close()
			return ids, err
		}
		ids = append(ids, id)
	}
	rows.Close()
	for _, id := range ids {
		if _, err := s.db.Exec(`DELETE FROM links WHERE id = ?`, id); err != nil {
			return ids, err
		}
	}
	return ids, nil
}
