package main

import (
	"strings"
	"testing"

	"tuis/kit"
)

func TestGitLogParsesWithItsStats(t *testing.T) {
	out := "\x1eabc1234\x1fJames\x1f1760000000\x1fFirst one\n\n 2 files changed, 3 insertions(+)\n\x1edef5678\x1fpm-cli\x1f1760000100\x1fNo stat\n"
	got := commits(out)
	if len(got) != 2 {
		t.Fatalf("%+v", got)
	}
	want := commit{sha: "abc1234", who: "James", when: 1760000000, subject: "First one", changed: "2 files changed, 3 insertions(+)"}
	if got[0] != want || got[1].changed != "" {
		t.Fatalf("%+v", got)
	}
}

func TestItDrawsAt80By24(t *testing.T) {
	app := kit.NewApp[data](starter{})
	app.Took(data{
		commits:  commits("\x1eabc1234\x1fJames\x1f1760000000\x1fFirst one\n\n 2 files changed\n"),
		branches: []branch{{"main", 1760000000}},
	}, nil)
	lines := kit.Snapshot(app, 80, 24)
	for _, l := range lines {
		if strings.Contains(l, "abc1234") && strings.Contains(l, "ago  First one") {
			return
		}
	}
	t.Fatalf("\n%s", strings.Join(lines, "\n"))
}
