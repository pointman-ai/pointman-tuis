package kit

import (
	"fmt"
	"strings"
	"time"

	"charm.land/lipgloss/v2"
	"github.com/charmbracelet/x/ansi"
)

// The kit's styles: use them so every TUI looks alike.
var (
	Dim    = lipgloss.NewStyle().Foreground(lipgloss.BrightBlack)
	Ok     = lipgloss.NewStyle().Foreground(lipgloss.Green)
	Warn   = lipgloss.NewStyle().Foreground(lipgloss.Yellow)
	Bad    = lipgloss.NewStyle().Foreground(lipgloss.Red)
	Bold   = lipgloss.NewStyle().Bold(true)
	Picked = lipgloss.NewStyle().Background(lipgloss.BrightBlack).Bold(true)
	Tab    = lipgloss.NewStyle().Foreground(lipgloss.Black).Background(lipgloss.Cyan).Bold(true)
)

// Width is how many cells s takes on screen, styles left out.
func Width(s string) int { return ansi.StringWidth(s) }

// Plain is s with its styles taken out.
func Plain(s string) string { return ansi.Strip(s) }

// Fit is one line exactly w cells wide: cut when longer, padded with spaces when shorter. Styles are kept.
func Fit(s string, w int) string {
	if w <= 0 {
		return ""
	}
	s, _, _ = strings.Cut(s, "\n")
	s = ansi.Truncate(s, w, "")
	return s + strings.Repeat(" ", max(0, w-Width(s)))
}

// Block is s as exactly h lines of w cells: each line fitted, missing ones blank, extra ones dropped.
func Block(s string, w, h int) string {
	lines := strings.Split(s, "\n")
	out := make([]string, h)
	for i := range out {
		if i < len(lines) {
			out[i] = Fit(lines[i], w)
		} else {
			out[i] = strings.Repeat(" ", max(0, w))
		}
	}
	return strings.Join(out, "\n")
}

// Wrap is s wrapped to lines of at most w cells, at spaces where it can.
func Wrap(s string, w int) string {
	return ansi.Wrap(s, max(1, w), "")
}

// Boxed is body in a box w×h with a dim border, titled when title isn't "". The body is cut to fit inside.
func Boxed(title, body string, w, h int) string {
	if w < 2 || h < 2 {
		return Block(body, max(0, w), max(0, h))
	}
	top := ""
	if title != "" {
		top = " " + title + " "
	}
	top = ansi.Truncate(top, w-2, "")
	lines := []string{Dim.Render("┌" + top + strings.Repeat("─", w-2-Width(top)) + "┐")}
	inner := strings.Split(Block(body, w-2, h-2), "\n")
	for _, l := range inner[:h-2] {
		lines = append(lines, Dim.Render("│")+l+Dim.Render("│"))
	}
	lines = append(lines, Dim.Render("└"+strings.Repeat("─", w-2)+"┘"))
	return strings.Join(lines, "\n")
}

// List is items in a box w×h with the person's row picked, scrolled to keep it in view.
func List(title string, items []string, at At, w, h int) string {
	inner, room := max(0, w-2), max(1, h-2)
	first := max(0, at.Row-room+1)
	var lines []string
	for i := first; i < len(items) && i < first+room; i++ {
		l := Fit(items[i], inner)
		if i == at.Row {
			l = Picked.Render(Plain(l))
		}
		lines = append(lines, l)
	}
	return Boxed(title, strings.Join(lines, "\n"), w, h)
}

// ListAndDetail lays a list beside the open row's detail, or the list alone across w×h when nothing is open. Below
// 100 cells wide the detail goes under the list.
func ListAndDetail(at At, w, h int, list, detail func(w, h int) string) string {
	if !at.Open {
		return list(w, h)
	}
	if w < 100 {
		lh := h * 40 / 100
		return list(w, lh) + "\n" + detail(w, h-lh)
	}
	lw := w * 45 / 100
	return lipgloss.JoinHorizontal(lipgloss.Top, Block(list(lw, h), lw, h), Block(detail(w-lw, h), w-lw, h))
}

// Over puts top over base, its top-left corner at column x, line y.
func Over(base, top string, x, y int) string {
	lines := strings.Split(base, "\n")
	for i, t := range strings.Split(top, "\n") {
		if y+i < 0 || y+i >= len(lines) {
			continue
		}
		b := lines[y+i]
		lines[y+i] = ansi.Cut(b, 0, x) + t + ansi.Cut(b, x+Width(t), Width(b))
	}
	return strings.Join(lines, "\n")
}

// Ago is how long ago t was, as people say it: "just now", "12s ago", "4m ago", "2h ago", "3d ago".
func Ago(t time.Time) string { return AgoSecs(int64(time.Since(t).Seconds())) }

// AgoSecs is [Ago] for a number of seconds.
func AgoSecs(s int64) string {
	switch {
	case s < 5:
		return "just now"
	case s < 60:
		return fmt.Sprintf("%ds ago", s)
	case s < 3600:
		return fmt.Sprintf("%dm ago", s/60)
	case s < 86_400:
		return fmt.Sprintf("%dh ago", s/3600)
	default:
		return fmt.Sprintf("%dd ago", s/86_400)
	}
}

// Now is seconds since 1970.
func Now() int64 { return time.Now().Unix() }
