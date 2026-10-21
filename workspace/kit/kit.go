// Package kit is what every TUI here shares (EXT-39; James, Pointman thread 17421): one look and one set of keys,
// on Bubble Tea.
//
// A TUI says what it shows ([Tui]): its screens, how to load its data, and how to draw each screen. The kit does the
// rest:
//   - the terminal, and the frame: a top bar with the title and how fresh the data is, tabs, and the keys at the
//     bottom;
//   - loading in the background, every few seconds and on r, keeping the last data when a load fails;
//   - the keys every TUI shares (README.md): tab screens, ↑↓/jk move, enter open, esc back, r refresh, ? help, q quit;
//   - --snapshot, which prints one frame as text (--size 80x24, --screen N, --row N, --open), so an agent or a test
//     can check a layout without a terminal.
//
// TUIs only read. One that changes something asks first, on screen.
package kit

import (
	"fmt"
	"os"
	"strconv"
	"strings"
	"time"

	tea "charm.land/bubbletea/v2"
	"github.com/charmbracelet/x/term"
)

// Tui is a TUI: what it shows and how. D is everything one load reads.
type Tui[D any] interface {
	// Title is what it shows, for the top bar: "Releases".
	Title() string
	// Screens are its screens, in tab order. One is fine: then there are no tabs.
	Screens() []string
	// Load reads everything fresh. It runs in the background, so it may take a few seconds; an error keeps the
	// last data on screen and says why at the top.
	Load() (D, error)
	// Every is how often to load again. Embed [Base] for 10 s.
	Every() time.Duration
	// Rows is how many rows the list on screen has: ↑↓ move among them and enter opens one. 0 for a screen
	// without a list.
	Rows(d D, screen int) int
	// Draw draws the screen the person is on (at) as w×h cells of text. Lines that are short or missing are
	// padded, and longer ones cut.
	Draw(d D, at At, w, h int) string
	// Keys are its own keys, for help: {keys, what they do}. Embed [Base] for none.
	Keys() [][2]string
}

// Base gives a [Tui] the usual Every and Keys: embed it and define your own to change them.
type Base struct{}

func (Base) Every() time.Duration { return 10 * time.Second }
func (Base) Keys() [][2]string    { return nil }

// At is where the person is: the screen, the row picked in its list, and whether that row is open (enter; esc
// closes).
type At struct {
	Screen int
	Row    int
	Open   bool
}

// Step is what a key asks for next.
type Step int

const (
	Stay Step = iota
	Refresh
	Quit
)

// App is a TUI on screen: its data, where the person is, and whether help is up.
type App[D any] struct {
	tui     Tui[D]
	data    D
	has     bool
	at      At
	help    bool
	updated time.Time // when the last load worked
	failed  string    // why the last load failed, in one line; "" when it worked
	loading bool
}

// NewApp is a TUI with nothing loaded yet.
func NewApp[D any](t Tui[D]) *App[D] {
	return &App[D]{tui: t, loading: true}
}

func (a *App[D]) At() At { return a.at }

// Data is the last load's data, and whether there is any yet.
func (a *App[D]) Data() (D, bool) { return a.data, a.has }

// Took is what a load got. A failed one keeps the data from before.
func (a *App[D]) Took(d D, err error) {
	a.loading = false
	if err != nil {
		a.failed, _, _ = strings.Cut(err.Error(), "\n")
	} else {
		a.data, a.has, a.failed, a.updated = d, true, "", time.Now()
	}
	a.rowInBounds()
}

func (a *App[D]) rows() int {
	if !a.has {
		return 0
	}
	return a.tui.Rows(a.data, a.at.Screen)
}

func (a *App[D]) rowInBounds() {
	a.at.Row = max(0, min(a.at.Row, a.rows()-1))
	if a.rows() == 0 {
		a.at.Open = false
	}
}

func (a *App[D]) goBy(by int) {
	n := max(1, len(a.tui.Screens()))
	a.at = At{Screen: ((a.at.Screen+by)%n + n) % n}
}

