// The starter: copy this folder to begin a TUI (README.md, "Adding a TUI"). It shows the git repo it runs in: its
// latest commits, each opening to what it changed, and its branches. Keep its shape: load everything in Load (it
// runs in the background), keep parsing in plain functions you can test, and draw with the kit.
//
// To open at once with the data it last loaded, embed kit.Kept beside kit.Base; its data then needs exported fields,
// and a test that they survive JSON.
package main

import (
	"fmt"
	"strconv"
	"strings"
	"time"

	"tuis/kit"
)

// commit is one commit, as git log gives it.
type commit struct {
	sha, who string
	when     int64
	subject  string
	changed  string // "3 files changed, 10 insertions(+)", when git says
}

type branch struct {
	name string
	when int64
}

// data is what a load reads.
type data struct {
	commits  []commit
	branches []branch
}

// commits parses `git log --format=%x1e%h%x1f%an%x1f%at%x1f%s --shortstat`: a record per commit, its stat on a
// line after.
func commits(out string) []commit {
	var got []commit
	for _, rec := range strings.Split(out, "\x1e") {
		head, rest, _ := strings.Cut(rec, "\n")
		f := strings.Split(head, "\x1f")
		if len(f) < 4 {
			continue
		}
		when, err := strconv.ParseInt(f[2], 10, 64)
		if err != nil {
			continue
		}
		c := commit{sha: f[0], who: f[1], when: when, subject: f[3]}
		for _, l := range strings.Split(rest, "\n") {
			if l = strings.TrimSpace(l); l != "" {
				c.changed = l
				break
			}
		}
		got = append(got, c)
	}
	return got
}

type starter struct{ kit.Base }

func (starter) Title() string        { return "This repo" }
func (starter) Screens() []string    { return []string{"Commits", "Branches"} }
func (starter) Every() time.Duration { return 15 * time.Second }

func (starter) Load() (data, error) {
	log, err := kit.RunOut(20*time.Second, "git", "log", "-n", "50", "--format=%x1e%h%x1f%an%x1f%at%x1f%s", "--shortstat")
	if err != nil {
		return data{}, err
	}
	lines, err := kit.RunLines("git", "branch", "-a", "--sort=-committerdate",
		"--format=%(refname:short)\t%(committerdate:unix)")
	if err != nil {
		return data{}, err
	}
	d := data{commits: commits(log)}
	for _, l := range lines {
		name, t, _ := strings.Cut(l, "\t")
		when, _ := strconv.ParseInt(t, 10, 64)
		d.branches = append(d.branches, branch{name, when})
	}
	return d, nil
}

func (starter) Rows(d data, screen int) int {
	return []int{len(d.commits), len(d.branches)}[screen]
}

func (starter) Draw(d data, at kit.At, w, h int) string {
	now := kit.Now()
	if at.Screen == 1 {
		items := make([]string, len(d.branches))
		for i, b := range d.branches {
			items[i] = b.name + kit.Dim.Render("  "+kit.AgoSecs(now-b.when))
		}
		return kit.List("Branches, latest first", items, at, w, h)
	}
	items := make([]string, len(d.commits))
	for i, c := range d.commits {
		items[i] = kit.Warn.Render(c.sha+" ") + kit.Dim.Render(fmt.Sprintf("%8s  ", kit.AgoSecs(now-c.when))) + c.subject
	}
	return kit.ListAndDetail(at, w, h,
		func(w, h int) string { return kit.List("Commits", items, at, w, h) },
		func(w, h int) string {
			c := d.commits[at.Row]
			body := kit.Wrap(c.subject, w-2) + "\n\n" +
				kit.Dim.Render(fmt.Sprintf("%s · %s · %s", c.sha, c.who, kit.AgoSecs(now-c.when))) + "\n" + c.changed
			return kit.Boxed(c.sha, body, w, h)
		})
}

func main() { kit.Main[data](starter{}) }
