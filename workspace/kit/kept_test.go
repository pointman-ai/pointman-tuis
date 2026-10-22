package kit

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

// keptFruit is fruit with its data kept between runs.
type keptFruit struct {
	fruit
	Kept
}

func TestKeptDataOpensAtOnceAndRefreshes(t *testing.T) {
	file := filepath.Join(t.TempDir(), "fruit.json")
	first := NewApp[[]string](keptFruit{})
	first.keep = file
	first.Took([]string{"apple", "pear"}, nil)
	if _, err := os.Stat(file); err != nil {
		t.Fatal("a load's data is kept")
	}

	next := NewApp[[]string](keptFruit{})
	next.keep = file
	next.restore()
	d, got := next.Data()
	if !got || strings.Join(d, ",") != "apple,pear" {
		t.Fatalf("the next run opens with it: %v %v", got, d)
	}
	lines := Snapshot(next, 80, 24)
	if !strings.HasSuffix(lines[0], "updated just now · refreshing") || !has(lines, "pear") {
		t.Fatalf("it says it's refreshing, over the kept data:\n%s", strings.Join(lines, "\n"))
	}
	next.Took([]string{"fig"}, nil)
	if lines := Snapshot(next, 80, 24); !strings.HasSuffix(lines[0], "updated just now") || has(lines, "pear") {
		t.Fatalf("the fresh load replaces it:\n%s", strings.Join(lines, "\n"))
	}
}

func TestKeptDataFromAnotherBuildIsLeftAlone(t *testing.T) {
	file := filepath.Join(t.TempDir(), "fruit.json")
	if err := os.WriteFile(file, []byte(`{"build":"an older one","saved":"2026-10-22T00:00:00Z","data":["old"]}`), 0o600); err != nil {
		t.Fatal(err)
	}
	app := NewApp[[]string](keptFruit{})
	app.keep = file
	app.restore()
	if _, got := app.Data(); got {
		t.Fatal("another build's data may have another shape")
	}
	if lines := Snapshot(app, 80, 24); !strings.HasSuffix(lines[0], "loading…") {
		t.Fatalf("so it loads as if there were none: %q", lines[0])
	}
}

func TestOnlyKeptTUIsKeepTheirData(t *testing.T) {
	plain := NewApp[[]string](fruit{})
	plain.keepIn(fruit{})
	kept := NewApp[[]string](keptFruit{})
	kept.keepIn(keptFruit{})
	if plain.keep != "" || !strings.HasSuffix(kept.keep, filepath.Join("pointman-tuis", filepath.Base(os.Args[0])+".json")) {
		t.Fatalf("%q %q", plain.keep, kept.keep)
	}
	old := NewApp[[]string](keptFruit{})
	old.keep = filepath.Join(t.TempDir(), "f.json")
	old.Took([]string{"apple"}, nil)
	old.updated = time.Now().Add(-5 * time.Minute)
	old.save()
	again := NewApp[[]string](keptFruit{})
	again.keep = old.keep
	again.restore()
	if lines := Snapshot(again, 80, 24); !strings.HasSuffix(lines[0], "updated 5m ago · refreshing") {
		t.Fatalf("it says how old the kept data is: %q", lines[0])
	}
}