// Key is a key, named as Bubble Tea names it ("j", "down", "shift+tab", "ctrl+c"): what it asks for next.
func (a *App[D]) Key(k string) Step {
	last := max(0, a.rows()-1)
	switch k {
	case "ctrl+c", "q":
		return Quit
	case "?":
		a.help = !a.help
	case "esc":
		if a.help {
			a.help = false
		} else {
			a.at.Open = false
		}
	case "r":
		a.loading, a.failed = true, ""
		return Refresh
	case "tab":
		a.goBy(1)
	case "shift+tab":
		a.goBy(-1)
	case "up", "k":
		a.at.Row = max(0, a.at.Row-1)
	case "down", "j":
		a.at.Row = min(last, a.at.Row+1)
	case "pgup":
		a.at.Row = max(0, a.at.Row-10)
	case "pgdown":
		a.at.Row = min(last, a.at.Row+10)
	case "home", "g":
		a.at.Row = 0
	case "end", "G":
		a.at.Row = last
	case "enter":
		a.at.Open = a.rows() > 0
	}
	return Stay
}

// keyHints are the key hints, longest first: the frame shows the longest that fits, so a narrow panel (a phone's)
// keeps "? keys" and "q quit" whole.
var keyHints = []string{
	" tab screens  ↑↓ move  ⏎ open  esc back  r refresh  ? keys  q quit",
	" ↑↓ move  ⏎ open  esc back  r refresh  ? keys  q quit",
	" ↑↓ move  ⏎ open  r refresh  ? keys  q quit",
	" ? keys  q quit",
}

// Frame is the whole frame at w×h, styled: top bar, tabs, the TUI's screen, the keys, and help over it when it's
// up.
func (a *App[D]) Frame(w, h int) string {
	screens := a.tui.Screens()
	tabs := len(screens) > 1
	bodyH := max(1, h-2)
	lines := []string{a.topBar(w)}
	if tabs {
		bodyH = max(1, h-3)
		var b strings.Builder
		for i, s := range screens {
			if i > 0 {
				b.WriteString(Dim.Render(" "))
			}
			if i == a.at.Screen {
				b.WriteString(Tab.Render(" " + s + " "))
			} else {
				b.WriteString(Dim.Render(" " + s + " "))
			}
		}
		lines = append(lines, Fit(b.String(), w))
	}
	var body string
	switch {
	case a.has:
		body = a.tui.Draw(a.data, a.at, w, bodyH)
	case a.failed != "":
		body = Boxed("", Bad.Render(a.failed), w, bodyH)
	default:
		body = Boxed("", Dim.Render("Loading…"), w, bodyH)
	}
	body = Block(body, w, bodyH)
	if a.help {
		body = a.helpOver(body, w, bodyH)
	}
	lines = append(lines, body)
	hint := keyHints[len(keyHints)-1]
	for i, k := range keyHints {
		if (i > 0 || tabs) && len([]rune(k)) <= w {
			hint = k
			break
		}
	}
	lines = append(lines, Fit(Dim.Render(hint), w))
	return Block(strings.Join(lines, "\n"), w, h)
}

func (a *App[D]) topBar(w int) string {
	title := " " + a.tui.Title()
	var news string
	switch {
	case a.loading:
		news = Dim.Render("loading… ")
	case a.failed != "":
		news = Bad.Render(a.failed + " ")
	default:
		news = Dim.Render("updated " + Ago(a.updated) + " ")
	}
	room := max(0, w-Width(title)-2)
	if Width(news) > room {
		news = Fit(news, room)
	}
	return Fit(Bold.Render(title), w-Width(news)) + news
}

func (a *App[D]) helpOver(body string, w, h int) string {
	var rows [][2]string
	if len(a.tui.Screens()) > 1 {
		rows = append(rows, [2]string{"tab  ⇧tab", "Next or previous screen"})
	}
	rows = append(rows,
		[2]string{"↑↓  j k", "Move"},
		[2]string{"g  G", "First or last"},
		[2]string{"⏎  esc", "Open · back"},
		[2]string{"r", "Refresh now"},
		[2]string{"?", "These keys"},
		[2]string{"q", "Quit"},
	)
	rows = append(rows, a.tui.Keys()...)
	lines := make([]string, len(rows))
	for i, r := range rows {
		lines[i] = fmt.Sprintf("%-12s%s", r[0], r[1])
	}
	bw, bh := min(44, w), min(len(lines)+2, h)
	return Over(body, Boxed("Keys", strings.Join(lines, "\n"), bw, bh), (w-bw)/2, (h-bh)/2)
}

