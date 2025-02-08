package hints

import (
	"bufio"
	"fmt"
	"io"
	"os"
	"regexp"
	"strings"
	"sync"

	"github.com/aspect-build/aspect-cli/pkg/ioutils"
	"github.com/creack/pty"
	"golang.org/x/term"
)

var DefaultStreams = ioutils.DefaultStreams

type Hints struct {
	Stdout *os.File
	Stderr *os.File

	hintMap    map[*regexp.Regexp]string
	hints      *hintSet
	hintsMutex sync.Mutex
	wg         sync.WaitGroup
	stdoutR    *os.File
	stderrR    *os.File
}

func New() *Hints {
	return &Hints{
		hintMap: map[*regexp.Regexp]string{},
		hints:   &hintSet{nodes: make(map[hintNode]struct{})},
	}
}

func (h *Hints) Configure(data interface{}) error {
	config, err := umarshalHintsConfig(data)
	if err != nil {
		return err
	}

	for _, entry := range config {
		regex, err := regexp.Compile(entry.pattern)
		if err != nil {
			return err
		}
		h.hintMap[regex] = entry.hint
	}

	return nil
}

func (h *Hints) Attach() error {
	if len(h.hintMap) == 0 {
		return nil
	}

	// Create stream pipes using pty if Stdout/Stderr is a terminal other using a standard pipe
	var err error
	stdoutTerm := term.IsTerminal(int(os.Stdout.Fd()))
	stderrTerm := term.IsTerminal(int(os.Stderr.Fd()))
	if stdoutTerm {
		h.stdoutR, h.Stdout, err = pty.Open()
	} else {
		h.stdoutR, h.Stdout, err = os.Pipe()
	}
	if err != nil {
		return err
	}
	if stderrTerm {
		h.stderrR, h.Stderr, err = pty.Open()
	} else {
		h.stderrR, h.Stderr, err = os.Pipe()
	}
	if err != nil {
		return err
	}

	DefaultStreams = ioutils.Streams{
		Stdin:  ioutils.DefaultStreams.Stdin,
		Stdout: h.Stdout,
		Stderr: h.Stderr,
	}

	// Create goroutines to forward streams and create hints on regex matches
	h.wg.Add(2)

	go func() {
		defer h.wg.Done()
		reader := bufio.NewReader(h.stdoutR)
		buffer := make([]byte, 4096)
		var lineBuffer strings.Builder
		for {
			n, err := reader.Read(buffer)
			if n > 0 {
				data := buffer[:n]
				fmt.Fprint(ioutils.DefaultStreams.Stdout, string(data))
				for _, b := range data {
					if b == '\n' {
						h.ProcessLine(strings.TrimSpace(lineBuffer.String()))
						lineBuffer.Reset()
					} else {
						lineBuffer.WriteByte(b)
					}
				}
			}
			if err != nil {
				if lineBuffer.Len() > 0 {
					h.ProcessLine(strings.TrimSpace(lineBuffer.String()))
				}
				if err != io.EOF {
					fmt.Fprintf(ioutils.DefaultStreams.Stderr, "Error reading from stdout: %v\n", err)
				}
				break
			}
		}
	}()

	go func() {
		defer h.wg.Done()
		reader := bufio.NewReader(h.stderrR)
		buffer := make([]byte, 4096)
		var lineBuffer strings.Builder
		for {
			n, err := reader.Read(buffer)
			if n > 0 {
				data := buffer[:n]
				fmt.Fprint(ioutils.DefaultStreams.Stderr, string(data))
				for _, b := range data {
					if b == '\n' {
						h.ProcessLine(strings.TrimSpace(lineBuffer.String()))
						lineBuffer.Reset()
					} else {
						lineBuffer.WriteByte(b)
					}
				}
			}
			if err != nil {
				if lineBuffer.Len() > 0 {
					h.ProcessLine(strings.TrimSpace(lineBuffer.String()))
				}
				if err != io.EOF {
					fmt.Fprintf(ioutils.DefaultStreams.Stderr, "Error reading from stderr: %v\n", err)
				}
				break
			}
		}
	}()

	return nil
}

