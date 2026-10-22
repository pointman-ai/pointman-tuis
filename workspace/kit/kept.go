package kit

import (
	"encoding/json"
	"os"
	"path/filepath"
	"strconv"
	"time"
)

// Kept, embedded in a Tui, keeps its data between runs: it opens at once with the data it last loaded, saying how old
// it is, and loads fresh data behind it. A --snapshot keeps its data too, so an agent's check warms it for the person.
// The data must survive encoding/json, so give it exported fields, and test that it does.
type Kept struct{}

func (Kept) kept() {}

type kept interface{ kept() }

// keptData is a Kept TUI's file: its data, when it loaded, and which build saved it.
type keptData[D any] struct {
	Build string    `json:"build"`
	Saved time.Time `json:"saved"`
	Data  D         `json:"data"`
}

// keepIn keeps a Kept TUI's data in the user's cache folder, one file per app, named after its program.
func (a *App[D]) keepIn(t Tui[D]) {
	if _, ok := any(t).(kept); !ok {
		return
	}
	dir, err := os.UserCacheDir()
	if err != nil {
		return
	}
	a.keep = filepath.Join(dir, "pointman-tuis", filepath.Base(os.Args[0])+".json")
}

// build is this program's build, so data saved by another build of it (whose data may have a different shape) is left
// alone: the program file's size and time.
func build() string {
	exe, err := os.Executable()
	if err != nil {
		return ""
	}
	st, err := os.Stat(exe)
	if err != nil {
		return ""
	}
	return st.ModTime().UTC().Format(time.RFC3339Nano) + " " + strconv.FormatInt(st.Size(), 10)
}

// save keeps the data the last load got. A failure only means the next run starts with nothing, as before.
func (a *App[D]) save() {
	if a.keep == "" {
		return
	}
	b, err := json.Marshal(keptData[D]{Build: build(), Saved: a.updated, Data: a.data})
	if err != nil {
		return
	}
	_ = os.MkdirAll(filepath.Dir(a.keep), 0o700)
	tmp := a.keep + ".tmp"
	if os.WriteFile(tmp, b, 0o600) == nil {
		_ = os.Rename(tmp, a.keep)
	}
}

// restore draws the data the last run kept, while the first load runs. Data from another build is left alone.
func (a *App[D]) restore() {
	if a.keep == "" {
		return
	}
	b, err := os.ReadFile(a.keep)
	if err != nil {
		return
	}
	var k keptData[D]
	if json.Unmarshal(b, &k) != nil || k.Build != build() {
		return
	}
	a.data, a.has, a.updated = k.Data, true, k.Saved
	a.rowInBounds()
}