// Snapshot is one frame of app as text, a line per row with no trailing spaces: what --snapshot prints, and what
// tests read.
func Snapshot[D any](app *App[D], w, h int) []string {
	lines := strings.Split(Plain(app.Frame(w, h)), "\n")
	for i, l := range lines {
		lines[i] = strings.TrimRight(l, " ")
	}
	return lines
}

// Main runs the TUI: in a terminal, till q; with --snapshot, prints one frame as text and returns.
func Main[D any](t Tui[D]) {
	if err := main(t, os.Args[1:]); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}

func main[D any](t Tui[D], args []string) error {
	for _, a := range args {
		if a == "--snapshot" {
			lines, err := snapshotFromArgs(t, args)
			if err != nil {
				return err
			}
			fmt.Println(strings.Join(lines, "\n"))
			return nil
		}
	}
	if !term.IsTerminal(os.Stdin.Fd()) || !term.IsTerminal(os.Stdout.Fd()) {
		return fmt.Errorf("%s draws in a terminal: run it in one, or with --snapshot to print a frame", t.Title())
	}
	_, err := tea.NewProgram(&model[D]{app: NewApp(t)}).Run()
	return err
}

func snapshotFromArgs[D any](t Tui[D], args []string) ([]string, error) {
	after := func(flag string) (string, bool) {
		for i, a := range args {
			if a == flag && i+1 < len(args) {
				return args[i+1], true
			}
		}
		return "", false
	}
	w, h := 80, 24
	if s, ok := after("--size"); ok {
		ws, hs, _ := strings.Cut(s, "x")
		var err1, err2 error
		w, err1 = strconv.Atoi(ws)
		h, err2 = strconv.Atoi(hs)
		if err1 != nil || err2 != nil || w < 1 || h < 1 {
			return nil, fmt.Errorf("--size WxH, like 80x24: not %q", s)
		}
	}
	number := func(flag string) (int, error) {
		s, ok := after(flag)
		if !ok {
			return 0, nil
		}
		n, err := strconv.Atoi(s)
		if err != nil || n < 0 {
			return 0, fmt.Errorf("%s N: not %q", flag, s)
		}
		return n, nil
	}
	screen, err := number("--screen")
	if err != nil {
		return nil, err
	}
	row, err := number("--row")
	if err != nil {
		return nil, err
	}
	app := NewApp(t)
	app.Took(t.Load())
	app.at = At{Screen: min(screen, max(0, len(t.Screens())-1)), Row: row}
	app.rowInBounds()
	for _, a := range args {
		if a == "--open" {
			app.at.Open = app.rows() > 0
		}
	}
	return Snapshot(app, w, h), nil
}

// the Bubble Tea program around an App: loads in the background, every Every() and on r

type loaded[D any] struct {
	data D
	err  error
}

type tick struct{ gen int }

type model[D any] struct {
	app  *App[D]
	w, h int
	gen  int // which load the next tick belongs to: a tick from before a refresh is dropped
	busy bool
}

func (m *model[D]) load() tea.Cmd {
	m.busy = true
	t := m.app.tui
	return func() tea.Msg {
		d, err := t.Load()
		return loaded[D]{d, err}
	}
}

func (m *model[D]) Init() tea.Cmd { return m.load() }

func (m *model[D]) Update(msg tea.Msg) (tea.Model, tea.Cmd) {
	switch msg := msg.(type) {
	case tea.WindowSizeMsg:
		m.w, m.h = msg.Width, msg.Height
	case loaded[D]:
		m.busy = false
		m.app.Took(msg.data, msg.err)
		m.gen++
		gen := m.gen
		return m, tea.Tick(m.app.tui.Every(), func(time.Time) tea.Msg { return tick{gen} })
	case tick:
		if msg.gen == m.gen && !m.busy {
			return m, m.load()
		}
	case tea.KeyPressMsg:
		switch m.app.Key(msg.String()) {
		case Quit:
			return m, tea.Quit
		case Refresh:
			if !m.busy {
				return m, m.load()
			}
		}
	}
	return m, nil
}

func (m *model[D]) View() tea.View {
	v := tea.NewView("")
	if m.w > 0 && m.h > 0 {
		v.Content = m.app.Frame(m.w, m.h)
	}
	v.AltScreen = true
	v.WindowTitle = m.app.tui.Title()
	return v
}
