package kit

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"os/exec"
	"strings"
	"time"
)

// RunOut is a command's output: its stdout, or an error with its first line of stderr. It gives up after wait.
func RunOut(wait time.Duration, program string, args ...string) (string, error) {
	ctx, cancel := context.WithTimeout(context.Background(), wait)
	defer cancel()
	cmd := exec.CommandContext(ctx, program, args...)
	var out, errOut bytes.Buffer
	cmd.Stdout, cmd.Stderr = &out, &errOut
	// a child it left running in the background can hold the pipes open after it exits: stop waiting for them
	cmd.WaitDelay = min(wait, time.Second)
	err := cmd.Run()
	var exit *exec.ExitError
	switch {
	case err == nil:
		return out.String(), nil
	case errors.Is(err, exec.ErrWaitDelay):
		return "", fmt.Errorf("%s left its output open past %s", program, wait)
	case ctx.Err() != nil:
		return "", fmt.Errorf("%s took over %s", program, wait)
	case errors.As(err, &exit):
		for _, l := range strings.Split(errOut.String(), "\n") {
			if strings.TrimSpace(l) != "" {
				return "", fmt.Errorf("%s: %s", program, l)
			}
		}
		return "", fmt.Errorf("%s: failed", program)
	default:
		return "", fmt.Errorf("running %s: %w", program, err)
	}
}

// RunLines is a command's output as lines, empty ones left out. It gives up after 20 s.
func RunLines(program string, args ...string) ([]string, error) {
	out, err := RunOut(20*time.Second, program, args...)
	if err != nil {
		return nil, err
	}
	var lines []string
	for _, l := range strings.Split(out, "\n") {
		if strings.TrimSpace(l) != "" {
			lines = append(lines, l)
		}
	}
	return lines, nil
}

// RunJSON is a command's output read as JSON into a T. It gives up after 20 s.
func RunJSON[T any](program string, args ...string) (T, error) {
	var v T
	out, err := RunOut(20*time.Second, program, args...)
	if err != nil {
		return v, err
	}
	if err := json.Unmarshal([]byte(out), &v); err != nil {
		return v, fmt.Errorf("%s's output isn't JSON: %w", program, err)
	}
	return v, nil
}
