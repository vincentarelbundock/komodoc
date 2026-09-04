package main

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestSeedRemoteUsesCuratedTitlesAndAnnotations(t *testing.T) {
	http, instance := newTestServer(t)
	instance.publishers = parsePolicy("anyone")

	file := filepath.Join(t.TempDir(), "example.html")
	if err := os.WriteFile(file, []byte("<h1>Filename title</h1><p>The phrase to annotate.</p>"), 0o644); err != nil {
		t.Fatal(err)
	}
	documents := []seedDocument{{
		File:  file,
		Title: "Curated title",
		Annotations: []seedAnnotation{{
			Motivation: "commenting",
			Exact:      "The phrase to annotate.",
			Body:       "Curated comment",
			Creator:    "Reviewer",
			Resolved:   true,
			Replies:    []string{"Curated reply"},
		}},
	}}

	seedRemote(http.URL, documents)
	seedRemote(http.URL, documents)

	entries := instance.store.list()
	if len(entries) != 1 || entries[0].Title != "Curated title" {
		t.Fatalf("remote seed entries are %#v", entries)
	}
	comments := instance.rooms.get(entries[0].Slug).snapshot()
	if len(comments) != 1 || comments[0].Body != "Curated comment" {
		t.Fatalf("remote seed comments are %#v", comments)
	}
	if !comments[0].Resolved || len(comments[0].Replies) != 1 || comments[0].Replies[0].Body != "Curated reply" {
		t.Fatalf("remote seed lost annotation state: %#v", comments[0])
	}
}

// An example's address is what people write down -- in the README, in a talk,
// in a bookmark -- so re-seeding must land on the same slug rather than mint a
// new random one and break every link.
func TestSeedingLocallyIsStableAcrossRuns(t *testing.T) {
	file := filepath.Join(t.TempDir(), "example.html")
	if err := os.WriteFile(file, []byte("<h1>Filename title</h1><p>The phrase to annotate.</p>"), 0o644); err != nil {
		t.Fatal(err)
	}
	documents := []seedDocument{{File: file, Title: "Curated title"}}

	slugs := make([]string, 2)
	for run := range slugs {
		dir := t.TempDir()
		seed(dir, documents)
		entries := newStore(dir).list()
		if len(entries) != 1 {
			t.Fatalf("run %d seeded %d documents, want 1", run, len(entries))
		}
		slugs[run] = entries[0].Slug
	}
	if slugs[0] != slugs[1] {
		t.Fatalf("two seeds produced %q and %q; the example moved", slugs[0], slugs[1])
	}
	if !strings.HasPrefix(slugs[0], "curated-title-") || len(slugs[0]) != len("curated-title-")+config.SuffixLength {
		t.Fatalf("seeded slug %q does not carry a suffix of the usual shape", slugs[0])
	}
}
