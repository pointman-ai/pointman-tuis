package kit

import (
	"errors"
	"strings"
	"testing"
	"time"

	tea "charm.land/bubbletea/v2"
)

// fruit has two screens: three fruit, and nothing.
type fruit struct{ Base }

func (fruit) Title() string           { return "Fruit" }
func (fruit) Screens() []string       { return []string{"Bowl", "Empty"} }
func (fruit) Load() ([]string, error) { return []string{"apple", "pear", "fig"}, nil }
func (fruit) Keys() [][2]string       { return [][2]string{{"e", "Eat it"}} }
func (fruit) Rows(d []string, screen int) int {
	if screen == 0 {
		return len(d)
	}
	return 0
}
func (fruit) Draw(d []string, at At, w, h int) string {
	return ListAndDetail(at, w, h,
		func(w, h int) string { return List("Bowl", d, at, w, h) },
		func(w, h int) string { return Boxed("", "all about "+d[at.Row], w, h) })
}

func bowl() *App[[]string] {
	app := NewApp[[]string](fruit{})
	app.Took(fruit{}.Load())
	return app
}

func has(lines []string, what string) bool {
	for _, l := range lines {
		if strings.Contains(l, what) {
			return true
		}
	}
	return false
}

func TestTheSharedKeys(t *testing.T) {
	app := bowl()
	if app.Key("j") != Stay {
		t.Fatal("j stays")
	}
	app.Key("down")
	app.Key("down")
	if app.At().Row != 2 {
		t.Fatalf("stops at the last row: %+v", app.At())
	}
	app.Key("g")
	if app.At().Row != 0 {
		t.Fatal("g goes to the first")
	}
	app.Key("G")
	if app.At().Row != 2 {
		t.Fatal("G goes to the last")
	}
	app.Key("enter")
	if !app.At().Open {
		t.Fatal("enter opens")
	}
	app.Key("esc")
	if app.At().Open {
		t.Fatal("esc closes")
	}
	app.Key("tab")
	if app.At() != (At{Screen: 1}) {
		t.Fatalf("tab: %+v", app.At())
	}
	app.Key("enter")
	if app.At().Open {
		t.Fatal("nothing to open on a screen without a list")
	}
	app.Key("shift+tab")
	app.Key("shift+tab")
	if app.At().Screen != 1 {
		t.Fatal("round the ends")
	}
	if app.Key("r") != Refresh || app.Key("q") != Quit || app.Key("ctrl+c") != Quit {
		t.Fatal("r refreshes, q and ctrl+c quit")
	}
}

func TestTheFrameAt80By24(t *testing.T) {
	app := bowl()
	lines := Snapshot(app, 80, 24)
	if len(lines) != 24 {
		t.Fatalf("%d lines", len(lines))
	}
	if !strings.HasPrefix(lines[0], " Fruit") || !strings.HasSuffix(lines[0], "updated just now") {
		t.Fatalf("top bar: %q", lines[0])
	}
	if !strings.Contains(lines[1], "Bowl") || !strings.Contains(lines[1], "Empty") {
		t.Fatalf("tabs: %q", lines[1])
	}
	if !has(lines, "apple") || !has(lines, "fig") {
		t.Fatalf("the list:\n%s", strings.Join(lines, "\n"))
	}
	if !strings.Contains(lines[23], "tab screens") || !strings.Contains(lines[23], "q quit") {
		t.Fatalf("keys: %q", lines[23])
	}
	app.Key("down")
	app.Key("enter")
	if !has(Snapshot(app, 80, 24), "all about pear") {
		t.Fatal("the open row's detail, below the list at 80")
	}
	if !has(Snapshot(app, 120, 24), "all about pear") {
		t.Fatal("the open row's detail, beside the list at 120")
	}
	app.Key("?")
	lines = Snapshot(app, 80, 24)
	if !has(lines, "Next or previous screen") || !has(lines, "Eat it") {
		t.Fatalf("help, with the TUI's own keys:\n%s", strings.Join(lines, "\n"))
	}
	app.Key("esc")
	if has(Snapshot(app, 80, 24), "Eat it") {
		t.Fatal("esc closes help first")
	}
	narrow := Snapshot(app, 50, 12)
	if narrow[11] != " ↑↓ move  ⏎ open  r refresh  ? keys  q quit" {
		t.Fatalf("the keys that fit, never cut: %q", narrow[11])
	}
	if !app.At().Open {
		t.Fatal("and the row stays open")
	}
	for _, l := range Snapshot(app, 50, 12) {
		if Width(l) > 50 {
			t.Fatalf("a line wider than the screen: %q", l)
		}
	}
}