func (h *Hints) Detach() {
	DefaultStreams = ioutils.DefaultStreams

	if h.Stdout != nil {
		h.Stdout.Close()
		h.Stdout = nil
	}
	if h.Stderr != nil {
		h.Stderr.Close()
		h.Stderr = nil
	}

	h.wg.Wait()
}

func (h *Hints) ProcessLine(line string) {
	sanitizedLine := stripColorCodes(line)
	for regex, hint := range h.hintMap {
		matches := regex.FindStringSubmatch(sanitizedLine)
		if len(matches) > 0 {
			// apply regex capture group replacements to given hint
			for i, match := range matches {
				if i == 0 {
					// skipping the first match because it will always contain the entire result
					// of the regex match. We are only after specific capture groups
					continue
				}
				hint = strings.ReplaceAll(hint, fmt.Sprint("$", i), match)
			}
			h.hintsMutex.Lock()
			h.hints.insert(hint)
			h.hintsMutex.Unlock()
		}
	}
}

func (h *Hints) PrintHints(f *os.File) {
	if h.hints.size == 0 {
		return
	}
	printBreak(f)
	printMiddle(f, "[Aspect CLI]")
	printMiddle(f, "")
	for node := h.hints.head; node != nil; node = node.next {
		lines := strings.Split(node.hint, "\n")
		for i, line := range lines {
			if i == 0 {
				printMiddle(f, "- "+line)
			} else {
				printMiddle(f, "  "+line)
			}
		}
	}
	printBreak(f)
}

func stripColorCodes(s string) string {
	var result strings.Builder
	i := 0
	for i < len(s) {
		if s[i] == '\x1b' && i+1 < len(s) && s[i+1] == '[' { // Start of ANSI escape sequence
			// Skip until we see 'm' which ends the ANSI code
			for i++; i < len(s) && s[i] != 'm'; i++ {
			}
			if i < len(s) {
				i++ // Skip the 'm'
			}
		} else {
			result.WriteByte(s[i])
			i++
		}
	}
	return result.String()
}

func printBreak(f *os.File) {
	// using buffer so that we can easily determine the current length of the string and
	// ensure we create a proper square with a straight border
	var b strings.Builder

	fmt.Fprint(&b, " ")

	for i := 0; i < 90; i++ {
		fmt.Fprint(&b, "-")
	}

	fmt.Fprint(&b, " ")

	fmt.Fprintln(f, b.String())
}

func printMiddle(f *os.File, str string) {
	// using buffer so that we can easily determine the current length of the string and
	// ensure we create a proper square with a straight border
	var b strings.Builder

	fmt.Fprint(&b, "| ")
	fmt.Fprint(&b, str)

	for b.Len() < 91 {
		fmt.Fprint(&b, " ")
	}

	fmt.Fprint(&b, "|")
	fmt.Fprintln(f, b.String())
}

type hintConfig struct {
	pattern string
	hint    string
}

func umarshalHintsConfig(data interface{}) ([]hintConfig, error) {
	result := []hintConfig{}

	if data == nil {
		return result, nil
	}

	entries, ok := data.([]interface{})

	if !ok {
		return nil, fmt.Errorf("expected hints config to be a list")
	}

	for i, h := range entries {
		m, ok := h.(map[string]interface{})
		if !ok {
			return nil, fmt.Errorf("expected hint entry %v to be a map", i)
		}

		pattern, ok := m["pattern"].(string)
		if !ok {
			return nil, fmt.Errorf("expected hint entry %v to have a 'pattern' attribute", i)
		}

		hint, ok := m["hint"].(string)
		if !ok {
			return nil, fmt.Errorf("expected hint entry '%v' to have a 'hint' attribute", i)
		}

		result = append(result, hintConfig{
			pattern: pattern,
			hint:    hint,
		})
	}

	return result, nil
}

type hintSet struct {
	head  *hintNode
	tail  *hintNode
	nodes map[hintNode]struct{}
	size  int
}

func (s *hintSet) insert(hint string) {
	node := hintNode{
		hint: hint,
	}
	if _, exists := s.nodes[node]; !exists {
		s.nodes[node] = struct{}{}
		if s.head == nil {
			s.head = &node
		} else {
			s.tail.next = &node
		}
		s.tail = &node
		s.size++
	}
}

type hintNode struct {
	next *hintNode
	hint string
}
