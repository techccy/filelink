package main

import (
	"mime"
	"path/filepath"
	"strings"
)

// textExts are served as text/plain (charset=utf-8) so browsers render them
// inline and AI fetchers read plain text directly. This deliberately includes
// .html and .svg: text can never execute on our origin.
var textExts = map[string]bool{
	".txt": true, ".md": true, ".markdown": true, ".mdown": true, ".csv": true,
	".tsv": true, ".json": true, ".jsonl": true, ".ndjson": true, ".log": true,
	".xml": true, ".yaml": true, ".yml": true, ".toml": true, ".ini": true,
	".cfg": true, ".conf": true, ".env": true, ".properties": true, ".plist": true,
	".sql": true, ".sh": true, ".bash": true, ".zsh": true, ".fish": true,
	".ps1": true, ".bat": true, ".cmd": true, ".py": true, ".pyi": true,
	".rb": true, ".php": true, ".pl": true, ".lua": true, ".js": true,
	".mjs": true, ".cjs": true, ".ts": true, ".tsx": true, ".jsx": true,
	".go": true, ".rs": true, ".java": true, ".kt": true, ".kts": true,
	".swift": true, ".c": true, ".h": true, ".cpp": true, ".cc": true,
	".cxx": true, ".hpp": true, ".hh": true, ".cs": true, ".scala": true,
	".groovy": true, ".gradle": true, ".r": true, ".jl": true, ".ex": true,
	".exs": true, ".erl": true, ".hrl": true, ".hs": true, ".clj": true,
	".cljs": true, ".vim": true, ".tex": true, ".rst": true, ".adoc": true,
	".org": true, ".srt": true, ".vtt": true, ".asm": true, ".sol": true,
	".proto": true, ".graphql": true, ".gql": true, ".diff": true, ".patch": true,
	".svg": true, ".html": true, ".htm": true, ".xhtml": true, ".css": true,
	".scss": true, ".less": true, ".vue": true, ".svelte": true, ".zig": true,
	".nim": true, ".dart": true, ".m": true, ".mm": true, ".tf": true,
	".bicep": true, ".ipynb": true,
}

// textBasenames covers extension-less files that are always text.
var textBasenames = map[string]bool{
	"makefile": true, "dockerfile": true, "gnumakefile": true, "rakefile": true,
	"gemfile": true, "procfile": true, "justfile": true, ".gitignore": true,
	".gitattributes": true, ".editorconfig": true, ".dockerignore": true,
	".npmrc": true, ".bashrc": true, ".zshrc": true, "license": true,
}

func mimeFor(filename string) string {
	base := strings.ToLower(filepath.Base(filename))
	ext := strings.ToLower(filepath.Ext(base))
	if textExts[ext] || textBasenames[base] {
		return "text/plain; charset=utf-8"
	}
	if ct := mime.TypeByExtension(ext); ct != "" {
		if i := strings.Index(ct, ";"); i >= 0 {
			ct = strings.TrimSpace(ct[:i])
		}
		// Belt and braces: never serve executable markup from user uploads.
		if ct == "text/html" || ct == "application/xhtml+xml" || ct == "image/svg+xml" {
			return "text/plain; charset=utf-8"
		}
		return ct
	}
	return "application/octet-stream"
}