func TestAFailedLoadKeepsTheDataAndSaysWhy(t *testing.T) {
	app := bowl()
	app.Took(nil, errors.New("core didn't answer\nmore"))
	lines := Snapshot(app, 80, 24)
	if !strings.HasSuffix(lines[0], "core didn't answer") {
		t.Fatalf("top bar: %q", lines[0])
	}
	if !has(lines, "apple") {
		t.Fatal("the data from before stays")
	}
	fresh := NewApp[[]string](fruit{})
	if !has(Snapshot(fresh, 80, 24), "Loading…") {
		t.Fatal("Loading… before the first load")
	}
	fresh.Took(nil, errors.New("no git here"))
	n := 0
	for _, l := range Snapshot(fresh, 80, 24) {
		if strings.Contains(l, "no git here") {
			n++
		}
	}
	if n < 2 {
		t.Fatal("with no data, the error is in the top bar and the body")
	}
	fresh.Key("r")
	if lines := Snapshot(fresh, 80, 24); !strings.HasSuffix(lines[0], "loading…") || has(lines, "no git here") {
		t.Fatalf("r says it's loading again:\n%s", strings.Join(lines, "\n"))
	}
}

func TestASnapshotFromTheCommandLine(t *testing.T) {
	args := func(s string) []string { return strings.Split(s, " ") }
	lines, err := snapshotFromArgs[[]string](fruit{}, args("--snapshot --size 100x10 --row 2 --open"))
	if err != nil || len(lines) != 10 || !has(lines, "all about fig") {
		t.Fatalf("%v\n%s", err, strings.Join(lines, "\n"))
	}
	lines, err = snapshotFromArgs[[]string](fruit{}, args("--snapshot --screen 9"))
	if err != nil || len(lines) != 24 || !has(lines, "Empty") {
		t.Fatalf("80x24 by default, the last screen at most: %v", err)
	}
	if _, err := snapshotFromArgs[[]string](fruit{}, args("--snapshot --size big")); err == nil {
		t.Fatal("--size big is refused")
	}
}

func TestCommandsAndHowLongAgo(t *testing.T) {
	lines, err := RunLines("sh", "-c", "echo a; echo; echo b")
	if err != nil || strings.Join(lines, ",") != "a,b" {
		t.Fatalf("%v %q", err, lines)
	}
	v, err := RunJSON[map[string]int]("sh", "-c", `echo '{"n": 1}'`)
	if err != nil || v["n"] != 1 {
		t.Fatalf("%v %v", err, v)
	}
	_, err = RunOut(5*time.Second, "sh", "-c", "echo nope >&2; exit 3")
	if err == nil || err.Error() != "sh: nope" {
		t.Fatalf("%v", err)
	}
	if _, err := RunOut(200*time.Millisecond, "sleep", "5"); err == nil || !strings.Contains(err.Error(), "took over") {
		t.Fatalf("%v", err)
	}
	// more than a pipe holds (64 KB)
	big, err := RunOut(5*time.Second, "sh", "-c", "yes 0123456789abcdef | head -n 20000")
	if err != nil || len(big) != 20000*17 {
		t.Fatalf("%v %d", err, len(big))
	}
	// a background child holding the output open: the limit still holds
	started := time.Now()
	_, err = RunOut(500*time.Millisecond, "sh", "-c", "sleep 5 & echo started")
	if err == nil || !strings.Contains(err.Error(), "left its output open") {
		t.Fatalf("%v", err)
	}
	if time.Since(started) > 3*time.Second {
		t.Fatal("waited past the limit")
	}
	got := strings.Join([]string{AgoSecs(2), AgoSecs(42), AgoSecs(300), AgoSecs(7200), AgoSecs(200_000)}, " ")
	if got != "just now 42s ago 5m ago 2h ago 2d ago" {
		t.Fatal(got)
	}
}

// counted counts its loads.
type counted struct {
	fruit
	n *int
}

func (c counted) Load() ([]string, error) { *c.n++; return c.fruit.Load() }

func TestItLoadsAgainOnTicksAndR(t *testing.T) {
	n := 0
	m := &model[[]string]{app: NewApp[[]string](counted{n: &n})}
	run := func(c tea.Cmd) tea.Msg {
		if c == nil {
			return nil
		}
		return c()
	}
	got := run(m.Init())
	if n != 1 {
		t.Fatalf("Init loads once: %d", n)
	}
	m.Update(got)
	if _, has := m.app.Data(); !has || m.busy {
		t.Fatal("the load's data is in, and nothing is loading")
	}
	stale := tick{m.gen}
	_, c := m.Update(tea.KeyPressMsg{Code: 'r', Text: "r"})
	if c == nil || !m.busy {
		t.Fatal("r loads now")
	}
	if _, c := m.Update(stale); c != nil {
		t.Fatal("a tick while a load runs does nothing")
	}
	m.Update(run(c))
	if _, c := m.Update(stale); c != nil {
		t.Fatal("a tick from before r is dropped")
	}
	if _, c := m.Update(tick{m.gen}); c == nil {
		t.Fatal("the latest tick loads again")
	}
	if n != 2 {
		t.Fatalf("loads: %d", n)
	}
	m.Update(tea.WindowSizeMsg{Width: 80, Height: 24})
	if v := m.View(); !v.AltScreen || !strings.Contains(Plain(v.Content), "apple") {
		t.Fatal("the view is the frame, on the alternate screen")
	}
	if _, c := m.Update(tea.KeyPressMsg{Code: 'q', Text: "q"}); c == nil {
		t.Fatal("q quits")
	}
}
